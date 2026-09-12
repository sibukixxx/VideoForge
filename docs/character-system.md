# Character system (VOICEVOX + Live2D + 3-state PNG)

Status: script → character-linked voice → lip-sync data →
`character_performance` timeline track → **rendered into `preview.mp4`** for
`model.type: png_lipsync` characters (P0-1: 3-state PNG renderer). Live2D
frame rendering is still **not** implemented — see
`docs/live2d-renderer-decision.md` for that plan and
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
| `videoforge-character` | `Character`/`CharacterManifest`/`CharacterVoice`/`CharacterModel`/`CharacterPresentation`/`CharacterPosition` domain types; `live2d::load_model3_json` (metadata-only Live2D model reader); `png::read_png_info`/`load_png_info` (metadata-only PNG header reader: dimensions, alpha channel — P0-1); `png_lipsync::load_png_lipsync_assets` (validates a `png_lipsync` model's three sprites). No Tauri/Windows/YMM4 dependency, same as the other domain crates. |
| `videoforge-core::config` | `Config::character_manifest: Option<String>` (workspace-relative path to a manifest) and `SpeakerConfig::character_id: Option<String>` (links a speaker to a character in that manifest). Both optional; `Config::validate` requires a manifest path whenever any speaker sets `character_id`. |
| `videoforge-core::character` | `load_manifest` (resolves and loads the manifest a workspace's config points at), `resolve_character_voices` (fills `VoiceParams::speaker_id` from a character's named voice via `TtsEngine::list_speakers()`), `presentation_transform` (maps a character's `presentation.{position,scale}` onto the same `Transform` every other visual clip uses — P0-1), `resolve_png_sprites` (resolves a `png_lipsync` character's three sprite paths). |
| `videoforge-core::validate` | For a dialogue whose speaker links a character: validates `expression=`/`motion=` script attributes against that character's known list (Live2D only, from `expressions`/`motions` or its `model3.json`) instead of warning them as unsupported; for a `png_lipsync` character, validates its three PNG sprites exist and carry an alpha channel; resolves `character_id`/`expression`/`motion` onto `ResolvedDialogue`. |
| `videoforge-core::doctor` | One `Character \`<id>\`` check per linked character (P0-3): voice resolves against a live/fake VOICEVOX, and the model (Live2D `model3.json` or PNG sprite set) is valid — *before* a generate is attempted. Never silently falls back to a different VOICEVOX voice. |
| `videoforge-core::lipsync` | `analyze_amplitude`: deterministic WAV → RMS-amplitude-per-window lip-sync curve (design §10). `mouth_state`/`MOUTH_HALF_THRESHOLD`/`MOUTH_OPEN_THRESHOLD`: maps one amplitude sample to `Closed`/`Half`/`Open` (P0-1). `mouth_segments`: merges a curve into timeline-absolute `Half`/`Open` windows for a renderer. All pure functions, fully unit tested, no external dependency. |
| `videoforge-core::wav` | `decode_pcm16_mono`: added alongside the existing header-only `parse_wav_info`, needed to actually read PCM samples for amplitude analysis. |
| `videoforge-core::preview` | `PreviewRequest::character_sprites: BTreeMap<String, CharacterSpriteSet>` — absolute paths to every `png_lipsync` character's sprites, resolved fresh each run and passed to the renderer exactly like `PreviewRequest::font` (never copied into `generated/`, never part of the project IR). |
| `videoforge-core::generate` | Calls `resolve_character_voices` before validation, then — after TTS synthesis — analyzes each character-linked dialogue's WAV, writes `assets/character/<id>/lipsync-<index>.json`, resolves that character's `presentation_transform` and (for `png_lipsync`) sprite paths, and feeds a `CharacterPerformanceInput` per dialogue into the timeline builder. |
| `videoforge-timeline` | `TimelineInput::character_performance: Vec<CharacterPerformanceInput>` (now carries `transform: Transform`); `build()` places one `CharacterPerformanceClip` per entry at its dialogue's scheduled start/duration, in a new `character_performance` track — only when the input is non-empty. |
| `videoforge-project` | `TrackKind::CharacterPerformance` / `Clip::CharacterPerformance(CharacterPerformanceClip)` — a new, additive clip type (`character`, `expression`, `motion`, `lip_sync: RelativeAssetPath`, `transform: Transform`). Does **not** bump `SCHEMA_VERSION` (additive, matches the existing test asserting that). |
| `videoforge-preview::command` | `build_character_overlays`/`CharacterOverlayPlan`: reads each performance clip's lip-sync curve and turns it into FFmpeg `overlay` filters — `closed` always on, `half`/`open` time-windowed with `enable='between(t,…)+…'` — composited onto the background *before* the caption `drawtext` chain, so captions always draw on top (P0-1). Position/size come from `Transform`, clamped in the filter graph itself so a character never overflows the frame or the caption safe area. |
| `videoforge-cli` | `videoforge character inspect <manifest>` (offline: parse + validate manifest structure, load each Live2D model's `model3.json` or each `png_lipsync` model's three sprites, print expressions/motions or sprite paths) and `videoforge character validate <manifest> [--fake-tts|--endpoint]` (adds VOICEVOX reachability + named speaker/style resolution, exit 2 on any character failing). |

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

### `model.type: png_lipsync` (P0-1)

A second, simpler model type: three transparent PNGs (closed / half-open /
fully-open mouth) instead of a Live2D model. Unlike Live2D, this **is**
composited into `preview.mp4` today — see "3-state PNG rendering" below.

```yaml
characters:
  - id: zundamon
    display_name: "ずんだもん"
    voice:
      provider: voicevox
      speaker: "ずんだもん"
      style: "ノーマル"
    model:
      type: png_lipsync
      closed: ./characters/zundamon/closed.png
      half: ./characters/zundamon/half.png
      open: ./characters/zundamon/open.png
    presentation:
      position: right   # left | center (default) | right
      scale: 0.72        # CharacterManifest::{MIN,MAX}_CHARACTER_SCALE bound this
```

* `closed`/`half`/`open` resolve the same way as a Live2D `model.path`
  (absolute, or relative to the manifest file) and are validated the same
  way `model3.json` is: `videoforge character inspect`, `videoforge doctor`,
  and `generate`'s own validation pass all load and check them *before* any
  synthesis or rendering happens (`videoforge_character::png_lipsync::load_png_lipsync_assets`).
* Each PNG must carry a real alpha channel (RGBA or grayscale+alpha) —
  `CharacterError::PngNotTransparent` otherwise. Only the `IHDR` header is
  read; pixel data is never decoded by VideoForge itself, only by FFmpeg at
  render time.
* `presentation` is optional; omitting it centers the character at
  `scale: 1.0`. It is not a second placement system — `core::character::presentation_transform`
  maps it onto the exact same `Transform` every `ImageClip`/`CharacterClip`
  already uses (`x`/`y` normalized centre, `scale` a multiplier), so nothing
  downstream of `project.vfp.json` needs to know `png_lipsync` exists.

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

## 3-state PNG rendering (P0-1)

For a `png_lipsync` character, `videoforge-preview` turns the amplitude
curve above into a discrete pose and composites it onto `preview.mp4`:

1. `core::lipsync::mouth_state(mouth_open)` maps each sample to `Closed`
   (`< MOUTH_HALF_THRESHOLD`), `Half` (`< MOUTH_OPEN_THRESHOLD`), or `Open`,
   using fixed, named constants — not tunable per-project yet, but never a
   magic number buried in the renderer.
2. `core::lipsync::mouth_segments` merges consecutive same-state samples
   into timeline-absolute `Half`/`Open` windows (dropping `Closed`, which is
   the implicit base state).
3. `videoforge_preview::command::build_character_overlays` does this once
   per `png_lipsync` character referenced anywhere in the project's
   `character_performance` track, across *all* of that character's
   dialogue clips.
4. The FFmpeg filter graph overlays that character's `closed` sprite for the
   character's entire on-screen presence (an inactive speaker's default
   pose — this is what makes speaker B read as "closed", not "absent",
   while speaker A is talking), then overlays `half`/`open` on top only
   during their windows via `enable='between(t,s1,e1)+between(t,s2,e2)+…'`.
   Because `half`/`open` are full re-draws of the character (not just a
   mouth cut-out), this fully covers the closed pose wherever it is active.
