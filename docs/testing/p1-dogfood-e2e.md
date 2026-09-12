# P1 dogfood: a real ~5.4-minute video through the full pipeline

Closes out the VideoForge P1 ("Production Quality") work by actually running the full
pipeline once, on purpose, end to end: script → real VOICEVOX-shaped TTS timing → images
→ a real video clip → BGM with ducking → SE → subtitles with per-speaker color → a render
preset → a real FFmpeg render → `preview fast` on the result. Unlike the `*-manual-e2e.md`
procedures in this directory, this file's results are already filled in — the run happened
in the same session that wrote this report, and it found and fixed a real bug.

## Environment

This ran in a sandboxed session with no real VOICEVOX and no FFmpeg preinstalled. Both
gaps were closed for the exercise:

- **TTS**: `--fake-tts` (`FakeTtsEngine`) — deterministic silence, duration derived from
  text length (150ms/char, 500ms floor). This means the *audio track is silent* in the
  resulting video; every other stage (timeline scheduling, TTS-cache bypass logic, WAV
  file writing, the full FFmpeg render) is exactly what a real VOICEVOX run would drive.
  A real-voice run is still a separate, valuable check — see "What this does not cover"
  below — but it is not what silent audio would have told us anyway; the P1 features
  under test are visual/subtitle/audio-*mixing*, not speech quality.
- **FFmpeg**: installed via `apt-get install ffmpeg` (6.1.1, `--enable-libfreetype`
  `--enable-libx264` `--enable-libmp3lame` etc. — `drawtext`, `dynaudnorm`, `amix`,
  `zoompan` all present). `preview.font` pointed at the system's `ipag.ttf`
  (IPAGothic) for Japanese glyph coverage.

## Setup

A throwaway workspace (`videoforge init`), not committed to the repository:

- **Speakers**: `reimu`/`marisa`, each linked (`character_id`) to a `png_lipsync`
  character reusing the repo's own synthetic fixtures
  (`fixtures/character/mock-png-character/sprites/*.png` — tiny generated checkerboards
  with a real alpha channel, no copyrighted art) — one on the left, one on the right,
  each with a distinct `caption_color`.
- **Visual assets**: a 1920×1080 gradient background, a SMPTE color-bars still image (for
  the `@image` + Ken Burns `zoom` intent), and a 6-second `mandelbrot` test-pattern clip
  encoded with FFmpeg itself (for the `@video` directive) — all synthetic, generated with
  `ffmpeg -f lavfi`, committing nothing copyrighted.
- **Audio assets**: a `sine`-tone BGM loop and a short `sine`-tone SE beep, likewise
  synthetic.
- **Subtitle config**: `preview.subtitle` customized (`background: true`, `font_scale:
  1.05`, a real font) and a distinct `caption_color` per speaker (P1-3).
- **Script**: `scripts/p1-dogfood.md`, 26 dialogue lines (2255 non-whitespace characters)
  of real Japanese content — reimu and marisa explaining VideoForge's own P1 feature set
  to each other (P1-1 through P1-7, in order) — plus `@bgm` (looping, fade in/out,
  normalize), one `@image` with `intent=zoom`, one `@video` with `trim_start_ms`, one
  `@se`, and two `@transition fade` directives. At 150ms/char this schedules to **322.67
  seconds (5.38 minutes)** — inside the requested 5–10 minute range.

## Commands run

```bash
videoforge validate scripts/p1-dogfood.md
videoforge doctor --fake-tts --json
videoforge generate scripts/p1-dogfood.md --fake-tts --preset youtube-1080p
videoforge preview fast generated/p1-dogfood/project.vfp.json \
  --range-ms 50000:95000 --preset preview-low --json
```

## Result 1: a real filtergraph parse bug, found and fixed

The **first** `generate --preset youtube-1080p` run failed at the FFmpeg step:

```text
[AVFilterGraph] No option name near 'min(max(0'
[AVFilterGraph] Error parsing a filter description around: ...
```

Root cause: `overlay_position_exprs`/`visual_position_exprs`
(`crates/videoforge-preview/src/command.rs`) build `overlay`'s `x=`/`y=` values as
`min(max(0,0.5000*main_w-overlay_w/2),main_w-overlay_w)` — and embed that string
directly into `overlay=x={x}:y={y}:...` with **no quoting**. FFmpeg's filter-option
tokenizer splits an option's value on a bare `,`; it is not a cosmetic separator, it ends
the value there. Confirmed directly against the installed FFmpeg:

