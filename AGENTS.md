# VideoForge

Local video-production compiler: Markdown script → canonical `VideoProject` IR → audio/subtitles/timeline → preview and YMM4 handoff.

## Source of truth
- Architecture: `docs/design/mvp-v0.2-cross-platform.md`
- IR contract: `docs/video-project.md`
- Testing notes: `docs/testing/`

## Commands
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all --check`
- Offline smoke: `cargo run -p videoforge-cli -- generate scripts/sample.md --fake-tts --no-preview`
- Desktop frontend: `cd apps/desktop && pnpm install && pnpm build`
- Desktop Rust is a separate workspace: `cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings && cargo test`

## Shared rules
- Dependency direction stays `cli → core ← {voicevox, preview, export-ymm4}`; core must not depend on Tauri, Windows APIs, or YMM4 schema.
- `project.vfp.json` is the generated project source of truth. Do not hand-edit `generated/`; change script/config and regenerate.
- Generated writes remain atomic; never write final output directly before successful generation.
- Config-supplied paths must remain inside the workspace.
- Keep desktop Tauri outside the root Cargo workspace so root tests remain platform-portable.
- VideoForge itself does not perform trend research or prose generation; those belong to agent/workflow Skills.

## Change-dependent checks
- Rust/core changes: test + clippy + fmt above.
- FFmpeg/VOICEVOX behavior: run the matching `docs/testing/` integration path when the dependency is available; report when unavailable.
- Desktop changes: validate frontend and the separate Tauri workspace.

## Done
- Applicable checks pass and the IR/dependency invariants remain true.
- Generated artifacts come from regeneration, not manual edits.
- Platform/hardware checks that could not run are named explicitly.
