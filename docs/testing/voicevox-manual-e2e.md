# Manual E2E: real VOICEVOX Engine

Tracks: [P1] "実VOICEVOXを使ったManual E2Eを追加する" (issue #10). Design §14
(VOICEVOX flow), §15 (VOICEVOX on macOS), §44 (manual acceptance on macOS).

`FakeTtsEngine` proves the plumbing offline; this procedure proves the real
thing end to end:

```text
sample.md → audio_query → synthesis → WAV → duration parse
         → project.vfp.json → captions.srt → preview.mp4
```

Two automated tests do most of the work once an engine is running. Run them
first; the manual steps below add what a test cannot judge (does it *sound*
right, does the preview *look* right) and record the environment.

## Prerequisites

- [VOICEVOX](https://voicevox.hiroshiba.jp/) (the app, or VOICEVOX Engine
  alone) started and answering at `http://127.0.0.1:50021`. On macOS the CPU
  build is enough (§15). Check with your browser: `http://127.0.0.1:50021/docs`.
- FFmpeg on `PATH` (or `VIDEOFORGE_FFMPEG=/path/to/ffmpeg`) if you want
  `preview.mp4`; without it `generate` records a `preview skipped` warning and
  everything else still runs.
- A release or debug build of the CLI: `cargo build -p videoforge-cli`.

Non-default endpoint (e.g. Docker): export `VIDEOFORGE_VOICEVOX_ENDPOINT`
for the tests and pass `--endpoint <url>` to the CLI. Anything other than
loopback also needs `tts.allow_remote_endpoint: true` in `videoforge.yaml`.

## Automated part

```bash
# engine-level: /version, /speakers, audio_query → synthesis → WAV header,
# and that a speed_scale override actually changes the audio length
cargo test -p videoforge-voicevox --test voicevox_real -- --nocapture

# CLI-level: init → validate → doctor → generate → artifacts → cache hit
cargo test -p videoforge-cli --test voicevox_real -- --nocapture
```

Both print `skipped: no VOICEVOX …` and pass when no engine answers, so they
are safe in the offline `cargo test --workspace`. With an engine running, the
CLI test asserts:

| Artifact | Expectation |
|---|---|
| `doctor --json` | `capabilities.voicevox_available == true`; the check detail names the engine version |
| `project.vfp.json` | 3 audio clips, `duration_ms` of each equals the RIFF header duration of its WAV, clips do not overlap |
| `assets/audio/00N.wav` | parseable RIFF/WAVE, duration > 0 |
| `captions.srt` | 3 cues, script text verbatim |
| `manifest.json` | `dialogues == 3`, `duration_ms` = end of the last clip, no `TTS cache disabled` warning |
| `preview.mp4` | present and > 1 KiB when `doctor` found FFmpeg, absent otherwise |
| second `generate` | all 3 dialogues reported `(cached)` — the cache key includes the engine version (issue #11) |

## Manual part

1. Start VOICEVOX and confirm the engine version: `videoforge doctor` shows
   `VOICEVOX connection … VOICEVOX <version> at http://127.0.0.1:50021`.
2. In a fresh workspace (`videoforge init demo && cd demo`), copy
   `fixtures/scripts/sample.md` to `scripts/sample.md`.
3. `videoforge speakers` — confirm the style ids used by `videoforge.yaml`
   (`reimu: 2`, `marisa: 3` by default) exist on this engine. Different
   VOICEVOX releases add styles but have not renumbered the defaults.
4. `videoforge generate scripts/sample.md`.
5. Play `generated/sample/assets/audio/001.wav` … `003.wav`: 霊夢 (style 2)
   for 001 and 003, 魔理沙 (style 3) for 002, text matches the script, no
   clipping or truncation at the end.
6. Play `generated/sample/preview.mp4`: audio and captions line up, the
   caption changes exactly when the next voice starts (with the configured
   `dialogue_gap_ms` of silence between them).
7. Open `generated/sample/captions.srt` in a player alongside the WAVs or in
   the preview: cue boundaries match what you hear.
8. Run `videoforge generate scripts/sample.md` again and confirm every
   dialogue prints `(cached)` and the run finishes without contacting the
   synthesizer (watch the VOICEVOX log if in doubt).
9. Quit VOICEVOX and run `generate` once more: it must fail fast with
   `voicevox_unavailable` (`--json` shows the code) and leave no
   `generated/sample` half-written (the previous output stays intact).

Record the result below with the OS build, VOICEVOX version, FFmpeg version
and VideoForge commit.

## Results log

| Date | OS | VOICEVOX | FFmpeg | VideoForge commit | Automated | Manual 5–9 | Notes |
| ---- | -- | -------- | ------ | ----------------- | --------- | ---------- | ----- |
| 2026-09-06 | macOS 26.1 (arm64) | 0.25.2 | 9.0.1 (Homebrew) | ad5d893 + this commit | pass (both tests, preview rendered) | _not yet_ | run by the CLI test in this repository checkout |
| _(not yet run)_ | Windows | | | | | | |
