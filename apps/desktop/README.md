# VideoForge Desktop (Tauri v2 + React)

Issue #13 / design §24–§26. A thin GUI over `videoforge-core`: the Rust side
(`src-tauri/`) is a composition root like `videoforge-cli/src/commands.rs` and
contains no validation, pipeline or export logic of its own.

```text
apps/desktop/
├── src/                React + TypeScript frontend (Vite)
│   ├── App.tsx         the single MVP screen
│   ├── api.ts          typed `invoke` wrappers + progress event listener
│   └── types.ts        mirrors of the Rust DTOs
└── src-tauri/          Tauri crate `videoforge-desktop` (its own cargo workspace)
    ├── src/commands.rs #[tauri::command]s → videoforge-core
    ├── src/error.rs    CommandError { code, message } — code = AppError::code()
    ├── src/state.rs    the running generate's CancellationToken
    ├── tauri.conf.json
    └── capabilities/   core:default + dialog:default only
```

## What the MVP does

| Feature | Command(s) |
|---|---|
| Open / create a workspace (native folder picker) | `open_workspace`, `create_workspace` |
| Pick a script under `scripts/`, edit and save it | `read_script`, `write_script` |
| Validate | `validate_script` |
| Generate with live progress and Cancel | `generate` (+ `generate:progress` events), `cancel_generate` |
| Play `preview.mp4` | `read_generated_file` (bytes over IPC → `blob:` URL) |
| Open Folder (Finder / Explorer) | `reveal_path` |
| Doctor | `doctor` |
| Timeline table from `project.vfp.json` | part of `generate` / `load_generated` |
| Windows: Export YMM4 / Export & Open in YMM4 | `export_ymm4` |
| macOS / Linux: "YMM4 unavailable" notice + Create Handoff Bundle | `bundle_ymm4` |

Which YMM4 buttons appear is decided by `platform_info` (Rust-side capability
detection, §25), never by sniffing the OS in the frontend.

Every failure reaches the UI as `{ code, message }` where `code` is the same
`AppError::code()` string `videoforge --json` prints; the banner shows the code
next to the message. `invalid_request` is the one GUI-only code (bad argument
from the frontend).

Out of scope, on purpose: a timeline editor, AI script generation, trend
detection, YouTube upload, asset management.

## Development

Prerequisites: Rust stable, Node 20+, pnpm, and the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS
(Xcode CLT on macOS; WebView2 + MSVC on Windows; on Linux
`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`).

```bash
cd apps/desktop
pnpm install
pnpm tauri dev          # runs vite + the Rust app with hot reload
pnpm build              # tsc --noEmit + vite build (frontend only)
pnpm tauri build        # installers under src-tauri/target/release/bundle/

cd src-tauri
cargo test              # command/path-confinement unit tests
cargo clippy --all-targets -- -D warnings
```

`src-tauri` is deliberately **not** a member of the root cargo workspace (see
`exclude` in the root `Cargo.toml`), so `cargo test --workspace` in the repo root
stays runnable without WebKitGTK. Core crates are pulled in by path.

The same environment variables as the CLI apply: `VIDEOFORGE_CACHE_DIR`,
`VIDEOFORGE_FFMPEG`, `VIDEOFORGE_YMM4_PATH`.

Icons under `src-tauri/icons/` are placeholders generated from a script; replace
them with `pnpm tauri icon <source.png>` when there is a real logo.

## Acceptance checklist (manual, per OS)

The build and the unit tests run in CI; the items below need a human on a real
desktop. Record the result with the OS build, VOICEVOX and FFmpeg versions.

| # | Check | Windows | macOS |
|---|---|---|---|
| 1 | `pnpm tauri dev` starts and shows the window | _pending_ | _pending_ |
| 2 | "新規 Workspace" creates `videoforge.yaml` + `scripts/sample.md`, "Workspace を開く" reopens it | _pending_ | _pending_ |
| 3 | Validate on `sample.md` lists 3 dialogues | _pending_ | _pending_ |
| 4 | Generate (fake TTS checked) completes; progress shows `音声合成 n/3` then `完了` | _pending_ | _pending_ |
| 5 | Generate with VOICEVOX running produces real audio | _pending_ | _pending_ |
| 6 | `preview.mp4` plays in the Output panel (FFmpeg on PATH) | _pending_ | _pending_ |
| 7 | Cancel during synthesis ends with code `cancelled` and no `generated/<slug>` half-written | _pending_ | _pending_ |
| 8 | Stopping VOICEVOX and generating shows code `voicevox_unavailable` in the banner | _pending_ | _pending_ |
| 9 | Open Folder reveals the output directory | _pending_ | _pending_ |
| 10 | Doctor table matches `videoforge doctor` | _pending_ | _pending_ |
| 11 | YMM4 section: Export YMM4 / Export & Open (Windows) — "unavailable" notice + Create Handoff Bundle (macOS) | _pending_ | _pending_ |
