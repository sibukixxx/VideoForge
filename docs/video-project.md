# VideoProject IR v1

This is the reference for `project.vfp.json` — the canonical intermediate representation
(`videoforge-project` crate) that every VideoForge stage after script parsing reads or writes.
For the full pipeline design (script DSL, workspace layout, CLI, VOICEVOX/FFmpeg adapters) see
[`docs/design/mvp-v0.2-cross-platform.md`](design/mvp-v0.2-cross-platform.md), whose §11 first
introduced this schema; this document tracks the schema itself as implemented, kept in sync by the
Contract Tests described below.

## Purpose

`project.vfp.json` is VideoForge's single source of truth (see the CLAUDE.md invariant of the same
name). Everything downstream — the FFmpeg preview, the YMM4 `.ymmp` export, the FCPXML/OTIO
interchange exporters, a future GUI editor — derives from it. Nothing upstream (the Markdown
script, VOICEVOX, character manifests) is visible past this boundary: a renderer or exporter reads
only `VideoProject`, never a script or a VOICEVOX response directly. This is what lets VideoForge
add a new renderer, a new exporter, or a new TTS provider without touching the other two.

## Architecture

```text
Markdown script (videoforge-script)
        │  parse → validate (videoforge-core::validate)
        ▼
Resolved dialogues + directives
        │  TTS (videoforge-core::tts, VOICEVOX or FakeTtsEngine) → AudioAssetRef-equivalent durations
        ▼
Timeline builder (videoforge-timeline)
        │  places every clip in absolute milliseconds
        ▼
   VideoProject IR  ──────────────►  project.vfp.json (this document)
        │                                   │
        │                     ┌─────────────┼─────────────────┬───────────────────┐
        ▼                     ▼             ▼                 ▼                   ▼
  validate_project      FFmpeg preview  YMM4 exporter   FCPXML/OTIO export   Asset Registry
  (structural, no I/O)  (videoforge-    (videoforge-    (videoforge-export-  (videoforge-core::
                         preview)        export-ymm4)    interchange)         assets, needs I/O)
```

`VideoProject` is a **Track + Clip** model, not a Scene/Segment tree: a `Track` groups clips of one
`TrackKind` (audio, caption, image, character, …) and a `Clip` is one timed, typed unit on a track.
This was already the shape in the codebase when this document was written and is deliberately kept
— see "Why Track/Clip and not Scene/Segment" below.

## Boundaries this IR enforces

* **TTS boundary.** `videoforge-core::tts::TtsEngine` is the seam; `VideoProject` never contains a
  VOICEVOX speaker id, endpoint, or audio-query payload — only a resolved `AudioClip::duration_ms`
  and a `RelativeAssetPath` to the rendered WAV. `videoforge-core::tts::FakeTtsEngine` and the real
  `videoforge-voicevox::VoicevoxEngine` are interchangeable behind this trait; a future OpenAI
  TTS/ElevenLabs/local-TTS provider is a new struct implementing the same trait, not an IR change.
* **Renderer boundary.** `videoforge-core::preview::PreviewRenderer` is the seam; `videoforge-preview`
  (FFmpeg) is the only implementation today, but it reads a `VideoProject` and never the other way
  around — the IR has no FFmpeg filtergraph syntax or codec settings in it (those live in
  `PreviewRequest`/`EncodeSettings`, passed alongside the project, not inside it).
