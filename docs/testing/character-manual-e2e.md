# Manual E2E: real VOICEVOX character + a real Live2D model or PNG sprites

Tracks the character system (`docs/character-system.md`): both the Live2D
half (data-only, P0) and the `png_lipsync` half (composited into
`preview.mp4`, P0-1). `FakeTtsEngine` plus `fixtures/character/mock-character/`
/ `fixtures/character/mock-png-character/` prove the plumbing fully offline
(`cargo test -p videoforge-core character:: generate:: validate:: doctor:: assets::`,
`cargo test -p videoforge-preview`, `cargo test -p videoforge-cli --test cli`);
this procedure proves the real thing end to end. Two independent parts:

* **Live2D** ("Manual part A" below) — a real VOICEVOX voice + a real Live2D
  model file, up to where that path stops (no frame rendering yet):

  ```text
  character.md → VOICEVOX (named speaker/style) → WAV
               → lip-sync amplitude curve → character_performance track
               → project.vfp.json
  ```

* **`png_lipsync` (P0-1)** ("Manual part B" below) — the full P0-5
  acceptance scenario: two real VOICEVOX speakers (e.g. ずんだもん /
  四国めたん), each with real closed/half/open PNG art, actually composited
  into a real FFmpeg `preview.mp4`:

  ```text
  Markdown → Character resolution → VOICEVOX → WAV → Lip Sync
           → PNG Character (closed/half/open) → Subtitle → FFmpeg → preview.mp4
  ```

Part A stops at `project.vfp.json`; there is **no frame rendering or preview
integration for Live2D yet** — see `docs/live2d-renderer-decision.md`. Part
B is the one that produces an actual video to watch.

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
# offline, no VOICEVOX/FFmpeg/real art needed — proves the pipeline shape
cargo test -p videoforge-character
cargo test -p videoforge-core character:: validate:: doctor:: lipsync:: wav:: generate:: assets::
cargo test -p videoforge-preview   # command-builder-level FFmpeg overlay graph, P0-1
cargo test -p videoforge-timeline
cargo test -p videoforge-project
cargo test -p videoforge-cli --test cli
```

All of the above run in the offline `cargo test --workspace` and must stay
green regardless of what the manual steps below find — they exercise the
same code paths against `fixtures/character/mock-character/` (Live2D) and
`fixtures/character/mock-png-character/` (`png_lipsync`, two synthetic
left/right characters) instead of a real character/model/art.

## Manual part A: Live2D — preparing your own character manifest

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

Record the result in "Results log A" below.

## Manual part B: `png_lipsync` — two real speakers, real preview.mp4 (P0-5)

Needs, in addition to the prerequisites above: FFmpeg on `PATH` (or
`VIDEOFORGE_FFMPEG`), and closed/half/open transparent PNGs for **two**
VOICEVOX speakers you have the right to use — e.g. ずんだもん and
四国めたん, if you have art for both; any two distinct speakers work. **Do
not add that artwork to this repository** (same rule as Live2D models).

1. Find both speakers' exact names with `videoforge speakers`.

2. Character manifest (again, outside this repo), two entries:

   ```yaml
   characters:
     - id: speaker-a
       display_name: "<display name A>"
       voice:
         provider: voicevox
         speaker: "<exact VOICEVOX speaker name A>"
         style: "<exact VOICEVOX style name A>"
       model:
         type: png_lipsync
         closed: /absolute/path/to/a/closed.png
         half: /absolute/path/to/a/half.png
         open: /absolute/path/to/a/open.png
       presentation:
         position: left
         scale: 0.8

     - id: speaker-b
       display_name: "<display name B>"
       voice:
         provider: voicevox
         speaker: "<exact VOICEVOX speaker name B>"
         style: "<exact VOICEVOX style name B>"
       model:
         type: png_lipsync
         closed: /absolute/path/to/b/closed.png
         half: /absolute/path/to/b/half.png
         open: /absolute/path/to/b/open.png
       presentation:
         position: right
         scale: 0.8
   ```

3. `videoforge character inspect <manifest>` — confirms both entries parse
   and both sprite sets are found, valid PNGs, and carry an alpha channel.
   A `png_sprite_invalid` naming "no alpha channel" means the PNG was
   exported without transparency (flatten-to-white during export is the
   usual cause) — re-export as RGBA.

4. `videoforge character validate <manifest> --endpoint http://127.0.0.1:50021`
   (or your VOICEVOX endpoint) — confirms both voices resolve.

5. Workspace: `videoforge init demo && cd demo`, then `videoforge.yaml`:

   ```yaml
   character_manifest: /absolute/path/to/characters.yaml

   speakers:
     speaker-a:
       character_id: speaker-a
     speaker-b:
       character_id: speaker-b
   ```

6. `videoforge doctor` — both `Character \`speaker-a\`` and
   `` Character `speaker-b` `` checks must be `ok`. This is the P0-3
   preflight: an unresolvable voice or a broken sprite must fail **here**,
   before any TTS work starts, and must never silently fall back to a
   different VOICEVOX speaker.

7. `scripts/dialogue.md`:

   ```
   speaker-a:
   Aが話しています。

   speaker-b:
   Bが話しています。

   speaker-a:
   もう一度Aです。
   ```

8. `videoforge generate scripts/dialogue.md` (real VOICEVOX, real FFmpeg —
   no `--fake-tts`/`--no-preview`).

9. Watch `generated/dialogue/preview.mp4` and confirm the P0-5 acceptance
   scenario directly:
   - While A's line plays, A's mouth visibly moves (opens/half-opens with
     the audio) and B is static with a closed mouth.
   - While B's line plays, the reverse: B's mouth moves, A is static/closed.
   - Neither character is ever cut off by the frame edge, and neither is
     ever hidden behind the caption text.
   - Re-run step 8 with the exact same script/manifest/assets: the new
     `preview.mp4` should look identical (same structure/timing) to the
     first — reproducibility (design goal of P0-5). Checking
     `stderr` for `→ Reusing preview.mp4 (cached)` confirms P0-4's cache
     also hit (nothing that affects the preview changed).

10. `videoforge assets generated/dialogue/project.vfp.json` — both
    characters should list `ok`/all-files-present with a `sha256:` hash
    each (P0-2).

Record the result in "Results log B" below.

## Results log A (Live2D)

| Date | OS | VOICEVOX | Character/voice | Live2D model | VideoForge commit | Steps 1–7 | Notes |
| ---- | -- | -------- | ---------------- | ------------- | ----------------- | --------- | ----- |
| _(not yet run)_ | | | | | | | |

## Results log B (`png_lipsync`, P0-5 acceptance)

| Date | OS | VOICEVOX | Speaker A / Speaker B | FFmpeg version | VideoForge commit | Steps 1–10 | Notes |
| ---- | -- | -------- | ---------------------- | --------------- | ----------------- | ----------- | ----- |
| _(not yet run)_ | | | | | | | |
