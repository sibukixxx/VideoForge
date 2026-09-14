use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_videoforge"))
}

fn run(cwd: &Path, args: &[&str]) -> std::process::Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("VIDEOFORGE_MARP", "/definitely/not/marp")
        .output()
        .expect("run videoforge")
}

#[test]
fn prompt_is_available_without_an_llm_api_or_marp() {
    let dir = tempfile::tempdir().unwrap();
    videoforge_core::init::init(dir.path(), Some("presentation")).unwrap();
    let output = run(
        dir.path(),
        &["presentation", "prompt", "scripts/sample.md"],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Marp Markdownだけ"));
    assert!(stdout.contains("VideoForgeのサンプル台本です"));
}

#[test]
fn validate_reports_a_typed_missing_marp_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    videoforge_core::init::init(dir.path(), Some("presentation")).unwrap();
    std::fs::write(
        dir.path().join("presentation.md"),
        "---\nmarp: true\ntheme: videoforge\nsize: 16:9\n---\n# test\n",
    )
    .unwrap();
    let output = run(
        dir.path(),
        &["--json", "presentation", "validate", "presentation.md"],
    );
    assert_eq!(output.status.code(), Some(2));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let diagnostics = value["validation"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.iter().any(|item| {
        item["code"] == "marp_unavailable"
            && item["path"] == "VIDEOFORGE_MARP"
            && item["remediation"].as_str().unwrap().contains("install Marp CLI")
    }));
}

#[test]
fn preflight_checks_marp_only_when_presentation_is_requested() {
    let dir = tempfile::tempdir().unwrap();
    videoforge_core::init::init(dir.path(), Some("presentation")).unwrap();
    std::fs::write(
        dir.path().join("presentation.md"),
        "---\nmarp: true\ntheme: videoforge\nsize: 16:9\n---\n# one\n---\n# two\n---\n# three\n",
    )
    .unwrap();

    let without = run(
        dir.path(),
        &["--json", "preflight", "scripts/sample.md", "--fake-tts"],
    );
    let without_json: serde_json::Value = serde_json::from_slice(&without.stdout).unwrap();
    assert!(!without_json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "marp_unavailable"));

    let with = run(
        dir.path(),
        &[
            "--json",
            "preflight",
            "scripts/sample.md",
            "--presentation",
            "presentation.md",
            "--fake-tts",
        ],
    );
    assert_eq!(with.status.code(), Some(2));
    let with_json: serde_json::Value = serde_json::from_slice(&with.stdout).unwrap();
    assert!(with_json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["code"] == "marp_unavailable"));
}
