# Speaker profile safety

Issue #46 adds a fail-closed identity check before real VOICEVOX generation.

## Why

A script-facing name must not silently produce another character's voice. A historical configuration such as:

```yaml
speakers:
  reimu:
    aliases: [霊夢]
    voice:
      speaker_id: 2
```

is unsafe when the connected VOICEVOX reports style id `2` as `四国めたん / ノーマル`. VideoForge now resolves the numeric id against the live engine before synthesis and refuses to generate under the wrong visible identity.

## Generation rules

For a real TTS engine, generation stops before synthesis/output when any of these is true:

- a configured numeric style id does not exist in the connected engine;
- a legacy numeric-only speaker has aliases, but none equals the VOICEVOX speaker name that owns that style id;
- two distinct canonical speaker keys resolve to the same VOICEVOX style id;
- a character-linked named speaker/style cannot be resolved;
- the linked character manifest or character id is invalid.

Character-linked profiles are authoritative by `voice.speaker` + `voice.style` names from the character manifest. Their resolved numeric id is written only to the in-memory config used by the existing pipeline.

## Legacy compatibility

A numeric-only profile without aliases cannot be semantically compared to a person/character name, so it is allowed but remains explicitly `legacy_unpinned_voice` in `SpeakerProfileReport`. A numeric-only profile whose alias already equals the engine speaker name is also allowed, but should still be migrated to a named character profile when visual identity is important.

The synthetic `fake` TTS engine is exempt from the runtime identity guard because it does not model a real VOICEVOX installation; this preserves deterministic offline tests. Character-linked fake fixtures are still resolved and validated normally.

## Recommended migration

Prefer a reusable character manifest:

```yaml
# videoforge.yaml
character_manifest: characters.yaml
speakers:
  zundamon:
    aliases: [ずんだもん]
    character_id: zundamon
```

```yaml
# characters.yaml
characters:
  - id: zundamon
    display_name: ずんだもん
    voice:
      provider: voicevox
      speaker: ずんだもん
      style: ノーマル
    model:
      type: png_lipsync
      closed: ./characters/zundamon/closed.png
      half: ./characters/zundamon/half.png
      open: ./characters/zundamon/open.png
```

This pins script name, VOICEVOX identity, and character assets to one profile instead of relying on an opaque numeric style id.
