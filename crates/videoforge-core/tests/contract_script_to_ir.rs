//! Contract Test B (P0 §15): script → VideoProject IR, end to end.
//!
//! Runs the real pipeline (`parse → validate → TTS → timeline → project IR`)
//! over a small, fixed script with the deterministic `FakeTtsEngine`, then
//! compares the resulting `project.vfp.json` byte-for-byte against a
//! checked-in golden fixture. This is a change detector across the whole
//! `videoforge-script` → `videoforge-timeline` → `videoforge-project`
//! boundary, distinct from `videoforge-project`'s own serialization contract
//! (Contract Test A), which never touches the script parser or timeline
//! builder at all.
//!
//! `FakeTtsEngine::duration_for` derives audio duration purely from
//! character count, so the same script always produces the same
//! `start_ms`/`duration_ms` values — no VOICEVOX or FFmpeg needed. `source
//! .generator_version` tracks the real crate version (`CARGO_PKG_VERSION`):
//! a version bump is expected to change it, which shows up here as a
//! deliberate, reviewable diff against the fixture rather than flakiness.

use std::sync::Arc;

use videoforge_core::tts::FakeTtsEngine;
use videoforge_core::{generate, init, GenerateDeps, GenerateOptions, Workspace};

const EXPECTED: &str = include_str!("../../../fixtures/projects/ir-contract-basic.vfp.json");

#[tokio::test]
async fn ir_contract_script_generates_the_frozen_project_ir() {
    let dir = tempfile::tempdir().unwrap();
    init::init(dir.path(), Some("ir-contract")).unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let script_path = ws.scripts_dir().join("ir-contract-basic.md");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/scripts/ir-contract-basic.md"
        ),
        &script_path,
    )
    .unwrap();

    let deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
    let out = generate(&ws, &script_path, GenerateOptions::default(), deps)
        .await
        .unwrap();

    let actual = out.project.to_json().unwrap();
    assert_eq!(
        actual, EXPECTED,
        "script → IR shape changed — if this is an intentional, additive \
         change (or a deliberate crate version bump touching \
         source.generator_version), regenerate \
         fixtures/projects/ir-contract-basic.vfp.json from this pipeline's \
         actual output; if not, something in the script parser, timeline \
         builder, or project IR just changed unexpectedly"
    );
}
