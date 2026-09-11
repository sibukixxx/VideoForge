use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _};
use serde::Serialize;
use videoforge_character::{CharacterManifest, PROVIDER_VOICEVOX};
use videoforge_core::doctor::{self, CheckStatus, DoctorInput};
use videoforge_core::export::{ExportRequest, ProjectExporter};
use videoforge_core::preview::PreviewRenderer;
use videoforge_core::progress::{FnProgress, GenerationStage};
use videoforge_core::project::VideoProject;
use videoforge_core::tts::{FakeTtsEngine, Speaker, TtsCache, TtsEngine};
use videoforge_core::{generate as core_generate, init as core_init, validate as core_validate};
use videoforge_core::{AppError, CancellationToken, GenerateDeps, GenerateOptions, Workspace};
use videoforge_export_ymm4::bundle::{create_bundle, BundleOptions};
use videoforge_export_ymm4::Ymm4Exporter;
use videoforge_platform::{current_platform, PlatformKind};
use videoforge_preview::FfmpegPreviewRenderer;
use videoforge_voicevox::VoicevoxEngine;

pub struct Context {
    pub workspace_override: Option<PathBuf>,
    pub json: bool,
}

impl Context {
    fn workspace_for(&self, hint: Option<&Path>) -> Result<Workspace, AppError> {
        match &self.workspace_override {
            Some(dir) => Workspace::open(dir),
            None => match hint {
                Some(p) => Workspace::discover(p).or_else(|_| Workspace::discover_cwd()),
                None => Workspace::discover_cwd(),
            },
        }
    }

    fn emit_json<T: serde::Serialize>(&self, value: &T) -> anyhow::Result<()> {
        println!("{}", serde_json::to_string_pretty(value)?);
        Ok(())
    }
}

fn make_tts(
    fake: bool,
    endpoint: &str,
    timeout_secs: u64,
    allow_remote_endpoint: bool,
) -> anyhow::Result<Arc<dyn TtsEngine>> {
    if fake {
        return Ok(Arc::new(FakeTtsEngine::default()));
    }
    Ok(Arc::new(VoicevoxEngine::new(
        endpoint,
        Duration::from_secs(timeout_secs.max(1)),
        allow_remote_endpoint,
    )?))
}

fn exit(code: u8) -> anyhow::Result<ExitCode> {
    Ok(ExitCode::from(code))
}

pub fn draft(ctx: &Context, action: crate::DraftAction) -> anyhow::Result<ExitCode> {
    use std::io::{Read, Write};
    use videoforge_core::draft::{self, Brief, Draft};
    let ws = ctx.workspace_for(None)?;
    let config = ws.load_config()?;
    let brief_path = match &action {
        crate::DraftAction::Prompt { brief }
        | crate::DraftAction::Check { brief, .. }
        | crate::DraftAction::Export { brief, .. } => brief,
    };
    let read = |path: &str| -> anyhow::Result<Vec<u8>> {
        let path = ws.resolve(path)?;
        let file = std::fs::File::open(&path).map_err(|e| AppError::read(&path, e))?;
        let mut bytes = Vec::new();
        file.take(1_000_001).read_to_end(&mut bytes)
            .map_err(|e| AppError::read(&path, e))?;
        if bytes.len() > 1_000_000 {
            return Err(AppError::InvalidScript("draft input exceeds 1 MB".into()).into());
        }
        Ok(bytes)
    };
    let brief: Brief = serde_json::from_slice(&read(brief_path)?)
        .map_err(|e| AppError::InvalidScript(e.to_string()))?;
    let response = match &action {
        crate::DraftAction::Prompt { .. } => {
            let prompt = draft::prompt(&brief, &config)?;
            if ctx.json {
                ctx.emit_json(&serde_json::json!({"prompt": prompt}))?;
            } else {
                println!("{prompt}");
            }
            return exit(0);
        }
        crate::DraftAction::Check { response, .. }
        | crate::DraftAction::Export { response, .. } => response,
    };
    let candidate: Draft = serde_json::from_slice(&read(response)?)
        .map_err(|e| AppError::InvalidScript(e.to_string()))?;
    let report = draft::check(&brief, &candidate, &config, &ws)?;
    if let crate::DraftAction::Export { reviewed_hash, reviewer, out, .. } = action {
        if !report.structurally_valid || reviewed_hash != report.review_hash
            || reviewer.trim().is_empty() || reviewer.contains(['\n', '\r'])
        {
            ctx.emit_json(&report)?;
            return exit(2);
        }
        if !out.starts_with("scripts/") || !out.ends_with(".md") {
            return Err(AppError::InvalidScript("output must be scripts/*.md".into()).into());
        }
        let path = ws.resolve(&out)?;
        // create_new refuses overwrite, including symlinks. No automatic generation.
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .open(&path).map_err(|e| AppError::write(&path, e))?;
        let header = format!("# Reviewed draft: {} / {}\n\n", report.review_hash, reviewer);
        // Header goes after front matter so the existing parser retains the title.
        let markdown = report.markdown.replacen("---\n\n", &format!("---\n\n{header}"), 1);
        file.write_all(markdown.as_bytes()).map_err(|e| AppError::write(&path, e))?;
        ctx.emit_json(&serde_json::json!({
            "ok": true, "script": out, "review_hash": report.review_hash,
            "reviewer": reviewer, "generated": false,
        }))?;
        return exit(0);
    }
    ctx.emit_json(&report)?;
    exit(if report.structurally_valid { 0 } else { 2 })
}

