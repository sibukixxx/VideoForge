# Manual E2E: real VOICEVOX character + a real Live2D model

Tracks the character/Live2D vertical slice (`docs/character-system.md`).
`FakeTtsEngine` plus `fixtures/character/mock-character/` prove the plumbing
fully offline (`cargo test -p videoforge-core character:: generate:: validate::`,
`cargo test -p videoforge-cli --test cli`); this procedure proves the real
thing — a real VOICEVOX character voice, a real Live2D model file — end to
end, up to where P0 stops:

```text
character.md → VOICEVOX (named speaker/style) → WAV
             → lip-sync amplitude curve → character_performance track
             → project.vfp.json
```

There is **no frame rendering or preview integration to check yet** — see
`docs/live2d-renderer-decision.md`. This procedure validates voice
resolution, model metadata validation, and the lip-sync/timeline data; it
does not validate anything visual.

## Prerequisites

- [VOICEVOX](https://voicevox.hiroshiba.jp/) running and answering at
  `http://127.0.0.1:50021` (see `docs/testing/voicevox-manual-e2e.md` for
  setup detail).
- A VOICEVOX character voice you have the right to use for this test (see
  `docs/character-licensing.md` — this procedure does not resolve any
  licensing question, it only tells you how to point VideoForge at whatever
  voice you've already confirmed you can use).
- A Live2D model file (`*.model3.json` plus its referenced textures/motions/
  expressions) for that same character, obtained through whatever legitimate
  channel applies to it. **Do not add it to this repository.** Keep it
  anywhere on your local disk.
- A release or debug build of the CLI: `cargo build -p videoforge-cli`.

## Automated part

```bash
# offline, no VOICEVOX/Live2D needed — proves the pipeline shape
cargo test -p videoforge-character
cargo test -p videoforge-core character:: validate:: lipsync:: wav:: generate::
cargo test -p videoforge-timeline
cargo test -p videoforge-project
cargo test -p videoforge-cli --test cli
```

All of the above run in the offline `cargo test --workspace` and must stay
green regardless of what the manual steps below find — they exercise the
same code paths against `fixtures/character/mock-character/` instead of a
real character/model.

## Manual part: preparing your own character manifest

1. Create a character manifest anywhere on disk (**not inside this
   repository**), e.g. `~/videoforge-characters/characters.yaml`:

   ```yaml
   characters:
     - id: my-character
       display_name: "<display name>"
       voice:
         provider: voicevox
         speaker: "<exact VOICEVOX speaker name>"
         style: "<exact VOICEVOX style name>"
       model:
         type: live2d
         path: /absolute/path/to/your/model.model3.json
   ```

   Find the exact speaker/style names with `videoforge speakers` (prints
   every name VOICEVOX currently offers) — they must match byte-for-byte.

2. `videoforge character inspect ~/videoforge-characters/characters.yaml` —
   confirms the manifest parses and lists the model's expressions/motions as
   read from your real `model3.json`. If this fails on `live2d_model_invalid`,
   the model file is not what VideoForge expects (verify it's a top-level
   `model3.json`, not the whole model directory).

3. `videoforge character validate ~/videoforge-characters/characters.yaml` —
   confirms VOICEVOX has a speaker/style matching what you wrote, and prints
   the resolved numeric `speaker_id`. A `voicevox_speaker_not_found` error
   here means the speaker/style name doesn't match `videoforge speakers`
   exactly (check whitespace, full-width vs. half-width characters).

4. In a fresh workspace (`videoforge init demo && cd demo`), point it at
   your manifest and link a speaker to your character in `videoforge.yaml`:

   ```yaml
   character_manifest: /absolute/path/to/characters.yaml

   speakers:
     my-character:
       character_id: my-character
   ```

   (Remove or keep the `reimu`/`marisa` speakers `init` wrote — they don't
   need a character and are unaffected either way.)

5. Write `scripts/character.md`:

   ```
   my-character[expression=<one from step 2>, motion=<one from step 2>]:
   こんにちは。
   ```

   Use an expression/motion name from step 2's output. A name not in that
   list is a validation error by design (design §12) — try one on purpose
   to confirm `videoforge validate scripts/character.md` reports it as an
   error naming the character and the known list, not a crash.

6. `videoforge generate scripts/character.md` (add `--no-preview` if FFmpeg
   isn't set up; it's irrelevant to this feature either way).

7. Check the output:
   - `generated/character/assets/audio/001.wav` — play it, confirm it's
     your character's real voice, not silence (a `--fake-tts` run would
     have produced silence; this step must be a real VOICEVOX run).
   - `generated/character/assets/character/my-character/lipsync-001.json` —
     open it; `samples` should track the WAV's loudness (near-zero
     `mouth_open` during silence at the start/end of the line, higher during
     speech).
   - `generated/character/project.vfp.json` — find the
     `character_performance` track; its one clip's `character`,
     `expression`, and `motion` should match what you wrote in step 5, and
     `start_ms`/`duration_ms` should match the `audio` track's clip for the
     same dialogue exactly (single timeline source of truth, design §18).

Record the result below.

## Results log

| Date | OS | VOICEVOX | Character/voice | Live2D model | VideoForge commit | Steps 1–7 | Notes |
| ---- | -- | -------- | ---------------- | ------------- | ----------------- | --------- | ----- |
| _(not yet run)_ | | | | | | | |
