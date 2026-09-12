use std::process::Command;

#[test]
fn review_export_refuses_stale_hash_and_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    videoforge_core::init::init(dir.path(), Some("draft")).unwrap();
    std::fs::write(
        dir.path().join("brief.json"),
        include_str!("../../../fixtures/draft/brief.json"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("response.json"),
        include_str!("../../../fixtures/draft/response.json"),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_videoforge"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["draft", "prompt", "brief.json"]).status.success());
    let check = run(&["draft", "check", "brief.json", "response.json"]);
    assert!(check.status.success());
    let report: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    let hash = report["review_hash"].as_str().unwrap();
    let export = |hash: &str, out: &str| {
        run(&[
            "draft",
            "export",
            "brief.json",
            "response.json",
            "--reviewed-hash",
            hash,
            "--reviewer",
            "tester",
            "--out",
            out,
        ])
    };
    assert_eq!(export("stale", "scripts/draft.md").status.code(), Some(2));
    assert!(!dir.path().join("scripts/draft.md").exists());
    assert!(!export(hash, "scripts/../../escape.md").status.success());
    assert!(export(hash, "scripts/draft.md").status.success());
    assert!(run(&["validate", "scripts/draft.md"]).status.success());
    let original = std::fs::read(dir.path().join("scripts/draft.md")).unwrap();
    assert!(!export(hash, "scripts/draft.md").status.success());
    assert_eq!(
        original,
        std::fs::read(dir.path().join("scripts/draft.md")).unwrap()
    );
    let brief_path = dir.path().join("brief.json");
    let changed = std::fs::read_to_string(&brief_path)
        .unwrap()
        .replace("初心者", "経験者");
    std::fs::write(brief_path, changed).unwrap();
    assert_eq!(export(hash, "scripts/changed.md").status.code(), Some(2));
}