* **Exporter boundary.** `videoforge-core::export::ProjectExporter` is the seam:
  `id() / capabilities() / export(ExportRequest { project, .. })`. `videoforge-export-ymm4`
  (Windows `.ymmp`, template-patch) and `videoforge-export-interchange` (`InterchangeExporter`,
  producing FCPXML or OTIO — the format a DaVinci Resolve/Premiere/After Effects import would use)
  both implement it today; both translate *from* the IR, and NLE-specific concepts (YMM4's item
  schema, FCPXML's `<spine>`) stay in those leaf crates, never in `videoforge-project`.
* **GUI boundary.** The Tauri desktop app (`apps/desktop`) is a client of this IR: it calls
  `videoforge-core` to validate/generate a project and displays the resulting `VideoProject`; it
  never builds an FFmpeg command or a YMM4 item itself (design principle in
  `docs/design/mvp-v0.2-cross-platform.md` §24).

## Schema

### Top level — `VideoProject`

| Field | Type | Notes |
|---|---|---|
| `schema_version` | `u32` | Currently `1` (`videoforge_project::SCHEMA_VERSION`). See Versioning. |
| `id` | `string` | The project/script slug. |
| `title` | `string` | From the script's front matter, or the slug. |
| `video` | `VideoSettings` | `{ width, height, fps }` — all `u32`, all range-checked positive by `validate_project`. |
| `source` | `SourceInfo` | `{ script?, template?, generator_version? }` — all optional, all provenance metadata; nothing here is read by a renderer/exporter. |
| `tracks` | `Track[]` | See below. Defaults to `[]`. |
| *(unknown fields)* | — | Preserved verbatim via `#[serde(flatten)]` into an internal `extra` map — see Compatibility. |

### `Track`

```json
{ "id": "audio", "kind": "audio", "clips": [ /* Clip[] */ ] }
```

`kind` is one of: `audio`, `caption`, `character`, `image`, `background`, `sound_effect`, `bgm`,
`character_performance`, `video`, `text` (`TrackKind`). A track's `clips` must all be the *same*
kind as the track (`validate_project`'s `track_clip_kind_mismatch`) — a track is a grouping
convenience for a renderer to iterate one kind at a time, not an independent structural concept
with its own rules.

### Timing model

Every clip carries `start_ms: u64` and `duration_ms: u64` — absolute position and length on the
timeline, in milliseconds. **Milliseconds are canonical**; frame numbers are computed only at
export time via `videoforge_project::time::millis_to_frame(ms, fps)`, so changing a project's `fps`
never rewrites `project.vfp.json`. There is no separate "Scene" grouping with its own `start_ms` —
each clip's own `start_ms` is authoritative, avoiding the double-bookkeeping the original P0 brief
warned about (a Scene-level `start_ms` that could drift from its children's own timings).

### Clip variants

Every `Clip` is a tagged union (`#[serde(tag = "type")]`) over these variants — each has its own
`id: string`, `start_ms`, `duration_ms`, and (except `Caption`/`Text`) a `source: RelativeAssetPath`:

| `type` | Track kind | Distinguishing fields |
|---|---|---|
| `audio` | `audio` | `speaker: string` (canonical speaker key). |
| `caption` | `caption` | `text`, `speaker`, `speaker_display?`, `color?` (per-speaker override; falls back to `preview.subtitle.font_color`). |
| `background` | `background` | Just `source` + timing — a static full-frame background. |
| `image` | `image` | `transform: Transform`, `presentation?: Presentation`. |
| `character` | `character` | A static 立ち絵 stand-in image; `speaker?`, `transform`, `presentation?`. |
| `character_performance` | `character_performance` | No `source` — instead `character` (id into the character manifest), `expression` (default `"default"`), `motion` (default `"idle"`), `lip_sync: RelativeAssetPath` (a deterministic lip-sync curve JSON), `transform`. |
| `bgm` | `bgm` | `volume`, `looping`, `trim_start_ms`, `fade_in_ms`, `fade_out_ms`, `normalize`. No `transform` — audio only, no frame placement. Ducked under overlapping dialogue by the renderer; the IR does not store the ducked level. |
| `sound_effect` | `sound_effect` | `volume`. Audio only, one-shot. |
| `video` | `video` | `trim_start_ms`, `volume`, `muted`, `looping`, `transform` (its `crop` is applied before `fit`). |
| `text` | `text` | On-screen text distinct from `caption` (not tied to a dialogue's speaker/timing); `transform`, `color?`. |

Every clip type also carries an `extra` map (`#[serde(flatten)]`) for forward-compatible unknown
fields, same as the project root.

### `Transform` (visual clips only: image, character, character_performance, video, text)

```json
{ "x": 0.5, "y": 0.5, "scale": 1.0, "rotation_deg": 0.0, "opacity": 1.0, "layer": 0, "fit": "contain", "crop": null }
```

* `x`, `y` — the clip's **centre**, normalized `0.0..=1.0` against the frame (`0.5, 0.5` = centre).
  Because these are fractions, not pixels, changing `video.width`/`video.height` (e.g. via a render
  preset) never requires recomputing clip placement.
* `scale` multiplies the size `fit` produces; `rotation_deg` is clockwise around the centre;
  `opacity` is `0.0..=1.0`; `layer` is z-order (larger drawn in front).
* `fit` is a closed enum — `contain` | `cover` | `stretch` | `none` — deliberately closed so every
  exporter can map every value; a renderer-specific fit mode belongs in `extra`, not a new variant.
* `crop`, when set, is a `CropRect { x, y, width, height }` normalized to the *source* image
  (`{0,0,1,1}` keeps the whole source), applied before `fit`/`scale`.
* All fields default (a bare `{}` or a partial object is valid) — `Transform::default()` is
  centred, fitted, fully opaque, unrotated, uncropped.

There is no time-varying field here (no keyframes, no easing) — see `Presentation` below for how
motion is expressed instead.

### `Presentation` (optional, on image/character/video/text clips)

```json
{ "role": "primary_visual", "intent": "fade", "intent_duration_ms": 300 }
```

Two free-form (not closed-enum) strings a renderer or an agent can ask about a visual clip, kept
separate from the concrete `Transform`:

* `role` — *what is this for?* Recommended vocabulary: `primary_visual`, `supporting_visual`,
  `diagram`, `character`, `background`, `callout`, `comparison`, `emphasis`, `overlay`.
* `intent` — *how should it appear?* Recommended vocabulary: `fade`, `slide`, `zoom`, `emphasis`,
  `cut`. `videoforge-preview` renders `fade` as an alpha ramp over `intent_duration_ms`, `slide`/
  `zoom` as a Ken-Burns-style pan/zoom over the clip's own `duration_ms`, anything else as a cut.

Unknown words are accepted (a script author or an agent may write outside the recommended
vocabulary) — `videoforge-core::validate` only warns, never errors, on an unrecognized `role`/
`intent`. There is deliberately no separate "OverlayClip" type: an overlay (watermark, lower-third)
is exactly an `ImageClip` with `role: "overlay"`.

### Asset paths — `RelativeAssetPath`

Every `source`/`lip_sync` field is a `RelativeAssetPath`, not a plain string: relative to the
directory containing `project.vfp.json`, forward-slash only, no `..`, no drive letter. Construction
*and* deserialization both reject an absolute path (Windows `C:\`, POSIX `/…`, UNC `//…`) or a
backslash — a malformed or platform-specific path can never enter the IR, so a project built on one
OS stays portable to another. Resolve it to a real filesystem path with `.resolve(project_dir)`.

## Versioning and compatibility

* `schema_version` (currently `1`) is checked at load: `VideoProject::from_value` rejects a
  `schema_version` newer than this build supports (`ProjectError::UnsupportedSchema`) and rejects a
  missing one (`ProjectError::MissingSchemaVersion`). A version this build *does* support but is
  older than current would be lifted by a migration step in that same function — none has been
  needed since v1 was frozen.
* **Additive changes do not bump `schema_version`.** A new optional field (`Option<T>` with
  `#[serde(default)]`) — e.g. `CharacterPerformanceClip::transform` — loads correctly from an older
  file (defaults kick in) and an older build loading a newer file still works, because:
* **Unknown fields round-trip.** Every struct with room to grow (`VideoProject`, every `Clip`
  variant) carries `#[serde(flatten)] extra: BTreeMap<String, Value>`. A field this build doesn't
  know about is preserved byte-for-byte through a load→save cycle rather than silently dropped —
  this is what lets a newer GUI or a newer renderer add project-level metadata without breaking an
  older CLI that only regenerates a project's audio.
* `schema_version` bumps only for an incompatible change (a renamed field, a changed meaning of an
  existing field, a removed required field) — see the invariant in the repo's `CLAUDE.md`.

## Validation

Validation is split across two layers, deliberately, because one needs filesystem access and the
other must not:

1. **Parse-time (`VideoProject::from_json` → `ProjectError`).** Pure, no I/O: rejects a missing or
   unsupported `schema_version`, or a malformed asset path, before a `VideoProject` value even
   exists.
2. **Structural (`videoforge_project::validate_project` → `ProjectValidationReport`).** Pure, no
   I/O, operates on an already-parsed `VideoProject`: empty/duplicate track or clip ids, a clip
   whose `type` doesn't match its track's `kind`, zero-duration clips, `start_ms + duration_ms`
   overflow, zero-valued `video` settings — all `errors`. A caption with no audio clip at the same
   speaker/timing, or a visual clip extending past the end of the dialogue timeline, are `warnings`
   — a report is `is_ok()` (safe to proceed) based on `errors` alone. Every issue carries a stable
   `code` (e.g. `duplicate_clip_id`) and a JSONPath-like `path` so a GUI or CI can key off it rather
   than parsing the message string.
3. **Asset existence (`videoforge_core::assets::AssetRegistry`, filesystem-dependent, so it lives in
   `videoforge-core`, not this crate).** `build_registry` resolves every clip's `RelativeAssetPath`
   (plus each referenced character's manifest files) against the project directory, records whether
   each exists, and SHA-256 hashes the ones that do; `AssetRegistry::missing()` lists what doesn't.
   This is the "does this asset reference actually resolve to a file" check from the P0 brief —
   kept separate from `validate_project` so the latter stays usable in a WASM/browser context with
   no filesystem at all (`videoforge-timeline-wasm` calls exactly this pure path).

Script-level checks one level upstream of the IR (unknown speaker, unknown character/expression/
motion, a directive's asset not found) live in `videoforge_core::validate` and run before the IR is
even built — see `docs/design/mvp-v0.2-cross-platform.md` §7.3.

## Determinism

The same script, config, and TTS engine produce byte-identical `project.vfp.json` output:

* Clip and track ids are sequential and derived from script order (`audio-001`, `caption-002`, …),
  never a random UUID.
* `FakeTtsEngine::duration_for` derives duration purely from character count — no wall-clock or
  randomness — which is what makes Contract Test B (below) reproducible offline.
* The only field that varies between builds is `source.generator_version` (`CARGO_PKG_VERSION`),
  and only on a deliberate crate version bump — never on rebuild, retry, or a different machine.
* `manifest.json` (a *sibling* file to `project.vfp.json`, not part of the IR) does carry a
  wall-clock `generated_at` timestamp — that non-determinism is confined to the manifest, which is
  documentation of one particular generate run, not the IR itself.

## Contract Tests

Three fixture-backed suites exist specifically to catch an unintended change to this schema, distinct
from the many inline unit tests already covering individual behaviors:

| Test | Location | What it freezes |
|---|---|---|
| A. Serialization | `crates/videoforge-project/tests/contract_serialization.rs` + `fixtures/video_project/basic.vfp.json` | A hand-built `VideoProject` touching every `Clip` variant and every optional field, compared byte-for-byte against the fixture in both directions (serialize and deserialize). |
| B. Script → IR | `crates/videoforge-core/tests/contract_script_to_ir.rs` + `fixtures/scripts/ir-contract-basic.md` + `fixtures/projects/ir-contract-basic.vfp.json` | The real `parse → validate → TTS(fake) → timeline → IR` pipeline end to end, byte-for-byte against the fixture. |
| C. Validation | `crates/videoforge-project/tests/contract_validation.rs` + `fixtures/video_project/invalid/*.json` | Each invalid fixture fails with a specific, named `ProjectError` variant or `ProjectValidationIssue::code` — one fixture per failure mode (missing/unsupported schema version, an absolute asset path, duplicate track/clip id, zero-duration clip, track/clip kind mismatch). |

When an intentional IR change is made, update the corresponding fixture(s) in the same commit (for
A/B, regenerate by printing `VideoProject::to_json()`/`GeneratedProject::project.to_json()` from the
test's own builder/pipeline and copying that output over the fixture file) — an unreviewed fixture
diff is the signal this whole document exists to make visible.

## Why Track/Clip and not Scene/Segment

An earlier design pass (and any brief referencing "Scene"/"Segment") considered a nested
`Scene { segments: [...] }` shape. What is actually implemented, and deliberately kept, is flatter:
one list of `Track`s, each a flat list of `Clip`s in absolute `start_ms`. A `Track` groups clips by
*kind* (all captions together, all BGM together) rather than by *narrative scene* — a renderer
iterating "all bgm clips" or "all caption clips" is a track lookup either way, and a nested
Scene→Segment tree would need its own start-time bookkeeping (exactly the double-bookkeeping risk
the original P0 brief flagged) without changing what any consumer actually needs to iterate. If a
narrative "scene" grouping becomes a real requirement later (e.g. for a GUI's scene-by-scene
editing view), it can be added as an optional grouping annotation over existing clips (e.g. a
`scene_id` on `Presentation` or a new top-level `scenes: [{ id, clip_ids: [...] }]` index) without
restructuring `tracks`/`clips` — additive, per the Compatibility rules above, not a schema bump.

## Example

A minimal but complete two-line dialogue, as actually produced by the pipeline (this is
`fixtures/projects/ir-contract-basic.vfp.json`, Contract Test B's golden fixture, generated with
`--fake-tts` so it needs neither VOICEVOX nor FFmpeg to reproduce):

```json
{
  "schema_version": 1,
  "id": "ir-contract-basic",
  "title": "IR Contract Fixture",
  "video": { "width": 1920, "height": 1080, "fps": 30 },
  "source": {
    "script": "scripts/ir-contract-basic.md",
    "template": "default",
    "generator_version": "0.1.0"
  },
  "tracks": [
    {
      "id": "audio",
      "kind": "audio",
      "clips": [
        { "type": "audio", "id": "audio-001", "source": "assets/audio/001.wav",
          "start_ms": 0, "duration_ms": 857, "speaker": "reimu" },
        { "type": "audio", "id": "audio-002", "source": "assets/audio/002.wav",
          "start_ms": 1057, "duration_ms": 463, "speaker": "marisa" }
      ]
    },
    {
      "id": "caption",
      "kind": "caption",
      "clips": [
        { "type": "caption", "id": "caption-001", "text": "こんにちは。",
          "start_ms": 0, "duration_ms": 857, "speaker": "reimu", "speaker_display": "霊夢" },
        { "type": "caption", "id": "caption-002", "text": "やあ。",
          "start_ms": 1057, "duration_ms": 463, "speaker": "marisa", "speaker_display": "魔理沙" }
      ]
    }
  ]
}
```

For an example touching every clip kind (image, character, character_performance, bgm, sound
effect, video, text) see `fixtures/video_project/basic.vfp.json` (Contract Test A's fixture).