// ---------------------------------------------------------------- init

pub fn init(ctx: &Context, dir: Option<PathBuf>, name: Option<String>) -> anyhow::Result<ExitCode> {
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let report = core_init::init(&dir, name.as_deref())?;
    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": true,
            "root": report.root,
            "created": report.created,
        }))?;
    } else {
        println!(
            "Initialized VideoForge workspace at {}",
            report.root.display()
        );
        println!();
        println!("Next steps:");
        if dir != Path::new(".") {
            println!("  cd {}", dir.display());
        }
        println!("  videoforge doctor");
        println!("  videoforge generate scripts/sample.md");
    }
    exit(0)
}

// ---------------------------------------------------------------- doctor

pub async fn doctor(
    ctx: &Context,
    fake_tts: bool,
    endpoint: Option<String>,
) -> anyhow::Result<ExitCode> {
    let workspace = ctx.workspace_for(None);
    let (endpoint, timeout, allow_remote) =
        match workspace.as_ref().ok().and_then(|ws| ws.load_config().ok()) {
            Some(cfg) => (
                endpoint.unwrap_or(cfg.tts.endpoint),
                cfg.tts.timeout_secs,
                cfg.tts.allow_remote_endpoint,
            ),
            None => (
                endpoint.unwrap_or_else(|| videoforge_voicevox::DEFAULT_ENDPOINT.to_string()),
                30,
                false,
            ),
        };
    let tts = make_tts(fake_tts, &endpoint, timeout, allow_remote)?;
    let preview = FfmpegPreviewRenderer::detect();
    let exporter = Ymm4Exporter::new();
    let platform = current_platform();

    let report = doctor::run(DoctorInput {
        workspace,
        tts: tts.as_ref(),
        tts_endpoint: endpoint,
        preview: Some(&preview),
        exporter: Some(&exporter),
        platform: platform.as_ref(),
    })
    .await;

    if ctx.json {
        ctx.emit_json(&report)?;
    } else {
        for check in &report.checks {
            let mark = match check.status {
                CheckStatus::Ok => "✓",
                CheckStatus::Warn => "!",
                CheckStatus::Fail => "✗",
                CheckStatus::Unavailable => "-",
            };
            println!("{mark} {:<20} {}", check.name, check.detail);
        }
        println!();
        let caps = &report.capabilities;
        println!("Platform: {}", caps.platform_label);
        println!(
            "YMM4: {}",
            if caps.can_export_ymm4 {
                if caps.can_open_ymm4 {
                    "export + open"
                } else {
                    "export only"
                }
            } else {
                "unavailable on this platform (use `videoforge bundle ymm4`)"
            }
        );
    }
    exit(if report.has_failures() { 2 } else { 0 })
}

// ---------------------------------------------------------------- speakers

