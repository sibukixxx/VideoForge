# Character system (VOICEVOX + Live2D)

Status: P0 vertical slice implemented (script → character-linked voice →
lip-sync data → `character_performance` timeline track). Frame rendering to
`preview.mp4` is **not** implemented yet — see
`docs/live2d-renderer-decision.md` for the plan and
`docs/character-video-pipeline.md` for the end-to-end pipeline this feature
extends.

## Why this exists

VideoForge's script → TTS → timeline → preview pipeline only knew about a
"speaker": a voice, a caption label, and (via `@character`) a static
stand-in image. This feature adds an optional second layer — a **character**
with a VOICEVOX voice identified by *name* (not a hard-coded numeric id) and
an optional Live2D model — without touching that existing pipeline for any
project that doesn't use it.

Nothing here is specific to any one VOICEVOX character. `videoforge-character`
has no notion of "Tsumugi" or "Zundamon"; a character is metadata (`id`,
`display_name`, `voice`, `model`, `expressions`, `motions`). The repository's
own fixture (`fixtures/character/mock-character/`) is a synthetic character
used only by the test suite.

## Crates and modules touched

| Crate / module | Responsibility |
|---|---|
| `videoforge-character` (new crate) | `Character`/`CharacterManifest`/`CharacterVoice`/`CharacterModel` domain types; `live2d::load_model3_json` (metadata-only Live2D model reader: expression names, motion group names). No Tauri/Windows/YMM4 dependency, same as the other domain crates. |
| `videoforge-core::config` | `Config::character_manifest: Option<String>` (workspace-relative path to a manifest) and `SpeakerConfig::character_id: Option<String>` (links a speaker to a character in that manifest). Both optional; `Config::validate` requires a manifest path whenever any speaker sets `character_id`. |
| `videoforge-core::character` | `load_manifest` (resolves and loads the manifest a workspace's config points at) and `resolve_character_voices` (fills `VoiceParams::speaker_id` from a character's named voice via `TtsEngine::list_speakers()`). |
| `videoforge-core::validate` | For a dialogue whose speaker links a character: validates `expression=`/`motion=` script attributes against that character's known list (explicit `expressions`/`motions` in the manifest, or read from its `model3.json`) instead of warning them as unsupported; resolves `character_id`/`expression`/`motion` onto `ResolvedDialogue`. |
| `videoforge-core::lipsync` (new module) | `analyze_amplitude`: deterministic WAV → RMS-amplitude-per-window lip-sync curve (design §10). Pure function, fully unit tested, no external dependency. |
| `videoforge-core::wav` | `decode_pcm16_mono`: added alongside the existing header-only `parse_wav_info`, needed to actually read PCM samples for amplitude analysis. |
| `videoforge-core::generate` | Calls `resolve_character_voices` before validation (so the resolved numeric id is what validation and the TTS cache see), then — after TTS synthesis — analyzes each character-linked dialogue's WAV, writes `assets/character/<id>/lipsync-<index>.json`, and feeds a `CharacterPerformanceInput` per dialogue into the timeline builder. |
| `videoforge-timeline` | `TimelineInput::character_performance: Vec<CharacterPerformanceInput>`; `build()` places one `CharacterPerformanceClip` per entry at its dialogue's scheduled start/duration, in a new `character_performance` track — only when the input is non-empty. |
| `videoforge-project` | `TrackKind::CharacterPerformance` / `Clip::CharacterPerformance(CharacterPerformanceClip)` — a new, additive clip type (`character`, `expression`, `motion`, `lip_sync: RelativeAssetPath`). Does **not** bump `SCHEMA_VERSION` (additive, matches the existing test asserting that). |
| `videoforge-cli` | `videoforge character inspect <manifest>` (offline: parse + validate manifest structure, load each Live2D model's `model3.json`, print expressions/motions) and `videoforge character validate <manifest> [--fake-tts|--endpoint]` (adds VOICEVOX reachability + named speaker/style resolution, exit 2 on any character failing). |

## Why "character" is not the existing `@character` directive

VideoForge already has `TrackKind::Character` / `Clip::CharacterClip` for
**立ち絵** (a static stand-in image placed via the `@character` script
directive). That is a different concept from this feature's "character" (a
voice + Live2D performance identity) and the two are **not** merged: this
feature's track is `TrackKind::CharacterPerformance` /
`Clip::CharacterPerformance`. A script can use either, both, or neither.

