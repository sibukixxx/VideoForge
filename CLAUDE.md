# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

VideoForge compiles Markdown scripts into voice, captions, a timeline IR, a preview video, and
YMM4 editor projects. See `README.md` for the user-facing quick start and script format, and
`docs/design/mvp-v0.2-cross-platform.md` for the design. Module doc comments cite `§N` — those
are headings in that design doc; look them up instead of guessing.

A speaker can optionally link to a **character** (a VOICEVOX voice resolved by name, plus an
optional Live2D model or a `png_lipsync` 3-state PNG model) for deterministic lip-sync data on the
timeline — entirely additive, see `docs/character-system.md`. A `png_lipsync` character is
actually composited into `preview.mp4` (closed/half/open mouth sprites swapped by amplitude,
`videoforge_preview::command::build_character_overlays`); Live2D still only produces timeline data.
`docs/character-video-pipeline.md` and `docs/live2d-renderer-decision.md` cover the pipeline and
the (not yet implemented) Live2D frame-rendering plan; `docs/character-licensing.md` tracks the
unresolved licensing questions around any specific character/model a user configures.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check

# single test / single crate
cargo test -p videoforge-core end_to_end_with_fake_tts
cargo test -p videoforge-cli --test cli          # the offline end-to-end CLI test
cargo test -p videoforge-project -- --nocapture

