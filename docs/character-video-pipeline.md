# Character video pipeline

How the character/Live2D feature (`docs/character-system.md`) fits into
VideoForge's existing generation pipeline (`core::generate::generate`,
documented in `CLAUDE.md`). The pipeline below produces a
`character_performance` track with a lip-sync file on disk for *every*
character, Live2D or `png_lipsync` alike; a `png_lipsync` character is then
also composited into `preview.mp4` by `videoforge-preview` (P0-1, see
"3-state PNG rendering" in `docs/character-system.md`). Live2D frame
rendering and FFmpeg composition are still Phase 1
(`docs/live2d-renderer-decision.md`).

## Where it sits in `generate()`

```
Script
  │
  ▼
Parse
  │
  ▼
resolve_character_voices()   ← NEW, before validation. No-op unless a
  │                             speaker links a character. Resolves a
  │                             character's named VOICEVOX voice to a
  │                             numeric speaker_id via list_speakers().
  ▼
Validate (validate_script)
  │  • unknown speaker → error (unchanged)
  │  • speaker linked to a character:
  │      - unknown character_id → error               ← NEW
  │      - missing/invalid Live2D model file → error   ← NEW
  │      - unknown expression=/motion= value → error    ← NEW
  │      - otherwise: attributes resolved onto
  │        ResolvedDialogue.{character_id,expression,motion}
  ▼
TTS (synthesize_all, concurrent, cancellable, cached — unchanged)
  │
  ▼
Character performance (NEW)
  │  for each dialogue whose speaker links a character WITH a model:
  │    lipsync::analyze_amplitude(wav) → LipSyncTrack
  │    write assets/character/<id>/lipsync-<index>.json
  │    → CharacterPerformanceInput{character, expression, motion, lip_sync}
  ▼
Timeline (videoforge_timeline::build)
  │  • existing audio/caption/background/image/character/bgm/se tracks, unchanged
  │  • NEW: character_performance track (only if any performance input exists),
  │    one CharacterPerformanceClip per dialogue, placed at that dialogue's
  │    scheduled start_ms/duration_ms — same "single timeline source of
  │    truth" as audio/captions (design §18): nothing here re-derives timing
  │    independently.
  ▼
Project IR (project.vfp.json) + captions.srt (unchanged consumers)
  │
  ▼
Preview (FFmpeg)
  │  • Live2D characters: unaware of the new track, renders exactly as before.
  │  • `png_lipsync` characters (P0-1): `PreviewRequest::character_sprites`
  │    resolves each character's closed/half/open PNGs (outside the project
  │    IR, like `preview.font`); `videoforge-preview` overlays them onto the
  │    background before the caption `drawtext` chain, time-windowed by that
  │    character's own lip-sync curve.
  ▼
manifest.json
```

## Output layout (design §24)

```
generated/<slug>/
 ├── assets/
 │    ├── audio/<NNN>.wav                          (unchanged)
 │    ├── background/…                             (unchanged)
 │    └── character/<character_id>/
 │         └── lipsync-<NNN>.json                   ← NEW
 ├── project.vfp.json    (adds a character_performance track when used)
 ├── captions.srt                                   (unchanged)
 ├── manifest.json                                  (unchanged shape)
 └── preview.mp4 / preview skipped                  (unchanged — does not
                                                        yet reflect the
                                                        character at all)
```

`project.vfp.json` — the single source of truth (`CLAUDE.md`'s own
invariant) — is exactly where this data lives; nothing about the character
performance is derivable only from `manifest.json` or only from the audio
files, and none of it duplicates timing that the audio/caption tracks
already own.

## `CharacterPerformanceClip` shape

```json
{
  "type": "character_performance",
  "id": "character-performance-001",
  "start_ms": 0,
  "duration_ms": 3410,
  "character": "tsumugi",
  "expression": "smile",
  "motion": "idle",
  "lip_sync": "assets/character/tsumugi/lipsync-001.json",
  "transform": { "x": 0.2, "y": 0.8, "scale": 0.8, "rotation_deg": 0.0, "opacity": 1.0, "layer": 0, "fit": "contain" }
}
```

`lip_sync` is a `RelativeAssetPath`, same rules as every other asset
reference in the project IR (forward slashes, relative to the project
directory, no `..`) — resolved via `Clip::asset()`, so it participates in
`VideoProject::referenced_assets()` like any other clip's source file.

`transform` (P0-1) is the same `Transform` every `ImageClip`/`CharacterClip`
already carries — `#[serde(default)]`, so a `project.vfp.json` written before
P0-1 still loads (additive, no `SCHEMA_VERSION` bump). It comes from
`core::character::presentation_transform`, mapping the character manifest's
`presentation.{position,scale}` onto `x`/`scale`, with `y` fixed near the
bottom of the frame. Every performance clip for the same character carries
an identical `transform` — it describes the character, not the dialogue.

## `LipSyncTrack` shape (the referenced file's contents)

```json
{
  "interval_ms": 50,
  "samples": [
    { "t_ms": 0, "mouth_open": 0.0 },
    { "t_ms": 50, "mouth_open": 0.31 },
    { "t_ms": 100, "mouth_open": 0.82 }
  ]
}
```

Deterministic and derived purely from the dialogue's own WAV — see
`docs/character-system.md`'s "Lip sync" section for the amplitude method and
why phoneme analysis is explicitly out of scope for P0 (design §10, §30).

## Phase 1: what plugs in next, and where (Live2D only)

`png_lipsync` is already fully plugged in (P0-1, see
`docs/character-system.md`'s "3-state PNG rendering"); what follows is
specific to the still-unimplemented Live2D frame-rendering path.

Per `docs/live2d-renderer-decision.md`, a `CharacterRenderer` trait
(mirroring `TtsEngine`/`PreviewRenderer`) will consume exactly the data this
pipeline already produces — a character id + model reference (from the
character manifest) + a `CharacterPerformanceClip` (expression, motion,
`lip_sync` curve) — and emit a transparent RGBA frame sequence under
`assets/character/<id>/frames/`. `videoforge-preview`'s existing FFmpeg
overlay step then composites those frames over the background, alongside
the existing audio/caption tracks, exactly as design §16 lays out:

```
Background → Character RGBA frames → Subtitle → Audio → FFmpeg → preview.mp4
```

Nothing in this P0 slice needs to change for that to plug in: the
`character_performance` track is already the renderer's complete input, and
the renderer is a new leaf crate implementing a new core trait — `core`
itself stays untouched, matching every other engine/exporter seam in this
codebase (`CLAUDE.md`'s "Dependency direction" table).