## Script format: no grammar change

The script parser already supports `Name[key=value, key2=value2]:` headers
(used today for e.g. `@character reimu[expression=happy]`). This feature
reuses that exact mechanism for dialogue lines:

```
tsumugi[expression=smile, motion=Wave]:
こんにちは。今日はAIについて解説します。
```

* `tsumugi` is a **speaker key** (or alias) in `videoforge.yaml`, exactly
  like any other speaker — resolved via the same `Config::resolve_speaker`.
* `expression=`/`motion=` are recognized **only** when that speaker's
  `SpeakerConfig::character_id` is set. On a plain speaker they still warn as
  an unsupported attribute — the very same "not supported in v0.1" warning
  every unrecognized dialogue attribute already produced, so a script that
  never uses a character sees no behavior change at all.
* Omitting them uses `"default"`/`"idle"` when the performance clip is built
  (never at validation time — validation leaves them as `None` when unsaid).
* An unknown expression or motion name — checked against the character's
  explicit `expressions`/`motions` list if given, else the model's
  `model3.json` — is a **validation error**, not a runtime crash or a
  warning (design §12).

## `videoforge.yaml`: linking a speaker to a character

```yaml
character_manifest: characters.yaml   # workspace-relative path

speakers:
  tsumugi:
    aliases: [つむぎ, 春日部つむぎ]
    character_id: tsumugi   # id inside characters.yaml
    voice:
      speaker_id: 0          # ignored: overwritten from the character's named voice
```

`Config::validate` rejects a `character_id` with no `character_manifest`
configured, so this combination can never silently do nothing.

## The character manifest itself

A **standalone** file, deliberately not embedded in `videoforge.yaml` — it's
meant to be reusable across projects and kept separate from any one video's
config, matching the licensing separation in `docs/character-licensing.md`:

```yaml
characters:
  - id: tsumugi
    display_name: "春日部つむぎ"
    voice:
      provider: voicevox
      speaker: "春日部つむぎ"   # VOICEVOX speaker *name*
      style: "ノーマル"          # VOICEVOX style *name*
    model:
      type: live2d
      path: /Users/you/live2d-models/kasukabe-tsumugi/model.model3.json
    # optional explicit allow-lists; otherwise read from the model3.json
    # expressions: [smile, surprised]
    # motions: [wave, nod]
```

`model.path` may be absolute or relative to the manifest file itself
(`CharacterModel::resolve_path`). It is **never** a `RelativeAssetPath` and
is never copied into a workspace or a generated output — Live2D model files
are explicitly out of scope for this repository, its releases, and any
`generated/` output (design §6, `docs/character-licensing.md`).

## VOICEVOX voice resolution: name, never a hard-coded id

`core::character::resolve_character_voices` is called once, at the very
start of `generate()`, before parsing/validating the script:

1. No-op (no network call at all) unless some speaker's config sets
   `character_id` — every existing numeric-`speaker_id` workspace is
   completely unaffected, including every existing test.
2. Otherwise it loads the character manifest, and for each linked character
   that declares a `voice`, calls `TtsEngine::list_speakers()` **once**
   (cached across all characters in the same run) and matches
   `voice.speaker`/`voice.style` by name against the returned
   `Speaker`/`SpeakerStyle` list.
3. The resolved numeric id overwrites `SpeakerConfig::voice.speaker_id`
   in-memory, before validation builds `ResolvedDialogue::voice` from it —
   so everything downstream (the TTS cache key, `synthesize_all`, the fake
   engine used by every offline test) keeps using the exact same
   `VoiceParams` struct it always has. **Nothing about `TtsEngine`,
   `TtsCache`, or synthesis was changed** to add this.
