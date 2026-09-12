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
  covered by `docs/testing/voicevox-manual-e2e.md`.
- **`subtitle.position: top`** was not exercised (this run used the default `bottom`) —
  still an open item in `CLAUDE.md`'s Known Gaps.
- **The `"slide"` Ken Burns intent** was not exercised (this run used `zoom`).
- **A video clip's own embedded audio** is still not mixed in (documented gap since P1-4) —
  the `mandelbrot` clip in this run was `muted=true` for exactly that reason.
- **No visual regression tooling.** Frames were eyeballed once, by hand, in this session;
  there is no automated pixel-diff or golden-frame test here or anywhere else in the repo.

## What is and isn't kept

The throwaway workspace, its synthetic assets, the rendered `preview.mp4`/
`preview.fast.mp4`, and the extracted frame PNGs all lived under the session's scratch
directory and were not committed — consistent with this repository never committing
generated output (`generated/` is gitignored) or binary video artifacts. What *is* kept is
this report, the regression test
(`overlay_position_expressions_parse_in_a_real_filtergraph`), and the bug fix itself.