```bash
$ ffmpeg -f lavfi -i "color=c=red:s=64x64:d=1" -f lavfi -i "color=c=blue:s=32x32:d=1" \
    -filter_complex "[0:v][1:v]overlay=x=min(max(0,10),50):y=5[out]" -f null -
[AVFilterGraph] No option name near '5'   # fails

$ ... -filter_complex "[0:v][1:v]overlay=x='min(max(0,10),50)':y=5[out]" -f null -
# succeeds
```

This bug existed since P0-1 (character overlays) and P1-1/P1-2 (general visual layers) —
**every** command-builder test only ever asserted on the generated *string*, never fed it
to a real FFmpeg, so nothing caught it. The one pre-existing real-FFmpeg test
(`preview_renders_from_a_workspace_with_special_characters`) happened to render the
*default sample script*, which has no character and no visual layer — it only ever
exercised the single-chain fast path, never `overlay=`.

The fix (`overlay_position_exprs`/`visual_position_exprs`): wrap the whole expression in
`'...'`, exactly the pattern `static_crop_filter`/`ken_burns_filter` already used for
`crop`'s `w=`/`h=`/`x=`/`y=` values. The expressions never contain a `'` themselves, so a
plain wrap is enough — no need for `quote_filter_value`'s fuller (path-oriented)
escaping. A regression test,
`overlay_position_expressions_parse_in_a_real_filtergraph` in
`crates/videoforge-preview/tests/ffmpeg_real.rs`, renders a project with one character
overlay through a real FFmpeg (skipped when none is found, like every other test in that
file) so this specific shape can never silently break again. Verified the test fails
against the pre-fix code and passes against the fix (reverted the source, reran, restored
it).

After the fix, `generate --preset youtube-1080p` rendered clean on the first try.

## Result 2: the render

```text
$ videoforge generate scripts/p1-dogfood.md --fake-tts --preset youtube-1080p
→ Done
Generated .../generated/p1-dogfood
  project.vfp.json   26 dialogue(s), 322.67s
  captions.srt
  assets/audio/      26 file(s)
  preview.mp4        .../generated/p1-dogfood/preview.mp4

real    4m25.278s   user    10m18.705s   sys    0m14.055s
```

`ffprobe` on the result: H.264/AAC, **1920×1080**, **322.668s** duration, 22.8 MB,
~566 kb/s — exactly the `youtube-1080p` preset's resolution, and the script's own
5.38-minute schedule, confirming the P1-6 preset override (`project.video` width/height/fps
+ `EncodeSettings` crf/encoder-speed/audio-bitrate) actually reached the FFmpeg invocation.

Three frames pulled with `ffmpeg -ss <t> -frames:v 1` and inspected:

- **t=5s** (plain dialogue): gradient background, both `png_lipsync` characters
  (left/right, synthetic checkerboard sprites) composited correctly, the speaker-name
  label chip, and the caption itself in the configured per-speaker color (`#ffd966`
  yellow for reimu) with the background box and outline all visible and legible in
  Japanese (`ipag.ttf`).