pub async fn speakers(
    ctx: &Context,
    fake_tts: bool,
    endpoint: Option<String>,
) -> anyhow::Result<ExitCode> {
    let cfg = ctx
        .workspace_for(None)
        .ok()
        .and_then(|ws| ws.load_config().ok());
    let endpoint = endpoint
        .or_else(|| cfg.as_ref().map(|c| c.tts.endpoint.clone()))
        .unwrap_or_else(|| videoforge_voicevox::DEFAULT_ENDPOINT.to_string());
    let allow_remote = cfg.as_ref().is_some_and(|c| c.tts.allow_remote_endpoint);
    let tts = make_tts(fake_tts, &endpoint, 30, allow_remote)?;
    let speakers = tts.list_speakers().await?;
    if ctx.json {
        ctx.emit_json(&speakers)?;
    } else {
        for s in &speakers {
            println!("{}", s.name);
            for st in &s.styles {
                println!("  {:>4}  {}", st.id, st.name);
            }
        }
    }
    exit(0)
}

// ---------------------------------------------------------------- validate

pub fn validate(ctx: &Context, script: PathBuf) -> anyhow::Result<ExitCode> {
    let ws = ctx.workspace_for(Some(&script))?;
    let cfg = ws.load_config()?;
    let script = resolve_script(&ws, &script)?;
    let report = core_validate::validate_file(&ws, &cfg, &script);
    if ctx.json {
        ctx.emit_json(&report)?;
    } else {
        println!("Script:   {}", report.script);
        if report.is_ok() {
            println!("Title:    {}", report.title);
            println!("Slug:     {}", report.slug);
            println!(
                "Dialogues: {} ({} chars)",
                report.dialogues.len(),
                report.total_chars
            );
            for d in &report.dialogues {
                let preview: String = d.text.chars().take(30).collect();
                println!(
                    "  {:03} {:<8} {}{}",
                    d.index,
                    d.speaker_display,
                    preview.replace('\n', " "),
                    if d.text.chars().count() > 30 {
                        "…"
                    } else {
                        ""
                    }
                );
            }
            if !report.directives.is_empty() {
                println!(
                    "Directives: {} ({} with a missing asset)",
                    report.directives.len(),
                    report.directives.iter().filter(|d| !d.exists).count()
                );
            }
        }
        print_issues("warning", &report.warnings);
        print_issues("error", &report.errors);
        println!();
        if report.is_ok() {
            println!("OK");
        } else {
            println!("FAILED: {} error(s)", report.errors.len());
        }
    }
    exit(if report.is_ok() { 0 } else { 2 })
}

fn print_issues(kind: &str, issues: &[core_validate::ValidationIssue]) {
    for i in issues {
        match i.line {
            Some(l) => println!("{kind}: line {l}: {}", i.message),
            None => println!("{kind}: {}", i.message),
        }
    }
}

fn resolve_script(ws: &Workspace, script: &Path) -> anyhow::Result<PathBuf> {
    if script.is_file() {
        return Ok(script.to_path_buf());
    }
    if let Ok(in_ws) = ws.resolve(&script.to_string_lossy()) {
        if in_ws.is_file() {
            return Ok(in_ws);
        }
    }
    Err(anyhow!("script not found: {}", script.display()))
}

// ---------------------------------------------------------------- generate

pub struct GenerateArgs {
    pub script: PathBuf,
    pub no_preview: bool,
    pub srt_speaker: bool,
    pub no_cache: bool,
    pub keep_tmp: bool,
    pub fake_tts: bool,
    pub endpoint: Option<String>,
}