# run the CLI without installing
cargo run -p videoforge-cli -- doctor --fake-tts --json
cargo run -p videoforge-cli -- generate scripts/sample.md --fake-tts --no-preview
cargo run -p videoforge-cli -- character inspect fixtures/character/mock-character/manifest.yaml
```

CI (`docs/ci/github-actions-ci.yml`, not yet under `.github/workflows/`) runs fmt → clippy →
test → release build → an offline smoke run of `init → validate → generate → bundle → export`, with
FFmpeg installed so the real-FFmpeg tests execute, plus a separate desktop job (pnpm build + cargo
check/test of the Tauri crate).

### Working without VOICEVOX / FFmpeg

`--fake-tts` swaps in an in-process silent engine, `--no-preview` skips FFmpeg. Every test in the
repo runs offline this way. When invoking the binary in tests or scratch runs, set
`VIDEOFORGE_CACHE_DIR` to a temp dir so the real OS TTS cache is not touched, and
`VIDEOFORGE_FFMPEG=/definitely/not/ffmpeg` to force the preview-unavailable path.

Env vars: `VIDEOFORGE_CACHE_DIR`, `VIDEOFORGE_DATA_DIR`, `VIDEOFORGE_FFMPEG`,
`VIDEOFORGE_YMM4_PATH`.

Two test files use the real thing when it is around and print `skipped:` otherwise:
`crates/videoforge-preview/tests/ffmpeg_real.rs` (FFmpeg on `PATH` / `VIDEOFORGE_FFMPEG`) and
`crates/videoforge-{voicevox,cli}/tests/voicevox_real.rs` (VOICEVOX at `127.0.0.1:50021`, or
`VIDEOFORGE_VOICEVOX_ENDPOINT`). The manual halves are `docs/testing/*-manual-e2e.md`.

`videoforge export ymm4 --force` is a hidden flag (`hide = true`, absent from `--help`). It is the
only way to exercise the YMM4 exporter off Windows — the CLI test and macOS development use it.
Its output has non-Windows paths and is not openable in YMM4.

## Architecture

### Dependency direction

`cli → core ← {voicevox, preview, export-ymm4}`. `videoforge-core` must never depend on Tauri,
Windows APIs, or the YMM4 schema. OS-specific behaviour lives behind `videoforge-platform`.

Four trait seams, all defined in core, all implemented in leaf crates:

| Trait | Defined in | Implemented by |
|---|---|---|
| `TtsEngine` | `core/src/tts/mod.rs` | `videoforge-voicevox`, `core::tts::FakeTtsEngine` |
| `PreviewRenderer` | `core/src/preview.rs` | `videoforge-preview` (FFmpeg) |
| `ProjectExporter` | `core/src/export.rs` | `videoforge-export-ymm4` |
| `Platform` | `videoforge-platform` | per-OS impls in that crate |

There are two composition roots, and they are the only places that pick concrete implementations
(`make_tts` → `VoicevoxEngine` or `FakeTtsEngine`, `FfmpegPreviewRenderer::detect`, `Ymm4Exporter`,
`TtsCache` rooted at the platform cache dir): `crates/videoforge-cli/src/commands.rs` and
`apps/desktop/src-tauri/src/commands.rs` (Tauri). A new engine or exporter means a new crate
implementing the trait plus wiring in both — core stays untouched. Behaviour the GUI needs goes into
core, never into the Tauri crate.

`apps/desktop/src-tauri` is **not** a member of the root cargo workspace (`exclude` in `Cargo.toml`):
`tauri` needs WebKitGTK on Linux, and the root `cargo test --workspace` must stay runnable without it.
Check it separately with `cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`;
the frontend with `cd apps/desktop && pnpm install && pnpm build`. The GUI reports failures as
`{ code, message }` where `code` is `AppError::code()`.

### Generation pipeline

`core::generate::generate` runs: parse → validate → TTS (concurrent, cancellable) → timeline →
project IR → SRT → preview → manifest. Everything is written into
`.generated-tmp/<slug>-<pid>-<nonce>/` and renamed onto `generated/<slug>/` only on success; an
existing output dir is moved aside as `generated/<slug>.old-<pid>-<nonce>` and rolled back if the
rename fails. Never write directly into `generated/`.

One slug is generated by one process at a time: from validation through the promote, the pipeline
holds an OS advisory lock on `generated/.locks/<slug>.lock` (fs4, `flock`/`LockFileEx`). A second
generate of the same slug fails immediately with `AppError::Busy` rather than waiting, so an agent
gets an answer instead of a hang. The lock file is reused and never deleted — unlinking it would
let two waiters lock different inodes and both believe they won.

Failures that are recoverable (no FFmpeg, no renderer) become warnings in the manifest via
`GenerationStage::PreviewSkipped` rather than errors.

### Visual track compositing (P1-1/P1-2/P1-5)

There is no separate "VisualTrack" hierarchy: `Clip::Image`/`Clip::Character`/`Clip::Video` all
carry the same `Transform` (position/scale/rotation/opacity/z-order/crop) and `Presentation`
(role/intent) every visual clip has had since P0 — a new clip kind means a new `Clip` variant, not
a new placement system. `videoforge_preview::command::build_visual_layers` collects them (sorted by
`Transform::layer`) into one shared compositing pass: each layer is its own FFmpeg input, filtered
(static crop → Ken Burns pan/zoom for *still images only* → `fit` scale into `frame × scale` →
rotation → opacity → `"fade"` intent's alpha ramp) and `overlay`'d onto the running frame with
`enable='between(t,start,end)'`, in this fixed order: background → visual layers → P0-1's character
`_performance` mouth overlays → captions → on-screen `Text` clips → the whole-video fade in/out.
Captions/text are always composited last so nothing is ever hidden behind them. A `Video` layer's
source is `trim`med to `[trim_start_ms, trim_start_ms + duration_ms)` then `setpts`-shifted so its
timestamps land on its absolute timeline position — this is what makes a video layer's `t` mean the
same "absolute timeline seconds" as an image layer's, which the crop/fade expressions above rely on.
Pan/zoom (`crop=...:eval=frame` referencing `t`, not the `zoompan` filter — see the module doc on
`RenderPlan::ken_burns_filter` for why) has not been verified against a real FFmpeg in this repo yet
(see "Known gaps"); an overlapping crossfade between adjacent clips is not implemented (each clip's
own `"fade"` only ramps its own alpha at its own boundaries, not a coordinated dissolve with its
neighbor) — "don't build a complex editor" per the P1-5 brief.

### Incremental build (P0-4, minimal)

TTS already skips synthesis per-dialogue via `TtsCache` (keyed on engine version + voice params +
text — see "TTS cache" below). `core::buildcache::preview_fingerprint` extends the same idea to the
other expensive stage: it hashes the project IR JSON + the resolved font's actual bytes +
`preview.background_color` + the renderer's `id()`. If that fingerprint matches the sidecar
`.preview.fingerprint` next to the *previous* `generated/<slug>/preview.mp4`, `generate` copies that
file instead of invoking FFmpeg again (`GenerationStage::PreviewCacheHit`). Timeline scheduling and
caption rendering are pure/cheap and are not separately cached. This is not a general per-stage
dependency graph — there is no `--from`/`--only` yet (see "Known gaps").

## Invariants

These span files and are easy to break silently:

- **`project.vfp.json` is the single source of truth.** `.ymmp`, `captions.srt`, `preview.mp4`
  are derived. Never hand-edit generated output; change the script or the config and regenerate.
- **`RelativeAssetPath`** (`videoforge-project/src/path.rs`) is the only asset path type in the IR:
  forward slashes, relative to the project dir, no `..`, no drive-letter `:` component. Absolute
  paths appear only at the moment of YMM4 materialization.
- **Config-supplied paths never escape the workspace.** `Workspace::resolve` returns a `Result` and
  rejects traversal; use `resolve_allow_absolute` only where an absolute path is legitimately
  allowed (the preview font). Don't "simplify" either back into an infallible join.
- **Milliseconds are canonical.** Frames are computed only at export time via `millis_to_frame`,
  so changing fps never rewrites the project.
- **`Config` is `#[serde(deny_unknown_fields)]`** — a new `videoforge.yaml` key needs a struct
  field *and* an update to `config::default_config_yaml`. **`VideoProject`, by contrast, preserves
  unknown fields** through `#[serde(flatten)] extra`; bump `project::SCHEMA_VERSION` only for
  incompatible changes.
- **`AppError::code()` strings are a public contract** — `--json` emits one as the `code` field on
  every failure, and a GUI would dispatch on it. Add a variant rather than widening
  `AppError::Other`; `Other` means "we did not classify this yet".
- **Exit codes**: 0 ok, 1 error, 2 validation/doctor failure.
- **`videoforge.yaml` numbers are range-checked at load** (`config::validate`), and voice scales
  are rejected unless finite — a NaN passes every `<`/`>` comparison, so the finite check has to
  come first. The bounds and their justification are commented at the top of `config.rs`.
- **The TTS endpoint is loopback-only by default** (`config::check_endpoint_allowed`, enforced both
  at config load and at engine construction so a `--endpoint` override can't bypass it).
  `tts.allow_remote_endpoint: true` is the opt-in.
- **Writes that would destroy user data ask first.** `bundle ymm4` refuses an existing output
  directory unless `--force`, and even then only when it looks like a VideoForge bundle.
- **TTS cache** is SHA256(schema, engine, engine *version*, speaker_id, text, speed, pitch,
  intonation, volume) under `<OS cache dir>/tts/v<CACHE_SCHEMA_VERSION>/`, never inside the
  workspace. The version comes from `TtsEngine::health()` via `tts::bind_cache`; if that fails the run
  proceeds **without** the cache and records a `TTS cache disabled` warning in the manifest — never
  fall back to a version-less key. Changing the hashed inputs means bumping `CACHE_SCHEMA_VERSION`,
  which also moves entries to a new directory so old and new keys cannot collide.
- **Filtergraph values are escaped twice** (`videoforge_preview::quote_filter_value`): option-pass
  escaping (`\`, `'`, `:`, edge whitespace) and then graph-pass quoting. Quoting alone silently
  drops `'` and `:`. Paths inside the graph stay relative to the project dir; only `preview.font`
  is absolute. `crates/videoforge-preview/tests/ffmpeg_real.rs` checks this against a real FFmpeg
  when one is on `PATH` (or `VIDEOFORGE_FFMPEG`) and skips otherwise.
- **The character/Live2D feature is opt-in and additive.** `Config::character_manifest` /
  `SpeakerConfig::character_id` are both `Option`; a workspace that sets neither runs through
  every stage of `generate()` exactly as before (byte-for-byte — no new stage even runs). A
  character's named VOICEVOX voice is resolved to a numeric `speaker_id` once, in
  `core::character::resolve_character_voices`, *before* validation — `VoiceParams`, `TtsCache`,
  and `TtsEngine` never see a name, only the resolved id, same as any hard-coded config.
- **YMM4 export is a template patch, not a serializer.** The template `.ymmp` is an opaque
  `serde_json::Value`; the exporter clones items whose `Remark` starts with `VF_PROTO_` and writes
  only `Text` / `Frame` / `Length` / `FilePath` / `Remark` / `IsHidden`. Everything else is
  preserved byte-for-byte in key order. The template's fps wins over the project's (with a warning).

## Known gaps

- `fixtures/templates/ymm4/default.ymmp` is synthetic. The Phase 0 spike — a real YMM4 template
  exported and reopened on Windows — has not been done; it is the largest technical risk. The
  procedure is written up in `docs/testing/ymm4-manual-e2e.md` with an empty results table;
  whoever runs it fills that in.
- FFmpeg preview is covered only at the command-builder level; no real render is tested. The
  filter graph needs an FFmpeg built with `drawtext` (libfreetype), and `doctor` reports FFmpeg as
  `ok` without checking for it — on such a build `generate` dies with `preview_render_failed`
  ("No such filter: 'drawtext'") after the TTS work is already done.
- Visual clip compositing (P1-1/P1-2/P1-5) is also command-builder-tested only: the `"fade"`
  intent's `fade=...:alpha=1` and a `Video` layer's `trim`+`setpts` offsetting follow well-known
  FFmpeg recipes, but the `"slide"`/`"zoom"` Ken Burns crop expressions (`eval=frame` referencing
  `t`, `clip(...)`) have not been run against a real FFmpeg from this repo — do that before relying
  on them. `Video::volume`/`muted` are parsed and stored but not yet mixed into the audio graph
  (P1-4 is where BGM/SE mixing lands; a video clip's own audio track is the same follow-up). A
  `Video` clip and a mismatched `png_lipsync` character overlay could both legally claim the same
  screen position — nothing detects that; it is on the author, same as any other clip authored by
  hand.
- The CI workflow is parked in `docs/ci/` because the authoring session could not create files
  under `.github/workflows/`. Enabling it is a `git mv` (see `docs/ci/README.md`).
- The Tauri GUI (`apps/desktop`) builds and its command layer is unit-tested, but it has not been
  launched on a real Windows or macOS desktop; the checklist is in `apps/desktop/README.md`.
- The Live2D half of the character pipeline (`docs/character-system.md`) stops at a
  `character_performance` timeline track and a deterministic lip-sync file — no Live2D frame is
  ever rendered, and there is no GUI wiring (CLI-only). The renderer direction is decided but not
  built; see `docs/live2d-renderer-decision.md`. A `model.type: png_lipsync` character, by
  contrast, **is** composited into `preview.mp4` (P0-1: 3-state closed/half/open PNG renderer,
  `videoforge_preview::command::build_character_overlays`) — see "3-state PNG rendering" in
  `docs/character-system.md`. Neither path has GUI wiring yet. The manual procedure
  (`docs/testing/character-manual-e2e.md`) has an empty results table — whoever runs it with a real
  VOICEVOX character (and, for the PNG path, real closed/half/open artwork) fills that in.
- Incremental build (P0-4) covers exactly one stage (the FFmpeg preview render, see "Incremental
  build" above). There is no general per-stage dependency graph, no `videoforge generate --from`/
  `--only`, and no cross-process "resume a failed generate" beyond what the existing tmp-dir/lock
  machinery already gives (a crashed generate never corrupts `generated/<slug>/`, but it does not
  resume mid-pipeline either — the next `generate` starts over from validation).
- The asset registry (P0-2, `core::assets`) is per-generate only: no cross-project catalog, no
  dedup index, no UI. `videoforge assets` reads back one `asset-registry.json` at a time.