4. No match → `AppError::VoicevoxSpeakerNotFound` (code
   `voicevox_speaker_not_found`), naming every speaker/style VOICEVOX
   actually has.

## Lip sync: deterministic amplitude, not phoneme analysis

`core::lipsync::analyze_amplitude(wav_bytes, interval_ms)` computes RMS
amplitude in fixed windows (default `DEFAULT_INTERVAL_MS = 50`, matching the
example in design §10) and maps it to a `mouth_open` value in `[0.0, 1.0]`.
It is:

* **Pure and deterministic** — the same WAV always produces the same curve,
  independent of playback; no phoneme model, no ML.
* **Written to disk** as `assets/character/<character_id>/lipsync-<index>.json`
  (a `LipSyncTrack { interval_ms, samples: [{ t_ms, mouth_open }, ...] }`),
  referenced from the `CharacterPerformanceClip::lip_sync` field — so a
  renderer built later can render frames **offline**, without VOICEVOX or a
  live playback session, exactly as design §10 requires.

This is intentionally the cheapest thing that produces a real, reproducible
lip-sync track; phoneme-perfect lip sync is an explicit non-goal (design §30).

## Error model additions

New `AppError` variants (design §22), each with its own stable `code()`:

| Variant | `code()` | When |
|---|---|---|
| `VoicevoxSpeakerNotFound` | `voicevox_speaker_not_found` | a character's named voice has no matching VOICEVOX speaker/style |
| `CharacterNotFound` | `character_not_found` | `character_id` has no entry in the manifest |
| `CharacterManifestInvalid` | `character_manifest_invalid` | the manifest YAML itself is invalid |
| `Live2dModelNotFound` | `live2d_model_not_found` | `model.path` does not resolve to a file |
| `Live2dModelInvalid` | `live2d_model_invalid` | the file exists but is not valid `model3.json` |
| `ExpressionNotFound` / `MotionNotFound` | `expression_not_found` / `motion_not_found` | a script names an expression/motion the character doesn't have |
| `LipSyncGenerationFailed` | `lipsync_generation_failed` | amplitude analysis failed on a dialogue's WAV (e.g. corrupt audio) |

`LIVE2D_RENDER_FAILED`, `FRAME_RENDER_FAILED`, and `COMPOSITION_FAILED` from
design §22 are **reserved, not yet implemented** — nothing in P0 renders a
frame, so there is nothing yet to fail that way (see "Known gaps" in
`CLAUDE.md` and `docs/live2d-renderer-decision.md`).

## CLI

```bash
videoforge character inspect ./characters.yaml
videoforge character validate ./characters.yaml --fake-tts   # or a real VOICEVOX endpoint
```

`inspect` is fully offline (no workspace, no VOICEVOX): it parses the
manifest, loads each character's `model3.json`, and prints its expressions
and motions. `validate` does everything `inspect` does plus resolves each
character's named voice against a live (or `--fake-tts`) VOICEVOX and exits
`2` if any character fails — the same exit-code convention as `videoforge
doctor`/`videoforge validate`.

## Testing

No real Live2D asset is ever used by the test suite. Every test — in
`videoforge-character`, `videoforge-core` (`character`, `validate`, `wav`,
`lipsync`, `generate`), `videoforge-project`, `videoforge-timeline`, and the
CLI's `tests/cli.rs` — runs against `fixtures/character/mock-character/`: a
synthetic `manifest.yaml` and a `model3.json` with the same field shape a
real Cubism model uses but no meshes, textures, or copyrighted content of any
kind, plus `FakeTtsEngine` (`--fake-tts`) for the voice side.

## What is *not* here yet (Phase 1)

* No frame rendering, no `preview.mp4` integration, no FFmpeg overlay of a
  rendered character — see `docs/live2d-renderer-decision.md`.
* No GUI character preview panel.
* No automatic (LLM-driven) expression/motion selection (design §13, P1).