pub async fn generate(ctx: &Context, args: GenerateArgs) -> anyhow::Result<ExitCode> {
    let ws = ctx.workspace_for(Some(&args.script))?;
    let cfg = ws.load_config()?;
    let script = resolve_script(&ws, &args.script)?;

    let endpoint = args.endpoint.unwrap_or(cfg.tts.endpoint.clone());
    let tts = make_tts(
        args.fake_tts,
        &endpoint,
        cfg.tts.timeout_secs,
        cfg.tts.allow_remote_endpoint,
    )?;
    let platform = current_platform();
    let cache = if args.no_cache {
        None
    } else {
        match platform.cache_dir() {
            Ok(dir) => Some(Arc::new(TtsCache::in_platform_cache(&dir))),
            Err(e) => {
                eprintln!("warning: TTS cache disabled: {e}");
                None
            }
        }
    };
    let preview: Option<Arc<dyn PreviewRenderer>> = Some(Arc::new(FfmpegPreviewRenderer::detect()));

    let json = ctx.json;
    let progress = Arc::new(FnProgress(move |stage: &GenerationStage| {
        if json {
            return;
        }
        match stage {
            GenerationStage::Parsing => eprintln!("→ Parsing script"),
            GenerationStage::Validating => eprintln!("→ Validating"),
            GenerationStage::Synthesizing {
                current,
                total,
                index,
                speaker,
                cached,
            } => eprintln!(
                "→ TTS {current}/{total}  #{index:03} {speaker}{}",
                if *cached { " (cached)" } else { "" }
            ),
            GenerationStage::BuildingTimeline => eprintln!("→ Building timeline"),
            GenerationStage::WritingProject => eprintln!("→ Writing project.vfp.json"),
            GenerationStage::WritingCaptions => eprintln!("→ Writing captions.srt"),
            GenerationStage::RenderingPreview => eprintln!("→ Rendering preview.mp4 (ffmpeg)"),
            GenerationStage::PreviewSkipped { reason } => eprintln!("→ {reason}"),
            GenerationStage::Completed => eprintln!("→ Done"),
        }
    }));

    let cancel = CancellationToken::new();
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("cancelling…");
                cancel.cancel();
            }
        });
    }

    let deps = GenerateDeps {
        tts,
        cache,
        preview,
        progress,
    };
    let options = GenerateOptions {
        preview: if args.no_preview { Some(false) } else { None },
        srt_include_speaker: args.srt_speaker,
        cancel,
        keep_tmp_on_failure: args.keep_tmp,
    };
    let out = core_generate::generate(&ws, &script, options, deps).await?;

    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": true,
            "slug": out.slug,
            "output_dir": out.output_dir,
            "project": out.project_path,
            "preview": out.preview_path,
            "manifest": out.manifest,
        }))?;
    } else {
        println!();
        println!("Generated {}", out.output_dir.display());
        println!(
            "  project.vfp.json   {} dialogue(s), {:.2}s",
            out.manifest.dialogues,
            out.manifest.duration_ms as f64 / 1000.0
        );
        println!("  captions.srt");
        println!("  assets/audio/      {} file(s)", out.manifest.audio.len());
        match &out.preview_path {
            Some(p) => println!("  preview.mp4        {}", p.display()),
            None => println!("  preview.mp4        (skipped)"),
        }
        for w in &out.warnings {
            println!("warning: {w}");
        }
        if platform.kind() != PlatformKind::Windows {
            println!();
            println!("YMM4 export requires Windows. To hand off:");
            println!(
                "  videoforge bundle ymm4 {}",
                ws.relative(&out.project_path)
            );
        } else {
            println!();
            println!(
                "Next: videoforge export ymm4 {}",
                ws.relative(&out.project_path)
            );
        }
    }
    exit(0)
}

// ---------------------------------------------------------------- export ymm4

pub async fn export_ymm4(
    ctx: &Context,
    project_path: PathBuf,
    template: Option<PathBuf>,
    out: Option<PathBuf>,
    open: bool,
    force: bool,
) -> anyhow::Result<ExitCode> {
    let project_path = project_path
        .canonicalize()
        .with_context(|| format!("project not found: {}", project_path.display()))?;
    let project_dir = project_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let project = VideoProject::load(&project_path)?;
    let workspace = ctx.workspace_for(Some(&project_path)).ok();
    let exporter = Ymm4Exporter {
        force_non_windows: force,
    };
    let caps = exporter.capabilities();
    if !caps.available {
        let msg = videoforge_export_ymm4::NON_WINDOWS_MESSAGE
            .replace("<project.vfp.json>", &project_path.display().to_string());
        if ctx.json {
            ctx.emit_json(&serde_json::json!({
                "ok": false,
                "code": "ymm4_unavailable",
                "error": msg,
                "suggest": format!("videoforge bundle ymm4 {}", project_path.display()),
            }))?;
        } else {
            eprintln!("Error:\n{msg}");
        }
        return exit(2);
    }

    let result = exporter
        .export(ExportRequest {
            project: &project,
            project_path: &project_path,
            project_dir: &project_dir,
            workspace: workspace.as_ref(),
            template: template.as_deref(),
            destination: out.as_deref(),
        })
        .await?;

    let platform = current_platform();
    let mut opened = false;
    if open {
        match platform.open_in_ymm4(&result.output) {
            Ok(()) => opened = true,
            Err(e) => eprintln!("warning: could not open YMM4: {e}"),
        }
    }

    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": true,
            "output": result.output,
            "warnings": result.warnings,
            "opened": opened,
        }))?;
    } else {
        println!("Exported {}", result.output.display());
        for w in &result.warnings {
            println!("warning: {w}");
        }
        if force && platform.kind() != PlatformKind::Windows {
            println!("note: --force on a non-Windows host; asset paths are not valid for YMM4");
        }
    }
    exit(0)
}

