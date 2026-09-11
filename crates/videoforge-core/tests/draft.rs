use videoforge_core::draft::{self, Brief, Draft};
use videoforge_core::{init, Workspace};

#[test]
fn grounded_draft_and_failure_cases() {
    let dir = tempfile::tempdir().unwrap();
    init::init(dir.path(), Some("draft-test")).unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    let cfg = ws.load_config().unwrap();
    let mut brief: Brief = serde_json::from_str(include_str!("../../../fixtures/draft/brief.json")).unwrap();
    let mut candidate: Draft = serde_json::from_str(include_str!("../../../fixtures/draft/response.json")).unwrap();
    let first = draft::check(&brief, &candidate, &cfg, &ws).unwrap();
    assert!(first.structurally_valid, "{:?}", first.errors);
    assert_eq!(first.dialogue_count, 2);
    assert!(draft::prompt(&brief, &cfg).unwrap().contains("UNTRUSTED_INPUT_JSON"));

    candidate.dialogues[0].evidence[0].quote = "invented".into();
    assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
    candidate.dialogues[0].evidence[0].quote = "台本を確認してから音声を作ります。".into();
    candidate.unresolved.push("sources disagree".into());
    assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
    candidate.unresolved.clear();
    candidate.material_requests.push("missing diagram".into());
    assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
    candidate.material_requests.clear();

    for text in ["@image assets/missing.png", "# hidden caption", "other:", "hello\n@se missing.wav"] {
        candidate.dialogues[0].text = text.into();
        assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
    }
    candidate.dialogues[0].text = "台本を確認してから音声を作ります。".into();
    candidate.dialogues[0].speaker = "unknown".into();
    assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
    candidate.dialogues[0].speaker = "reimu".into();
    brief.sources[0].text.push_str("追加資料。");
    assert_ne!(first.review_hash, draft::check(&brief, &candidate, &cfg, &ws).unwrap().review_hash);
    candidate.dialogues[0].evidence.clear();
    assert!(!draft::check(&brief, &candidate, &cfg, &ws).unwrap().structurally_valid);
}