5. Placement comes from the clip's own `transform` (`x`/`y` normalized
   centre, `scale`), clamped inside the filter graph itself
   (`min(max(0,…),…)` on both axes) so a character can never be pushed
   outside the frame, and vertically capped above the same pixel band the
   caption `drawtext` calls reserve — captions are always drawn *after* the
   character layer in the graph, so they are never hidden behind one.
6. Two or more characters are independent overlay chains, each keyed by
   character id — the acceptance scenario (A talks → only A's mouth moves,
   B stays closed; then the reverse) falls out of this directly, since each
   character's windows come only from *that* character's own performance
   clips.

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
| `PngSpriteNotFound` | `png_sprite_not_found` | a `png_lipsync` model's `closed`/`half`/`open` does not resolve to a file |
| `PngSpriteInvalid` | `png_sprite_invalid` | the file is not a valid PNG, or has no alpha channel |

`LIVE2D_RENDER_FAILED`, `FRAME_RENDER_FAILED`, and `COMPOSITION_FAILED` from
design §22 remain **reserved for the Live2D path**: `png_lipsync` doesn't
need them since compositing failures there surface as the ordinary
`PreviewRenderFailed`/FFmpeg-exit-status path every other preview failure
already uses (see "Known gaps" in `CLAUDE.md` and
`docs/live2d-renderer-decision.md`).

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