// ---------------------------------------------------------------- bundle ymm4

pub fn bundle_ymm4(
    ctx: &Context,
    project_path: PathBuf,
    out: Option<PathBuf>,
    template: Option<PathBuf>,
    zip: bool,
    force: bool,
) -> anyhow::Result<ExitCode> {
    let project_path = project_path
        .canonicalize()
        .with_context(|| format!("project not found: {}", project_path.display()))?;
    let workspace = ctx.workspace_for(Some(&project_path)).ok();
    let result = create_bundle(
        &project_path,
        workspace.as_ref(),
        BundleOptions {
            out_dir: out,
            template,
            zip,
            force,
        },
    )?;
    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": true,
            "dir": result.dir,
            "zip": result.zip,
            "warnings": result.warnings,
        }))?;
    } else {
        println!("Bundle created: {}", result.dir.display());
        if let Some(z) = &result.zip {
            println!("Zip:            {}", z.display());
        }
        for w in &result.warnings {
            println!("warning: {w}");
        }
        println!();
        println!("On Windows:");
        println!("  videoforge export ymm4 <bundle>/project.vfp.json");
    }
    exit(0)
}

// ---------------------------------------------------------------- character

#[derive(Serialize)]
struct CharacterVoiceReport {
    provider: String,
    speaker: String,
    style: String,
    /// Only filled by `character validate`, which has a TTS engine to ask.
    resolved_speaker_id: Option<u32>,
    error: Option<String>,
}

#[derive(Serialize)]
struct CharacterModelReport {
    #[serde(rename = "type")]
    model_type: String,
    path: String,
    resolved_path: PathBuf,
    expressions: Vec<String>,
    motions: Vec<String>,
    error: Option<String>,
}

#[derive(Serialize)]
struct CharacterEntryReport {
    id: String,
    display_name: String,
    voice: Option<CharacterVoiceReport>,
    model: Option<CharacterModelReport>,
    /// `false` if anything above failed. `character inspect` never checks
    /// the voice against a live engine, so a character with no model is
    /// always `ok` there even though `character validate` might still find
    /// its voice unresolvable.
    ok: bool,
}

/// Build one report per character: model file is always checked (offline,
/// local file I/O); the voice is only checked against `tts` when given
/// (`character validate`) — `character inspect` reports the voice as
/// declared, unchecked.
async fn build_character_reports(
    manifest: &CharacterManifest,
    manifest_dir: &Path,
    tts: Option<&dyn TtsEngine>,
) -> Vec<CharacterEntryReport> {
    let mut speakers_cache: Option<anyhow::Result<Vec<Speaker>>> = None;
    let mut out = Vec::with_capacity(manifest.characters.len());
    for c in &manifest.characters {
        let mut ok = true;

        let voice = if let Some(v) = &c.voice {
            let (resolved_speaker_id, error) = match tts {
                None => (None, None),
                Some(tts) => {
                    if speakers_cache.is_none() {
                        speakers_cache =
                            Some(tts.list_speakers().await.map_err(anyhow::Error::from));
                    }
                    match speakers_cache.as_ref().unwrap() {
                        Ok(speakers) => {
                            let found = speakers
                                .iter()
                                .find(|s| s.name == v.speaker)
                                .and_then(|s| s.styles.iter().find(|st| st.name == v.style));
                            match found {
                                Some(st) => (Some(st.id), None),
                                None => (
                                    None,
                                    Some(format!(
                                        "VOICEVOX has no speaker `{}` with style `{}`",
                                        v.speaker, v.style
                                    )),
                                ),
                            }
                        }
                        Err(e) => (None, Some(e.to_string())),
                    }
                }
            };
            if v.provider != PROVIDER_VOICEVOX || error.is_some() {
                ok = false;
            }
            Some(CharacterVoiceReport {
                provider: v.provider.clone(),
                speaker: v.speaker.clone(),
                style: v.style.clone(),
                resolved_speaker_id,
                error,
            })
        } else {
            None
        };

        let model = if let Some(m) = &c.model {
            let resolved_path = m.resolve_path(manifest_dir);
            match videoforge_character::live2d::load_model3_json(&resolved_path) {
                Ok(info) => Some(CharacterModelReport {
                    model_type: m.model_type.clone(),
                    path: m.path.clone(),
                    resolved_path,
                    expressions: info.expressions,
                    motions: info.motions,
                    error: None,
                }),
                Err(e) => {
                    ok = false;
                    Some(CharacterModelReport {
                        model_type: m.model_type.clone(),
                        path: m.path.clone(),
                        resolved_path,
                        expressions: Vec::new(),
                        motions: Vec::new(),
                        error: Some(e.to_string()),
                    })
                }
            }
        } else {
            None
        };

        out.push(CharacterEntryReport {
            id: c.id.clone(),
            display_name: c.display_name.clone(),
            voice,
            model,
            ok,
        });
    }
    out
}

