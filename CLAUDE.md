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

# render presets (P1-6) and fast preview (P1-7)
cargo run -p videoforge-cli -- generate scripts/sample.md --preset youtube-1080p
cargo run -p videoforge-cli -- generate scripts/sample.md --preset preview-low --range-ms 0:15000
cargo run -p videoforge-cli -- preview fast generated/sample/project.vfp.json --preset preview-low --range-ms 0:15000
```

CI (`docs/ci/github-actions-ci.yml`, not yet under `.github/workflows/` — see "Known gaps") runs
fmt → clippy → test → release build → an offline smoke run of
`init → validate → generate → bundle → export`, with FFmpeg installed so the real-FFmpeg tests
execute, plus a separate desktop job (pnpm build + cargo check/test of the Tauri crate).

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
Pan/zoom is `crop=...:eval=frame` referencing `t`, not the `zoompan` filter (see the module doc on
`RenderPlan::ken_burns_filter` for why); the `"zoom"` half was rendered through a real FFmpeg and
visually confirmed in `docs/testing/p1-dogfood-e2e.md`, `"slide"` has not been (see "Known gaps"). An
overlapping crossfade between adjacent clips is not implemented (each clip's own `"fade"` only ramps
its own alpha at its own boundaries, not a coordinated dissolve with its neighbor) — "don't build a
complex editor" per the P1-5 brief. **Every `overlay=x=<expr>:y=<expr>` value must be wrapped in
`'...'`** (`overlay_position_exprs`/`visual_position_exprs`) — FFmpeg's filter-option tokenizer splits
an option's value on a bare `,`, so an unquoted `min(max(0,...` breaks filtergraph parsing the moment
a project has more than a background and captions. This shipped broken from P0-1 through P1-2 because
every test asserted on the generated *string*, never fed it to a real FFmpeg; found and fixed via
dogfooding (`docs/testing/p1-dogfood-e2e.md`), with a regression test
(`overlay_position_expressions_parse_in_a_real_filtergraph` in
`crates/videoforge-preview/tests/ffmpeg_real.rs`) that renders a real character overlay through FFmpeg.

### Subtitle engine (P1-3)

Caption styling is one config struct, `config::SubtitleConfig` (`videoforge.yaml`'s
`preview.subtitle`), threaded into the renderer as `PreviewRequest::subtitle` and stored on
`RenderPlan` — there is no separate "subtitle engine" module, just parameters the existing caption
`drawtext` chain (unchanged since P0) now reads instead of hard-coding. `position` (`bottom`/`top`)
picks which frame edge captions anchor to; `margin_fraction` replaced the `0.20`-of-height constant
that used to be baked into `caption_safe_area_px`/the `y=` expressions, so both the caption text and
the character-overlay clamp (`overlay_position_exprs`) move together — a `top`-anchored caption
flips the character clamp to a *minimum* `y` (stay below the top safe area) instead of a maximum
(stay above the bottom one), so a character can never be placed under captions on either edge.
`font_color`/`outline_color`/`outline_width`/`background`/`background_color`/`font_scale` map
directly onto `drawtext`'s `fontcolor`/`bordercolor`/`borderw`/optional `box=1:boxcolor=.../
boxborderw=10`/font-size multiplier; `font_scale` also shrinks `max_chars_per_line`'s wrap width so
long lines still fit. Per-speaker color (`SpeakerConfig::caption_color`) is resolved once at
`DialogueInput` construction time in `core::generate` — the same "resolve early, bake into the IR"
pattern P0's character voice resolution established — landing on `CaptionClip::color`; a caption
with no override falls back to `subtitle.font_color` in `RenderPlan::build`. The speaker-name label
above the caption text keeps its own always-on box (a "chip"), independent of
`subtitle.background`, which only toggles a box behind the caption text itself.
`buildcache::preview_fingerprint` hashes `SubtitleConfig`'s `Debug` output too, so a subtitle-only
config edit invalidates the incremental-build cache (P0-4) the same as a background-color change.

### Audio engine (P1-4)

`RenderPlan::audio_filter` mixes independent groups of FFmpeg audio inputs — dialogue, a `Video`
visual layer's own embedded audio (unless `muted`), BGM (`BgmPlan`), and sound effects
(`SoundEffectPlan`) — with `amix=inputs=N:duration=longest:normalize=0` rather than per-pair
`amerge`, so adding a group never restructures the graph. A `Video` layer's own audio reads its
`:a` stream from the *same* FFmpeg input `visual_layer_filters` already gives its `:v` stream
(`RenderPlan::visual_layer_input_index`, shared by both so they can never drift) — no extra input
needed. BGM/SE inputs, by contrast, are appended at the *end* of the FFmpeg input list, after
character sprites, via `RenderPlan::bgm_and_se_inputs`/`bgm_input_start`/`se_input_start` —
deliberately, so the existing `:v`-stream index formulas for visual layers and character overlays
(already covered by tests) never have to be recomputed when BGM/SE are added or removed.

Each BGM clip's chain is, in order: an `atrim` bounding a looping (`-stream_loop -1`) source back
down to its own `duration_ms` (so the otherwise-infinite input still terminates) — or, for a
non-looping clip with `trim_start_ms > 0`, a plain `atrim=start=...` — then **`asetpts=PTS-STARTPTS`**
(load-bearing: `atrim` never rewrites timestamps on its own, confirmed against a real FFmpeg; without
this reset, `adelay` below stacks its offset on top of the untouched trim-point PTS instead of
replacing it, landing the clip later than its configured `start_ms` — a real bug this repo shipped
and fixed, see "Known gaps"), resample/format, `adelay` to its timeline position, optional
`dynaudnorm` (the `normalize` flag), its base `volume`, optional `afade` in/out, and — the headline
feature — **ducking**: a single `volume=eval=frame:volume='if(<union-of-between(t,...)-windows>,
DUCK,1)'` expression that reuses the exact enable-window-union pattern P0-1 established for character
mouth overlays, applied here to an audio `volume` filter instead of a video `overlay`'s `enable=`.
`BGM_DUCK_VOLUME` (0.35) is the one constant governing how far BGM drops under dialogue; there is no
per-clip override yet. A `Video` layer's own audio chain (`video_layer_audio_chain`) follows the same
`atrim`+`asetpts`+`adelay` shape, bounded to the same `[trim_start_ms, trim_start_ms + duration_ms)`
window `video_trim_filter` uses for its picture — confirmed with a real FFmpeg render (an audible
tone embedded in the video source, verified via spectrogram to start and stop exactly on the clip's
timeline bounds).

### Render presets and fast preview (P1-6/P1-7)

`core::preset` is a fixed catalog (`PRESETS: &[RenderPreset]`) of three named bundles —
`youtube-1080p`, `youtube-short`, `preview-low` — each pairing a resolution/fps with
`preview::EncodeSettings` (codec/encoder-speed/CRF/audio-bitrate). A preset is deliberately *not* a
free-form config surface (no arbitrary codec strings in `videoforge.yaml`): `videoforge generate
--preset <name>` and `videoforge preview fast --preset <name>` both just look a name up. Applying one
overrides `VideoProject::video` (width/height/fps) *after* `videoforge_timeline::build` — safe only
because every clip's `Transform` position is a normalized `0.0..=1.0` fraction, never a pixel value,
so changing the render resolution never touches placement math; `project.vfp.json` then correctly
records the resolution actually rendered. `EncodeSettings` isn't part of the project IR at all (CRF
etc. aren't scheduling data), so it travels through `PreviewRequest::encode` the same way
`background_color`/`subtitle` do, and `buildcache::preview_fingerprint` hashes it too — a preset
switch at the same resolution must still invalidate the P0-4 cache.

Fast preview (P1-7) has two independent parts. (1) A `generate --range-ms START:END` renders the
whole pipeline as usual but adds an *output-side* `-ss`/`-t` trim in `build_args` — the filter graph
is still built for the full timeline, so every absolute-timeline expression (fades, Ken Burns,
ducking windows) keeps meaning what it always has; FFmpeg just decodes/filters everything and drops
what falls outside the window. A ranged generate writes to `preview.fast.mp4`
(`generate::PREVIEW_FAST_FILE`), never `preview.mp4`, and skips the P0-4 cache entirely — it is a
disposable, exploratory render, not "the" cached preview. (2) `videoforge preview fast
<project.vfp.json>` (`core::fastpreview::render`) skips parsing/validation/TTS/timeline scheduling
altogether and re-renders straight from an *already generated* `project.vfp.json` — the genuinely
"fast" half, since TTS re-synthesis (the slow step `generate` would otherwise repeat, cache hits
aside) never runs at all. It re-resolves `png_lipsync` character sprites from the character manifest
(cheap: no audio, no lip-sync analysis — the project's `character_performance` clips already carry
their lip-sync file paths from the original `generate`). A "selected scene" is just a `[start_ms,
end_ms)` range by another name; turning a scene/dialogue index into those bounds (e.g. by reading a
cue's timing out of `captions.srt`) is left to the caller — this module only needs the resolved
range.

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
- FFmpeg detection now checks the filters and encoders required by the preview graph (including
  `drawtext`, `overlay`, `amix`, `libx264`, and `aac`) before `doctor` reports it available. A build
  without libfreetype therefore produces an FFmpeg warning before TTS work rather than a late
  `preview_render_failed`. This remains an environment-level check: `doctor` still has no script
  argument, so project-specific asset enumeration and duration/size estimation belong to #47's
  next preflight slice.
- Visual clip compositing (P1-1/P1-2/P1-5): two real-FFmpeg dogfood rounds
  (`docs/testing/p1-dogfood-e2e.md`) have now exercised background + two `png_lipsync`
  character overlays + an `@image` with both `intent=zoom` and `intent=slide` + a `@video`
  layer with `trim_start_ms`, and visually confirmed all of it composites correctly. This is
  also what found and fixed three real bugs: the `overlay=x=/y=` quoting bug (see above);
  `ken_burns_filter` passing `eval=frame` to the `crop` filter, which has no such option at
  all in this FFmpeg build (confirmed via `ffmpeg -h filter=crop` — none of `w`/`h`/`x`/`y`
  need one, they already re-evaluate every frame when their expression isn't a constant) and
  aborted the whole render with "Option not found" — this is why `"slide"` specifically had
  never been verified before; and `bgm_chain`'s `atrim` never resetting PTS before `adelay`
  when `trim_start_ms > 0`, which silently shifted a trimmed BGM clip's actual start later
  than its configured `start_ms` (confirmed with a minimal real-FFmpeg repro). `Video::volume`/
  `muted` are now mixed into the audio graph too (`video_layer_audio_chain`, the audio
  counterpart to `video_trim_filter`'s picture handling) — confirmed with a real FFmpeg
  render carrying an audible tone in its own video track, verified landing at the exact
  right timeline position via spectrogram (`showspectrumpic`), not just "the render didn't
  error." A `Video` clip and a mismatched `png_lipsync` character overlay could both legally
  claim the same screen position — nothing detects
  that; it is on the author, same as any other clip authored by hand. BGM ducking's
  `BGM_DUCK_VOLUME` is a single global constant, not a per-clip or per-config value; the
  dogfood video's BGM chain (loop/fade-in/fade-out/normalize/ducking) rendered without
  error but was not byte-level verified against an independent reference (only that the
  render succeeded and the audio track is valid AAC).
- The subtitle engine (P1-3): the dogfood run rendered `bottom`-positioned captions with a
  per-speaker `caption_color`, `subtitle.background`, and `font_scale` through a real
  FFmpeg with a real CJK font (`ipag.ttf`) and confirmed them legible in extracted frames.
  `subtitle.position: top` in particular has still not been rendered through a real
  FFmpeg. Line wrapping (`wrap_text`) is still a naive fixed-character-count hard-wrap
  (CJK-appropriate, since every glyph is ~1em; a long unbroken Latin word is not treated
  specially). There is no automatic shrink-to-fit for a caption that is long even after
  wrapping — an author who sets an extreme `font_scale` or writes a very long line can
  still push text off the safe area horizontally (only the vertical safe-area/character-
  overlap axis is structurally enforced). Per-speaker styling (P1-3's "speaker-style")
  covers only caption text color (`SpeakerConfig::caption_color`); font/size/outline/
  background remain global (`preview.subtitle`), not per-speaker.
- Render presets (P1-6) are a fixed catalog of three names — no user-defined preset, and no
  way to override a single field of a preset (e.g. "youtube-1080p but crf 20") without
  picking a name and accepting its whole bundle. The dogfood run rendered `youtube-1080p`
  (`generate --preset`) and `preview-low` (`preview fast --preset`) through a real FFmpeg
  and confirmed via `ffprobe` that both landed the expected resolution — the actual
  perceptual bitrate/quality tradeoff of each preset's CRF/encoder-speed choice has not
  been evaluated by eye or by any metric, only that the encode succeeds. Fast preview's
  (P1-7) `-ss`/`-t` output-side trim was also confirmed for real: `preview fast
  --range-ms 50000:95000` produced an exactly-45.000s file (`ffprobe`), landing on the
  requested window, in 27.5s versus 4m25s for the full render — and a frame pulled from it
  matched the corresponding point in the full render, confirming the trim does not
  desynchronize the filter graph's absolute-timeline expressions from the encoded window.
  `videoforge preview fast` still has no manifest entry of its own (it does not write or
  update `manifest.json` at all) and no "selected scene" CLI convenience — a scene has to
  be turned into a millisecond range by the caller (e.g. from `captions.srt`) before it
  reaches `--range-ms`.
- CI is authored but still parked in `docs/ci/` (`github-actions-ci.yml`, `micro-wasm.yml`), not
  live under `.github/workflows/`. A session this round attempted to enable it via `git mv` and
  also fixed a test (`concurrent_generates_of_one_slug_do_not_race`, see the generate.rs history)
  that would have made a freshly-enabled CI flaky — but the push was rejected by GitHub
  ("refusing to allow a GitHub App to create or update workflow `.github/workflows/ci.yml` without
  `workflows` permission"): this session's GitHub App token has no `workflows` scope, and per this
  environment's own policy that kind of denial is reported rather than routed around. Enabling it
  is still exactly the `git mv docs/ci/github-actions-ci.yml .github/workflows/ci.yml && git mv
  docs/ci/micro-wasm.yml .github/workflows/micro-wasm.yml` described in `docs/ci/README.md`,
  done by whoever/whatever has that permission (a human, or a token with `workflows` scope). Once
  it is enabled it has never actually run on GitHub's infrastructure either way — every job's
  individual commands are what this repo's own `cargo test --workspace`/`clippy`/`fmt --check`
  already run locally, verified repeatedly across sessions, but "watched go green on
  Windows/macOS runners" remains a real gap until someone does both steps.
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
