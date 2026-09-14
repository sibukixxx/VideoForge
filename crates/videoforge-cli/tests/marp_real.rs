//! Real Marp smoke test. It follows the repository's external-binary
//! convention: skip when Marp is absent, fail when an installed Marp cannot
//! render (for example because its supported browser is missing).

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_videoforge"))
}

#[test]
fn renders_real_marp_png_sequence_when_available() {
    let marp = std::env::var_os("VIDEOFORGE_MARP").unwrap_or_else(|| "marp".into());
    if Command::new(&marp).arg("--version").output().is_err() {
        eprintln!("skipping real Marp test: executable not found");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    videoforge_core::init::init(dir.path(), Some("marp-real")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/presentation/presentation.md"),
        dir.path().join("deck.md"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("assets/image")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/presentation/presentation-flow.svg"),
        dir.path().join("assets/image/presentation-flow.svg"),
    )
    .unwrap();

    let output = Command::new(bin())
        .args(["presentation", "render", "deck.md"])
        .current_dir(dir.path())
        .env("VIDEOFORGE_MARP", &marp)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let generated = dir.path().join("generated/deck/presentation");
    for index in 1..=6 {
        assert!(generated.join(format!("slide-{index:03}.png")).is_file());
    }
}