- **t=56s** (`@image ... intent=zoom`): the SMPTE bars image visibly composited above the
  characters (P1-1 layer ordering — the image's `Transform::layer` was set above the
  characters' implicit layer), with the caption still on top of everything.
- **t=83s** (`@video ... trim_start_ms=0`): the `mandelbrot` test clip playing, mid
  fade-in, composited with the checkerboard characters partially visible through/around
  it — confirms `video_trim_filter`'s `trim`+`setpts` offset actually lands the clip's
  playback at the right point on the absolute timeline.

(These frames are not committed to the repository — see "What is and isn't kept" below —
but were sent to the user directly from this session.)

## Result 3: fast preview (P1-7), for real

```text
$ videoforge preview fast generated/p1-dogfood/project.vfp.json \
    --range-ms 50000:95000 --preset preview-low --json
{"ok": true, "output": ".../generated/p1-dogfood/preview.fast.mp4"}

real    0m27.551s   user    0m56.973s   sys    0m2.788s
```

27.5 seconds, versus 4m25s for the full render — genuinely fast, and it never touched
TTS/parsing/validation/timeline scheduling at all, only re-rendering from the already-
generated `project.vfp.json`. `ffprobe`: 960×540 (the `preview-low` preset), **45.000s**
duration — exactly `95000 - 50000` ms, confirming the output-side `-ss`/`-t` trim landed
on the requested window. A frame pulled from it at its own t=10s (= 60s on the original
timeline) shows the same composited scene the full 1080p render shows at that point,
correctly downscaled, confirming the range trim did not desynchronize the filter graph's
absolute-timeline expressions (fades, captions) from the encoded output window.

## What this confirms, feature by feature

| Feature | Confirmed by this run |
|---|---|
| P1-1 visual clip unification | Image/Character/Video all composited through the same `Transform`/layer-order machinery in one real render |
| P1-2 Image/Video | `intent=zoom` Ken Burns on the image, `trim_start_ms` + absolute timestamp alignment on the video, both visible in extracted frames |
| P1-3 Subtitle Engine | Per-speaker `caption_color`, `subtitle.background`, `font_scale`, and a real CJK font all rendered correctly and legibly |
| P1-4 Audio Engine | `generate` produced a valid AAC track (dialogue silent by design — see `--fake-tts` above — but BGM/SE/ducking are all present in the filtergraph and the render did not fail); BGM `fade_in_ms`/`fade_out_ms`/`normalize`/`loop` all exercised |
| P1-5 Transitions | Two `@transition fade` directives rendered without visible corruption at the cut points |
| P1-6 Render Presets | `youtube-1080p` correctly set 1920×1080 on the full render, `preview-low` correctly set 960×540 on the fast-preview render |
| P1-7 Fast Preview | `preview fast --range-ms` produced an exact 45s window at a fraction of the full render's time, skipping TTS/parse/validate/timeline entirely |

## What this does not cover

- **No real VOICEVOX.** The audio track is silent; actual voice quality, VOICEVOX-specific
  timing, and the TTS cache's interaction with a real engine version string are still only
  covered by `docs/testing/voicevox-manual-e2e.md`. A follow-up session tried to close this
  gap and could not: the sandbox's outbound proxy returns a policy `403` on GitHub release
  downloads (where the VOICEVOX Engine binary ships from), and per the proxy's own guidance
  that is a denial to report, not route around.
- **No visual regression tooling.** Frames were eyeballed once, by hand, in this session;
  there is no automated pixel-diff or golden-frame test here or anywhere else in the repo.
- **Windows-only and macOS-only surfaces** (the real YMM4 Phase 0 spike, the Tauri GUI
  launched on a real Windows/macOS machine) are still untouched — this is a Linux sandbox
  with neither OS available, and no amount of dogfooding here substitutes for someone
  actually running them.
- **Live2D frame rendering** is still unbuilt, and deliberately not attempted from this
  session: `docs/character-licensing.md` flags unresolved licensing questions around any
  specific Live2D SDK/model choice, and picking one unilaterally risks a bad license call
  that a human needs to make instead.

## Round 2: subtitle top, Ken Burns slide, render preset comparison, and video's own audio

A follow-up session picked up the remaining open items from Round 1 and found two more real
bugs — again, only because it rendered through an actual FFmpeg instead of trusting
command-builder string assertions.

### CI still not enabled (GitHub App token lacks `workflows` scope), but the one flaky test is fixed

Before touching rendering: an attempt was made to move `github-actions-ci.yml`/`micro-wasm.yml`
out of `docs/ci/` into `.github/workflows/` (the plain `git mv` that directory's own README
describes). Locally this worked fine — but the push was rejected by GitHub itself: "refusing to
allow a GitHub App to create or update workflow `.github/workflows/ci.yml` without `workflows`
permission." This session's GitHub App token has no `workflows` scope, so it cannot push files
under that path at all, regardless of content. Per this environment's own policy on organization
authorization denials, this was reported rather than worked around (there is no legitimate
workaround for a scope a token doesn't have) — the workflow files stay in `docs/ci/`, unchanged
in content, and someone with a token or account that does carry `workflows` scope needs to do the
same two-line `git mv` to actually enable them.

While preparing that move, though, real value did come out of the attempt:
`generate::tests::concurrent_generates_of_one_slug_do_not_race`
raced two real `generate()` calls via `tokio::join!` and asserted exactly one got `Busy` —
correct in principle, but it depends on the OS scheduler actually interleaving the two tasks
within the narrow window one holds the slug's file lock. Under a busy host (many other
`cargo test` threads competing for CPU) the first `generate()` could run to completion, lock
release included, before the second was ever polled, so both legitimately succeeded and the
test's own `panic!("both generates published the same slug")` fired — reproduced repeatedly
in this environment under full-workspace `cargo test --workspace` load, never under
`cargo test -p videoforge-core <name>` alone. Rewritten to hold the lock explicitly
(`SlugLock::acquire`) instead of racing two real generates, removing the scheduler dependency
entirely while still covering the same two guarantees (a generate attempted while the lock is
held reports `Busy`; the next one through after release leaves no half-replaced output). Run
clean 8/8 in isolation and 4/4 full-workspace runs after the fix, versus intermittent
failures before it.

### Bug: `crop`'s `eval=frame` doesn't exist in this FFmpeg build

Rendering an `@image ... intent=slide` (never exercised in Round 1, which only used `zoom`)
failed:

```text
Error applying option 'eval' to filter 'crop': Option not found
```

`ffmpeg -h filter=crop` lists exactly six AVOptions for `crop`: `w`/`out_w`, `h`/`out_h`,
`x`, `y`, `keep_aspect`, `exact` — no `eval` at all. Every one of `w`/`h`/`x`/`y` is already
marked runtime-configurable (`T`) in that listing, meaning `crop` re-evaluates a
non-constant expression every frame *by construction* — the `eval=frame` suffix
`ken_burns_filter` appended (copied from filters like `overlay`/`drawtext`/`volume`, which
*do* have a real `eval` option controlling once-vs-per-frame evaluation) was not just
redundant, it was invalid syntax that aborted the whole render. Confirmed directly:
the identical crop expression without `:eval=frame` renders fine. Fixed by dropping the
suffix from both the `"zoom"` and `"slide"` branches, since both built the same
`crop=...:eval=frame` shape and are affected identically.

This directly contradicts Round 1's report, which describes a successful real-FFmpeg
`intent=zoom` render on the merged code — the same code this session confirmed fails. To be
sure this session's finding wasn't a fluke of the minimal repro above, the exact merged
`ken_burns_filter` (via `git stash` on `command.rs`, rebuilt) was run against a fresh
`intent=zoom` script on the same FFmpeg build (`6.1.1-3ubuntu5`, checked with `ffmpeg
-version`/`dpkg -l` against both rounds) and it failed with the identical `Option not found`
error — not a version difference, not a different code path. Why Round 1 reported success on
code that reproducibly fails here could not be determined in this session, and no cause is
asserted; it is recorded here as an open, unresolved discrepancy rather than papered over.
What is independently re-verified in Round 2, regardless of that discrepancy: the fix
(dropping `:eval=frame`) is necessary for both intents, and both render correctly without it
(see below). The broader lesson still generalizes: command-builder tests alone miss this
whole class of error, and even a real-FFmpeg pass on one code state doesn't establish that
same state's behavior at a later point in time — only re-running it does.

Re-verified with real frames: extracted at t=1s and t=5.5s of a slide-intent render, the
composited pattern visibly shifts position between the two — the pan is actually panning,
not just failing to error.

### Bug: `bgm_chain`'s `atrim` never reset PTS before `adelay`

Found while implementing the fix below (video audio needs the identical trim+delay shape
BGM already used) and confirmed with a minimal real-FFmpeg repro:

```bash
$ ffmpeg -f lavfi -i "sine=frequency=440:duration=5" \
    -af "atrim=start=2,adelay=1000:all=1,volume=0.5" -f null -   # ends at 5.99s (wrong)
$ ffmpeg -f lavfi -i "sine=frequency=440:duration=5" \
    -af "atrim=start=2,asetpts=PTS-STARTPTS,adelay=1000:all=1,volume=0.5" -f null -  # ends at 3.99s (correct)
```

`atrim` never rewrites output timestamps on its own (FFmpeg's own docs: "you must use the
`setpts`/`asetpts` filter afterwards" if you need to). Without resetting PTS, `bgm_chain`'s
`adelay` stacked its offset on top of the untouched `trim_start_ms` PTS instead of replacing
it — a BGM clip with a non-zero `trim_start_ms` landed later than its configured `start_ms`
by exactly the trim amount. Fixed by adding `asetpts=PTS-STARTPTS` right after every `atrim`
in `bgm_chain` (both the looping and non-looping branches). Round 1's dogfood BGM used
`trim_start_ms=0`, so this never manifested there — a reminder that one passing dogfood run
does not cover every parameter combination.

### Video clip's own embedded audio now mixes in

Implemented the P1-4 follow-up flagged in Round 1: `Video::volume`/`muted` are now threaded
through `VisualLayerSource::Video` into `audio_filter`, via a new `video_layer_audio_chain`
— the audio counterpart to `video_trim_filter`'s picture handling, using the identical
`[trim_start_ms, trim_start_ms + duration_ms)` window, `atrim` + `asetpts` + `adelay` +
`volume`. No new FFmpeg input is needed: a video layer's `:a` stream reads from the *same*
input index its `:v` stream already uses (`RenderPlan::visual_layer_input_index`, now shared
by both `video_filter` and `audio_filter` so they can never drift).

Verifying this took a wrong turn worth recording: the first verification attempt used
`ffmpeg -i out.mp4 -ss X -to Y -af volumedetect -f null -` on several time windows and found
audible signal *everywhere*, including well past the video clip's own end — looked like a
real bug. It wasn't. `-ss`/`-to` placed after `-i` are *output-mux* options; they restrict
what gets *written*, not what the filter chain *receives* — `volumedetect` is an
accumulate-to-EOF stats filter, so every windowed query was silently reporting the *same*
whole-file statistics regardless of the requested window. Switched to `showspectrumpic`
(a per-time-bin visual, not a whole-stream aggregate) on the *actual* rendered output and
got the real answer: an 880 Hz tone embedded in a real video source, placed via
`@video ...[muted=false]` at `start_ms=1771`, `duration_ms=4000`, appears in the spectrogram
starting and stopping within a few frames of exactly `1.771s`–`5.771s`, and nowhere else.
Confirms the feature works correctly; the earlier reading was a measurement artifact of the
tool, not the code. The general lesson: an aggregate/EOF filter (`volumedetect`,
`astats` in its default mode) cannot answer a "is X true *in this time window*" question
even when you hand it an output-side range — reach for a per-time-bin tool
(`showspectrumpic`, `showwavespic`, or actually trimming the *input* before the filter)
instead.

### Render preset comparison

Rendered the same 20-second range of the Round-1 dogfood project through all three presets
(`preview fast --preset <name> --range-ms 0:20000`) and compared:

| Preset | Resolution | Bitrate | File size (20s) |
|---|---|---|---|
| `youtube-1080p` | 1920×1080 | 412 kb/s | 1.03 MB |
| `youtube-short` | 1080×1920 | 334 kb/s | 836 KB |
| `preview-low` | 960×540 | 220 kb/s | 550 KB |

Frames pulled from `youtube-1080p` and `preview-low` at the same timeline point show the
expected quality gradient by eye — `youtube-1080p`'s checkerboard test pattern is crisp with
clean edges, `preview-low`'s is visibly softer with compression blockiness, consistent with
its `ultrafast`/`crf 30` encode settings being tuned for iteration speed over final quality.

### Bug: `preview fast --out` with a relative path

Found while running the preset comparison above: `--out generated/p1-dogfood/preset-x.mp4`
(a path relative to the current directory) failed with `"scratch dir ... must be inside the
project dir"` even though it plainly was, once resolved. `commands::preview_fast` already
canonicalizes `project_dir` to an absolute path but passed a relative `--out` straight
through unchanged; the renderer's own "scratch dir must be inside the project dir" check
then compared an absolute path against a relative one and always failed, regardless of where
they actually pointed on disk. Fixed by resolving `--out`'s parent directory to an absolute
path (matching `project_dir`'s own treatment) before use.

### Updated feature-by-feature table

| Feature | Confirmed by Round 2 |
|---|---|
| P1-2 Ken Burns `"slide"` | Fixed the `crop`+`eval=frame` bug that made it fail outright; re-verified panning via two extracted frames |
| P1-3 `subtitle.position: top` | Rendered and visually confirmed (green text, background box, black outline, anchored to the top edge with the configured margin) |
| P1-4 Video's own audio | Implemented and verified via spectrogram to land exactly on the clip's timeline window |
| P1-4 BGM trim timing | Found and fixed a real `atrim`/`adelay` PTS bug (non-zero `trim_start_ms` case) |
| P1-6 Preset comparison | Confirmed distinct resolution/bitrate/visible-quality tradeoffs across all three presets, not just distinct config values |
| P1-7 `preview fast --out` | Fixed a relative-path resolution bug found while using it |
| CI | Still parked in `docs/ci/` — this session's push lacks `workflows` scope; the one flaky test that would have affected it is made deterministic |

## What is and isn't kept

The throwaway workspaces, their synthetic assets, the rendered preview/fast-preview files,
and the extracted frame PNGs/spectrograms all lived under the session's scratch directory
and were not committed — consistent with this repository never committing generated output
(`generated/` is gitignored) or binary video artifacts. What *is* kept is this report, the
regression tests (`overlay_position_expressions_parse_in_a_real_filtergraph`,
`video_filter_zoom_intent_only_applies_to_image_layers`'s updated assertions,
`audio_filter_mixes_a_video_layers_own_audio_when_not_muted`,
`audio_filter_skips_a_muted_video_layers_audio`), and the bug fixes themselves.