No real Live2D or character artwork is ever used by the test suite. Every
test — in `videoforge-character`, `videoforge-core` (`character`,
`validate`, `doctor`, `wav`, `lipsync`, `generate`), `videoforge-project`,
`videoforge-timeline`, `videoforge-preview`, and the CLI's `tests/cli.rs` —
runs against one of two fixtures:

* `fixtures/character/mock-character/`: a synthetic `manifest.yaml` and a
  `model3.json` with the same field shape a real Cubism model uses but no
  meshes, textures, or copyrighted content of any kind (Live2D path).
* `fixtures/character/mock-png-character/`: a synthetic `manifest.yaml`
  linking two `png_lipsync` characters (`mock_a` at `position: left`,
  `mock_b` at `position: right`) to tiny (8×8), programmatically generated
  RGBA checkerboard PNGs under `sprites/` — real, valid, transparent PNGs,
  just not artwork of any kind — plus `no_alpha.png` and `not_a_png.png` for
  the rejection paths.

`FakeTtsEngine` (`--fake-tts`) stands in for VOICEVOX in both. FFmpeg
compositing itself is tested offline at the command-builder level
(`videoforge-preview::command`), the same pattern
`crates/videoforge-preview/tests/ffmpeg_real.rs` uses for everything else —
a real-FFmpeg render of the two-character scenario is part of
`docs/testing/character-manual-e2e.md`.

## What is *not* here yet (Phase 1)

* No Live2D frame rendering, no FFmpeg overlay of a Live2D character — see
  `docs/live2d-renderer-decision.md`. (3-state PNG rendering **is** done —
  see "3-state PNG rendering" above.)
* No GUI character preview panel, and no GUI wiring at all (CLI-only, same
  as the rest of this feature).
* No automatic (LLM-driven) expression/motion selection (design §13, P1).
* A `png_lipsync` character's on-screen presence spans the *entire* video
  once it performs anywhere — there is no "leaves the frame" concept yet
  (matches the P0-1 acceptance scenario: an inactive speaker is `closed`,
  not absent).
* No configurable mouth-state thresholds — `MOUTH_HALF_THRESHOLD`/
  `MOUTH_OPEN_THRESHOLD` are fixed constants (`core::lipsync`), not exposed
  in `videoforge.yaml` or the character manifest yet.