fn print_character_report(r: &CharacterEntryReport, checked: bool) {
    let mark = if !checked {
        " "
    } else if r.ok {
        "✓"
    } else {
        "✗"
    };
    println!();
    println!("{mark} {}  ({})", r.display_name, r.id);
    if let Some(v) = &r.voice {
        match (&v.error, v.resolved_speaker_id) {
            (Some(e), _) => println!("  voice: {}/{} — {e}", v.speaker, v.style),
            (None, Some(id)) => {
                println!("  voice: {}/{} -> speaker_id {id}", v.speaker, v.style)
            }
            (None, None) => println!("  voice: {} / {} / {}", v.provider, v.speaker, v.style),
        }
    }
    if let Some(m) = &r.model {
        println!("  model: {} at {}", m.model_type, m.resolved_path.display());
        match &m.error {
            Some(e) => println!("    ! {e}"),
            None => {
                let list = |v: &[String]| {
                    if v.is_empty() {
                        "(none declared)".to_string()
                    } else {
                        v.join(", ")
                    }
                };
                println!("    expressions: {}", list(&m.expressions));
                println!("    motions:     {}", list(&m.motions));
            }
        }
    }
}

pub async fn character_inspect(ctx: &Context, manifest_path: PathBuf) -> anyhow::Result<ExitCode> {
    let manifest = CharacterManifest::load(&manifest_path)
        .with_context(|| format!("character manifest: {}", manifest_path.display()))?;
    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));
    let reports = build_character_reports(&manifest, manifest_dir, None).await;

    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": true,
            "manifest": manifest_path,
            "characters": reports,
        }))?;
    } else {
        println!("Manifest: {}", manifest_path.display());
        for r in &reports {
            print_character_report(r, false);
        }
    }
    exit(0)
}

pub async fn character_validate(
    ctx: &Context,
    manifest_path: PathBuf,
    fake_tts: bool,
    endpoint: Option<String>,
) -> anyhow::Result<ExitCode> {
    let manifest = CharacterManifest::load(&manifest_path)
        .with_context(|| format!("character manifest: {}", manifest_path.display()))?;
    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));

    let cfg = ctx
        .workspace_for(Some(&manifest_path))
        .ok()
        .and_then(|ws| ws.load_config().ok());
    let endpoint = endpoint
        .or_else(|| cfg.as_ref().map(|c| c.tts.endpoint.clone()))
        .unwrap_or_else(|| videoforge_voicevox::DEFAULT_ENDPOINT.to_string());
    let allow_remote = cfg.as_ref().is_some_and(|c| c.tts.allow_remote_endpoint);
    let tts = make_tts(fake_tts, &endpoint, 30, allow_remote)?;

    let reports = build_character_reports(&manifest, manifest_dir, Some(tts.as_ref())).await;
    let ok = reports.iter().all(|r| r.ok);

    if ctx.json {
        ctx.emit_json(&serde_json::json!({
            "ok": ok,
            "manifest": manifest_path,
            "characters": reports,
        }))?;
    } else {
        println!("Manifest: {}", manifest_path.display());
        for r in &reports {
            print_character_report(r, true);
        }
        println!();
        println!("{}", if ok { "OK" } else { "FAILED" });
    }
    exit(if ok { 0 } else { 2 })
}
