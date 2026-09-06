// Mirrors of the Rust DTOs in src-tauri/src/commands.rs and the core types
// they embed. Keep field names identical to the serde output.

export type PlatformKind = "windows" | "macos" | "linux" | "other";

export interface CommandError {
  /** `AppError::code()` — the same stable strings `videoforge --json` prints. */
  code: string;
  message: string;
}

export interface PlatformInfo {
  platform: PlatformKind;
  platform_label: string;
  can_export_ymm4: boolean;
  can_open_ymm4: boolean;
  ymm4_path: string | null;
  ymm4_unavailable_reason: string | null;
}

export type CheckStatus = "ok" | "warn" | "fail" | "unavailable";

export interface DoctorCheck {
  name: string;
  status: CheckStatus;
  detail: string;
}

export interface Capabilities {
  platform: PlatformKind;
  platform_label: string;
  voicevox_available: boolean;
  voicevox_endpoint: string;
  ffmpeg_available: boolean;
  can_export_ymm4: boolean;
  can_open_ymm4: boolean;
}

export interface DoctorReport {
  checks: DoctorCheck[];
  capabilities: Capabilities;
}

export interface ScriptEntry {
  rel: string;
  name: string;
}

export interface GeneratedEntry {
  slug: string;
  output_dir: string;
  has_preview: boolean;
}

export interface WorkspaceInfo {
  root: string;
  name: string;
  speakers: string[];
  tts_endpoint: string;
  scripts: ScriptEntry[];
  generated: GeneratedEntry[];
}

export interface ValidationIssue {
  line: number | null;
  message: string;
}

export interface ResolvedDialogue {
  index: number;
  speaker_key: string;
  speaker_display: string;
  text: string;
  line: number;
}

export interface ValidationReport {
  script: string;
  title: string;
  slug: string;
  template: string | null;
  errors: ValidationIssue[];
  warnings: ValidationIssue[];
  dialogues: ResolvedDialogue[];
  total_chars: number;
}

export type GenerationStage =
  | { stage: "parsing" }
  | { stage: "validating" }
  | {
      stage: "synthesizing";
      current: number;
      total: number;
      index: number;
      speaker: string;
      cached: boolean;
    }
  | { stage: "building_timeline" }
  | { stage: "writing_project" }
  | { stage: "writing_captions" }
  | { stage: "rendering_preview" }
  | { stage: "preview_skipped"; reason: string }
  | { stage: "completed" };

export interface GenerateRequest {
  no_preview?: boolean;
  srt_speaker?: boolean;
  no_cache?: boolean;
  fake_tts?: boolean;
  endpoint?: string | null;
}

export interface Manifest {
  schema_version: number;
  generator_version: string;
  source: string;
  project: string;
  preview: string | null;
  captions: string;
  audio: string[];
  generated_at: string;
  platform: string;
  duration_ms: number;
  dialogues: number;
  warnings: string[];
}

export interface AudioClip {
  type: "audio";
  id: string;
  source: string;
  start_ms: number;
  duration_ms: number;
  speaker: string;
}

export interface CaptionClip {
  type: "caption";
  id: string;
  text: string;
  start_ms: number;
  duration_ms: number;
  speaker: string;
  speaker_display: string | null;
}

export interface BackgroundClip {
  type: "background";
  id: string;
  source: string;
  start_ms: number;
  duration_ms: number;
}

export type Clip = AudioClip | CaptionClip | BackgroundClip;

export interface Track {
  id: string;
  kind: string;
  clips: Clip[];
}

/** Only the parts of project.vfp.json the timeline table needs. */
export interface VideoProject {
  id: string;
  title: string;
  video: { width: number; height: number; fps: number };
  tracks: Track[];
}

export interface GeneratedInfo {
  slug: string;
  output_dir: string;
  project_path: string;
  preview_path: string | null;
  manifest: Manifest;
  project: VideoProject;
  warnings: string[];
}

export interface ExportResponse {
  output: string;
  warnings: string[];
  opened: boolean;
}

export interface BundleResponse {
  dir: string;
  zip: string | null;
  warnings: string[];
}
