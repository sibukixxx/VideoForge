# Character video pipeline

How the character/Live2D feature (`docs/character-system.md`) fits into
VideoForge's existing generation pipeline (`core::generate::generate`,
documented in `CLAUDE.md`). Two character paths are deliberately separate:

- static transparent PNG stand-ins (`@character`) are composited by FFmpeg;
- Live2D models produce performance/lip-sync data, but frame rendering remains Phase 1
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
  • static CharacterClip PNGs are overlaid by start/duration and layer
  • alpha, position, scale, rotation and opacity are preserved
  • captions render above character PNGs
  • CharacterPerformanceClip / Live2D frames are not rendered yet
  │
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
 └── preview.mp4 / preview skipped                  (static PNG stand-ins
                                                       are visible; Live2D
                                                       output is not yet rendered)
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
  "lip_sync": "assets/character/tsumugi/lipsync-001.json"
}
```

`lip_sync` is a `RelativeAssetPath`, same rules as every other asset
reference in the project IR (forward slashes, relative to the project
directory, no `..`) — resolved via `Clip::asset()`, so it participates in
`VideoProject::referenced_assets()` like any other clip's source file.

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

## Phase 1: what plugs in next, and where

Static PNG composition is now the reference FFmpeg overlay path. Per
`docs/live2d-renderer-decision.md`, a future `CharacterRenderer` trait
(mirroring `TtsEngine`/`PreviewRenderer`) will consume exactly the data this
pipeline already produces — a character id + model reference (from the
character manifest) + a `CharacterPerformanceClip` (expression, motion,
`lip_sync` curve) — and emit a transparent RGBA frame sequence under
`assets/character/<id>/frames/`. `videoforge-preview`'s FFmpeg overlay
step can then composite those frames over the background, alongside
the existing audio/caption tracks, exactly as design §16 lays out:

```
Background → Character RGBA frames → Subtitle → Audio → FFmpeg → preview.mp4
```

Nothing in this P0 slice needs to change for that to plug in: the
`character_performance` track is already the renderer's complete input, and
the renderer is a new leaf crate implementing a new core trait — `core`
itself stays untouched, matching every other engine/exporter seam in this
codebase (`CLAUDE.md`'s "Dependency direction" table).
