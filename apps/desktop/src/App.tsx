import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api, describeStage, formatSeconds, toCommandError } from "./api";
import type {
  CaptionClip,
  CommandError,
  DoctorReport,
  GeneratedInfo,
  GenerationStage,
  PlatformInfo,
  ValidationReport,
  WorkspaceInfo,
} from "./types";

type Busy = "idle" | "validating" | "generating" | "exporting" | "bundling" | "doctor";

interface LogLine {
  id: number;
  text: string;
  kind: "info" | "warn" | "error";
}

function ErrorBanner({ error, onClose }: { error: CommandError; onClose: () => void }) {
  return (
    <div className="banner error" role="alert">
      <code className="code">{error.code}</code>
      <span className="message">{error.message}</span>
      <button className="link" onClick={onClose} aria-label="close">
        ×
      </button>
    </div>
  );
}

export default function App() {
  const [platform, setPlatform] = useState<PlatformInfo | null>(null);
  const [workspace, setWorkspace] = useState<WorkspaceInfo | null>(null);
  const [script, setScript] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [savedContent, setSavedContent] = useState("");
  const [validation, setValidation] = useState<ValidationReport | null>(null);
  const [generated, setGenerated] = useState<GeneratedInfo | null>(null);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [doctor, setDoctor] = useState<DoctorReport | null>(null);
  const [error, setError] = useState<CommandError | null>(null);
  const [busy, setBusy] = useState<Busy>("idle");
  const [log, setLog] = useState<LogLine[]>([]);
  const [progress, setProgress] = useState<GenerationStage | null>(null);
  const [fakeTts, setFakeTts] = useState(false);
  const [noPreview, setNoPreview] = useState(false);
  const logId = useRef(0);
  const logEnd = useRef<HTMLDivElement | null>(null);

  const dirty = content !== savedContent;

  const pushLog = useCallback((text: string, kind: LogLine["kind"] = "info") => {
    logId.current += 1;
    setLog((l) => [...l.slice(-199), { id: logId.current, text, kind }]);
  }, []);

  const fail = useCallback(
    (e: unknown) => {
      const err = toCommandError(e);
      setError(err);
      pushLog(`[${err.code}] ${err.message}`, "error");
    },
    [pushLog],
  );

  // Platform capabilities first (offline, instant), then progress events.
  useEffect(() => {
    api.platformInfo().then(setPlatform).catch(fail);
    let unlisten: (() => void) | undefined;
    api
      .onProgress((stage) => {
        setProgress(stage);
        pushLog(describeStage(stage), stage.stage === "preview_skipped" ? "warn" : "info");
      })
      .then((u) => {
        unlisten = u;
      })
      .catch(fail);
    return () => unlisten?.();
  }, [fail, pushLog]);

  useEffect(() => {
    logEnd.current?.scrollIntoView({ block: "end" });
  }, [log]);

  // Preview video: fetch bytes through IPC and hand a blob: URL to <video>.
  useEffect(() => {
    let revoked = false;
    let url: string | null = null;
    setPreviewUrl(null);
    if (workspace && generated?.preview_path) {
      api
        .readGeneratedFile(workspace.root, generated.preview_path)
        .then((bytes) => {
          if (revoked) return;
          url = URL.createObjectURL(new Blob([bytes], { type: "video/mp4" }));
          setPreviewUrl(url);
        })
        .catch(fail);
    }
    return () => {
      revoked = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [workspace, generated, fail]);

  const loadWorkspace = useCallback(
    async (root: string) => {
      const info = await api.openWorkspace(root);
      setWorkspace(info);
      setGenerated(null);
      setValidation(null);
      pushLog(`Workspace: ${info.root}`);
      const first = info.scripts[0]?.rel ?? null;
      setScript(first);
      if (first) {
        const text = await api.readScript(info.root, first);
        setContent(text);
        setSavedContent(text);
      } else {
        setContent("");
        setSavedContent("");
      }
      return info;
    },
    [pushLog],
  );

  const pickWorkspace = async () => {
    try {
      const dir = await openDialog({ directory: true, multiple: false, title: "Workspace を選択" });
      if (typeof dir === "string") await loadWorkspace(dir);
    } catch (e) {
      fail(e);
    }
  };

  const createWorkspace = async () => {
    try {
      const dir = await openDialog({
        directory: true,
        multiple: false,
        title: "新しい Workspace を作る場所（videoforge init）",
      });
      if (typeof dir !== "string") return;
      const info = await api.createWorkspace(dir, null);
      pushLog(`Initialized ${info.root}`);
      await loadWorkspace(info.root);
    } catch (e) {
      fail(e);
    }
  };

  const selectScript = async (rel: string) => {
    if (!workspace) return;
    if (dirty && !window.confirm("未保存の変更があります。破棄しますか？")) return;
    try {
      const text = await api.readScript(workspace.root, rel);
      setScript(rel);
      setContent(text);
      setSavedContent(text);
      setValidation(null);
    } catch (e) {
      fail(e);
    }
  };

  const saveScript = async () => {
    if (!workspace || !script) return;
    try {
      await api.writeScript(workspace.root, script, content);
      setSavedContent(content);
      pushLog(`Saved ${script}`);
    } catch (e) {
      fail(e);
    }
  };

  const validate = async () => {
    if (!workspace || !script) return;
    setError(null);
    setBusy("validating");
    try {
      if (dirty) await saveScript();
      const report = await api.validateScript(workspace.root, script);
      setValidation(report);
      pushLog(
        report.errors.length === 0
          ? `Validate OK: ${report.dialogues.length} dialogues, ${report.total_chars} chars`
          : `Validate FAILED: ${report.errors.length} error(s)`,
        report.errors.length === 0 ? "info" : "error",
      );
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  };

  const generate = async () => {
    if (!workspace || !script) return;
    setError(null);
    setBusy("generating");
    setProgress(null);
    try {
      if (dirty) await saveScript();
      const out = await api.generate(workspace.root, script, {
        fake_tts: fakeTts,
        no_preview: noPreview,
      });
      setGenerated(out);
      out.warnings.forEach((w) => pushLog(w, "warn"));
      pushLog(`Generated ${out.output_dir}`);
      setWorkspace(await api.openWorkspace(workspace.root));
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  };

  const cancel = async () => {
    try {
      if (await api.cancelGenerate()) pushLog("cancelling…", "warn");
    } catch (e) {
      fail(e);
    }
  };

  const loadGenerated = async (slug: string) => {
    if (!workspace) return;
    try {
      setGenerated(await api.loadGenerated(workspace.root, slug));
    } catch (e) {
      fail(e);
    }
  };

  const runDoctor = async () => {
    setBusy("doctor");
    setError(null);
    try {
      setDoctor(await api.doctor(workspace?.root ?? null, fakeTts));
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  };

  const reveal = (path: string) => api.revealPath(path).catch(fail);

  const exportYmm4 = async (openAfter: boolean) => {
    if (!workspace || !generated) return;
    setBusy("exporting");
    setError(null);
    try {
      const res = await api.exportYmm4(workspace.root, generated.project_path, openAfter);
      res.warnings.forEach((w) => pushLog(w, "warn"));
      pushLog(`Exported ${res.output}${res.opened ? " (opened in YMM4)" : ""}`);
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  };

  const bundleYmm4 = async () => {
    if (!workspace || !generated) return;
    setBusy("bundling");
    setError(null);
    try {
      const res = await api.bundleYmm4(workspace.root, generated.project_path);
      res.warnings.forEach((w) => pushLog(w, "warn"));
      pushLog(`Bundle created: ${res.zip ?? res.dir}`);
      await reveal(res.zip ?? res.dir);
    } catch (e) {
      fail(e);
    } finally {
      setBusy("idle");
    }
  };

  const captions = useMemo<CaptionClip[]>(
    () =>
      generated?.project.tracks
        .flatMap((t) => t.clips)
        .filter((c): c is CaptionClip => c.type === "caption") ?? [],
    [generated],
  );

  const idle = busy === "idle";
  const canRun = !!workspace && !!script && idle;

  return (
    <div className="app">
      <header className="topbar">
        <h1>VideoForge</h1>
        <span className="muted">{platform?.platform_label ?? "…"}</span>
        <div className="spacer" />
        <button onClick={pickWorkspace} disabled={!idle}>
          Workspace を開く
        </button>
        <button onClick={createWorkspace} disabled={!idle}>
          新規 Workspace
        </button>
        <button onClick={runDoctor} disabled={!idle}>
          Doctor
        </button>
      </header>

      {error && <ErrorBanner error={error} onClose={() => setError(null)} />}

      <main className="layout">
        <section className="panel scripts">
          <h2>Script</h2>
          {workspace ? (
            <>
              <div className="row">
                <span className="muted ellipsis" title={workspace.root}>
                  {workspace.name || "(no name)"} — {workspace.root}
                </span>
                <button className="small" onClick={() => reveal(workspace.root)}>
                  Open Folder
                </button>
              </div>
              <div className="row">
                <select
                  value={script ?? ""}
                  onChange={(e) => selectScript(e.target.value)}
                  disabled={!idle}
                >
                  {workspace.scripts.length === 0 && <option value="">(scripts/*.md がありません)</option>}
                  {workspace.scripts.map((s) => (
                    <option key={s.rel} value={s.rel}>
                      {s.rel}
                    </option>
                  ))}
                </select>
                <button className="small" onClick={saveScript} disabled={!dirty || !idle}>
                  Save{dirty ? " *" : ""}
                </button>
              </div>
              <textarea
                className="editor"
                value={content}
                onChange={(e) => setContent(e.target.value)}
                spellCheck={false}
                disabled={!script || !idle}
              />
              <div className="row">
                <label>
                  <input type="checkbox" checked={fakeTts} onChange={(e) => setFakeTts(e.target.checked)} />
                  fake TTS（VOICEVOX なしで配管確認）
                </label>
                <label>
                  <input type="checkbox" checked={noPreview} onChange={(e) => setNoPreview(e.target.checked)} />
                  preview.mp4 を省略
                </label>
              </div>
              <div className="row actions">
                <button onClick={validate} disabled={!canRun}>
                  Validate
                </button>
                <button className="primary" onClick={generate} disabled={!canRun}>
                  Generate
                </button>
                {busy === "generating" && (
                  <button className="danger" onClick={cancel}>
                    Cancel
                  </button>
                )}
                <span className="muted">Speakers: {workspace.speakers.join(", ")}</span>
              </div>
            </>
          ) : (
            <p className="muted">
              「Workspace を開く」で <code>videoforge.yaml</code> のあるフォルダを選ぶか、「新規 Workspace」で作成してください。
            </p>
          )}
        </section>

        <section className="panel side">
          <h2>Progress</h2>
          {busy === "generating" && progress?.stage === "synthesizing" && (
            <progress value={progress.current} max={progress.total} />
          )}
          {busy === "generating" && progress?.stage !== "synthesizing" && <progress />}
          <div className="log">
            {log.map((l) => (
              <div key={l.id} className={`line ${l.kind}`}>
                {l.text}
              </div>
            ))}
            <div ref={logEnd} />
          </div>

          {validation && (
            <>
              <h2>Validation — {validation.errors.length === 0 ? "OK" : "FAILED"}</h2>
              <div className="muted">
                {validation.title} · slug <code>{validation.slug}</code> · {validation.dialogues.length} dialogues
              </div>
              {validation.errors.map((i, n) => (
                <div key={`e${n}`} className="line error">
                  {i.line != null ? `line ${i.line}: ` : ""}
                  {i.message}
                </div>
              ))}
              {validation.warnings.map((i, n) => (
                <div key={`w${n}`} className="line warn">
                  {i.line != null ? `line ${i.line}: ` : ""}
                  {i.message}
                </div>
              ))}
              <table>
                <tbody>
                  {validation.dialogues.map((d) => (
                    <tr key={d.index}>
                      <td className="num">{String(d.index).padStart(2, "0")}</td>
                      <td>{d.speaker_display}</td>
                      <td className="ellipsis">{d.text}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}

          {doctor && (
            <>
              <h2>Doctor</h2>
              <table>
                <tbody>
                  {doctor.checks.map((c) => (
                    <tr key={c.name} className={`status-${c.status}`}>
                      <td className="num">
                        {c.status === "ok" ? "✓" : c.status === "warn" ? "!" : c.status === "fail" ? "✗" : "-"}
                      </td>
                      <td>{c.name}</td>
                      <td className="ellipsis" title={c.detail}>
                        {c.detail}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="muted">
                VOICEVOX {doctor.capabilities.voicevox_available ? "✓" : "✗"} · FFmpeg{" "}
                {doctor.capabilities.ffmpeg_available ? "✓" : "✗"} · YMM4{" "}
                {doctor.capabilities.can_export_ymm4
                  ? doctor.capabilities.can_open_ymm4
                    ? "export + open"
                    : "export only"
                  : "unavailable"}
              </div>
            </>
          )}
        </section>

        <section className="panel output">
          <h2>
            Output
            {workspace && workspace.generated.length > 0 && (
              <select
                className="inline"
                value={generated?.slug ?? ""}
                onChange={(e) => e.target.value && loadGenerated(e.target.value)}
                disabled={!idle}
              >
                <option value="">generated/…</option>
                {workspace.generated.map((g) => (
                  <option key={g.slug} value={g.slug}>
                    {g.slug}
                    {g.has_preview ? "" : " (no preview)"}
                  </option>
                ))}
              </select>
            )}
          </h2>
          {generated ? (
            <>
              <div className="row">
                <span className="ellipsis" title={generated.output_dir}>
                  {generated.slug} · {formatSeconds(generated.manifest.duration_ms)}s ·{" "}
                  {generated.manifest.dialogues} dialogues
                </span>
                <button className="small" onClick={() => reveal(generated.output_dir)}>
                  Open Folder
                </button>
              </div>
              {previewUrl ? (
                <video className="preview" controls src={previewUrl} />
              ) : (
                <div className="preview placeholder">
                  {generated.preview_path ? "loading preview…" : "preview.mp4 なし（FFmpeg 未検出または省略）"}
                </div>
              )}
              <h3>Timeline</h3>
              <table>
                <tbody>
                  {captions.map((c, i) => (
                    <tr key={c.id}>
                      <td className="num">{String(i + 1).padStart(2, "0")}</td>
                      <td>{c.speaker_display ?? c.speaker}</td>
                      <td className="num">
                        {formatSeconds(c.start_ms)} – {formatSeconds(c.start_ms + c.duration_ms)}
                      </td>
                      <td className="ellipsis" title={c.text}>
                        {c.text}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <h3>YMM4</h3>
              {platform?.can_export_ymm4 ? (
                <div className="row actions">
                  <button onClick={() => exportYmm4(false)} disabled={!idle}>
                    Export YMM4
                  </button>
                  <button
                    onClick={() => exportYmm4(true)}
                    disabled={!idle || !platform.can_open_ymm4}
                    title={platform.can_open_ymm4 ? platform.ymm4_path ?? "" : "YukkuriMovieMaker.exe が見つかりません（VIDEOFORGE_YMM4_PATH）"}
                  >
                    Export &amp; Open in YMM4
                  </button>
                </div>
              ) : (
                <div className="notice">
                  <p>
                    YukkuriMovieMaker4 は {platform?.platform_label ?? "このOS"} では利用できません。
                    VideoForge での生成処理は完了しています。Windows で編集する場合は YMM4 Handoff Bundle を作成してください。
                  </p>
                  <button onClick={bundleYmm4} disabled={!idle}>
                    Create YMM4 Handoff Bundle
                  </button>
                </div>
              )}
            </>
          ) : (
            <p className="muted">Generate すると、ここに preview とタイムラインが表示されます。</p>
          )}
        </section>
      </main>
    </div>
  );
}
