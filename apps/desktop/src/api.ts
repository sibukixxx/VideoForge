// Typed wrappers around the Tauri commands. Argument keys are snake_case to
// match `#[tauri::command(rename_all = "snake_case")]` on the Rust side.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  BundleResponse,
  CommandError,
  DoctorReport,
  ExportResponse,
  GeneratedInfo,
  GenerateRequest,
  GenerationStage,
  PlatformInfo,
  ScriptEntry,
  ValidationReport,
  WorkspaceInfo,
} from "./types";

export const PROGRESS_EVENT = "generate:progress";

export function isCommandError(e: unknown): e is CommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as CommandError).code === "string" &&
    typeof (e as CommandError).message === "string"
  );
}

/** Anything thrown by `invoke` becomes a CommandError; `other` is the fallback code. */
export function toCommandError(e: unknown): CommandError {
  if (isCommandError(e)) return e;
  return { code: "other", message: e instanceof Error ? e.message : String(e) };
}

export const api = {
  platformInfo: () => invoke<PlatformInfo>("platform_info"),

  doctor: (workspace: string | null, fake_tts: boolean, endpoint: string | null = null) =>
    invoke<DoctorReport>("doctor", { workspace, fake_tts, endpoint }),

  createWorkspace: (dir: string, name: string | null) =>
    invoke<WorkspaceInfo>("create_workspace", { dir, name }),

  openWorkspace: (root: string) => invoke<WorkspaceInfo>("open_workspace", { root }),

  readScript: (root: string, script: string) =>
    invoke<string>("read_script", { root, script }),

  writeScript: (root: string, script: string, content: string) =>
    invoke<ScriptEntry>("write_script", { root, script, content }),

  validateScript: (root: string, script: string) =>
    invoke<ValidationReport>("validate_script", { root, script }),

  generate: (root: string, script: string, request: GenerateRequest) =>
    invoke<GeneratedInfo>("generate", { root, script, request }),

  cancelGenerate: () => invoke<boolean>("cancel_generate"),

  loadGenerated: (root: string, slug: string) =>
    invoke<GeneratedInfo>("load_generated", { root, slug }),

  readGeneratedFile: (root: string, path: string) =>
    invoke<ArrayBuffer>("read_generated_file", { root, path }),

  revealPath: (path: string) => invoke<void>("reveal_path", { path }),

  exportYmm4: (root: string, project_path: string, open_after: boolean) =>
    invoke<ExportResponse>("export_ymm4", { root, project_path, open_after }),

  openInYmm4: (root: string, ymmp_path: string) =>
    invoke<void>("open_in_ymm4", { root, ymmp_path }),

  bundleYmm4: (root: string, project_path: string) =>
    invoke<BundleResponse>("bundle_ymm4", { root, project_path }),

  onProgress: (handler: (stage: GenerationStage) => void): Promise<UnlistenFn> =>
    listen<GenerationStage>(PROGRESS_EVENT, (event) => handler(event.payload)),
};

export function describeStage(stage: GenerationStage): string {
  switch (stage.stage) {
    case "parsing":
      return "台本を解析中";
    case "validating":
      return "検証中";
    case "synthesizing":
      return `音声合成 ${stage.current}/${stage.total}  #${String(stage.index).padStart(3, "0")} ${stage.speaker}${stage.cached ? "（キャッシュ）" : ""}`;
    case "building_timeline":
      return "タイムラインを構築中";
    case "writing_project":
      return "project.vfp.json を書き出し中";
    case "writing_captions":
      return "captions.srt を書き出し中";
    case "rendering_preview":
      return "preview.mp4 をレンダリング中（FFmpeg）";
    case "preview_skipped":
      return stage.reason;
    case "completed":
      return "完了";
  }
}

export function formatSeconds(ms: number): string {
  return (ms / 1000).toFixed(2);
}
