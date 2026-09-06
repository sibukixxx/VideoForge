# Manual E2E: real YMM4 acceptance test

Tracks: [P0] "実YMM4でのE2E Acceptance Testを実施・文書化する" (issue #5).

Unit tests cover the template-patch logic (`videoforge-export-ymm4`), but the
MVP is only proven when a generated `.ymmp` actually opens, edits, and
re-saves in a real copy of YukkuriMovieMaker4 on Windows. This requires a
Windows machine with YMM4 installed and cannot be exercised from this
repository's CI (Ubuntu/macOS/Windows runners do not have YMM4 available),
so it stays a manual procedure. Whoever runs it should fill in the results
table at the bottom and commit it back to this file.

## Prerequisites

- Windows 10/11 with [YukkuriMovieMaker4](https://manjubox.net/ymm4/) installed.
- VOICEVOX Engine running locally (or use `--fake-tts` to skip real synthesis
  and focus purely on the exporter/YMM4 interop).
- A VideoForge workspace (`videoforge init`) with `scripts/sample.md`.

## Procedure

1. **Author a real template.** Open YMM4, create a new project, and add:
   - one audio item, 備考 (Remark) set to `VF_PROTO_AUDIO`
   - one caption/text item, Remark set to `VF_PROTO_CAPTION`
   - (optional) one 立ち絵/character item, Remark set to `VF_PROTO_CHARACTER`

   Save as `templates/ymm4/default.ymmp` in the workspace (see
   `fixtures/templates/ymm4/README.md` for the prototype contract).
2. **Generate a project.**
   ```
   videoforge generate scripts/sample.md
   ```
3. **Export to YMM4.**
   ```
   videoforge export ymm4 generated/sample/project.vfp.json
   ```
4. **Open in YMM4.** Launch the produced `.ymmp` (or pass `--open` in step 3).
5. **Verify audio clips** are present, point at the correct `.wav` files, and
   sit at the expected timeline positions.
6. **Verify captions** show the correct text at the correct timing.
7. **Verify timing** overall matches `captions.srt` (no drift, no overlap).
8. **Edit a caption** in YMM4 and confirm the edit sticks.
9. **Move an audio clip** in YMM4, save, close, and reopen the project to
   confirm YMM4's own save round-trip still works (i.e. VideoForge's patch
   didn't corrupt fields YMM4 depends on).

## `force_non_windows` note

`Ymm4Exporter::force_non_windows` (and the CLI's hidden `--force` on
`export ymm4`) is not test-only dead weight: it's what lets the
template-patch logic itself (prototype lookup, field patching, Windows-path
materialization string-building) run in CI on Linux/macOS runners via
`videoforge-cli/tests/cli.rs`, without needing a real Windows host for every
commit. It deliberately cannot make the *output* valid for YMM4 on a
non-Windows host (asset paths are materialized as Windows paths that don't
exist on the CI runner) — only this manual procedure, on real Windows with
real YMM4, can confirm that. The flag stays `hide = true` in the CLI so it
doesn't show up as a supported production workflow in `--help`.

## Results log

| Date | YMM4 version | VideoForge commit | Result | Notes |
| ---- | ------------ | ------------------ | ------ | ----- |
| _(not yet run)_ | | | | |
