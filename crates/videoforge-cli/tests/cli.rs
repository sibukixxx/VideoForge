//! End-to-end CLI test: init → validate → generate (fake TTS) → bundle →
//! export (forced) using the repository fixtures. Runs on every platform
//! without VOICEVOX or FFmpeg.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_videoforge"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn run(cwd: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("VIDEOFORGE_CACHE_DIR", cwd.join(".cache"))
        .env("VIDEOFORGE_FFMPEG", "/definitely/not/ffmpeg")
        .output()
        .expect("run videoforge");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn full_pipeline_offline() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("demo");

    let (code, stdout, stderr) = run(tmp.path(), &["init", "demo"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(ws.join("videoforge.yaml").is_file());
    assert!(ws.join("AGENTS.md").is_file());

    // fixture script + template into the workspace
    std::fs::copy(
        repo_root().join("fixtures/scripts/sample.md"),
        ws.join("scripts/001-ai-news.md"),
    )
    .unwrap();
    std::fs::copy(
        repo_root().join("fixtures/templates/ymm4/default.ymmp"),
        ws.join("templates/ymm4/default.ymmp"),
    )
    .unwrap();

    let (code, stdout, _) = run(&ws, &["validate", "scripts/001-ai-news.md"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("Dialogues: 3"));

    let (code, stdout, stderr) = run(&ws, &["doctor", "--fake-tts", "--json"]);
    let doctor: serde_json::Value = serde_json::from_str(&stdout).expect(&stderr);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert_eq!(doctor["capabilities"]["voicevox_available"], true);
    assert_eq!(doctor["capabilities"]["ffmpeg_available"], false);
    assert_eq!(doctor["capabilities"]["can_export_ymm4"], cfg!(windows));

    let (code, stdout, stderr) = run(
        &ws,
        &["generate", "scripts/001-ai-news.md", "--fake-tts", "--json"],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let gen: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(gen["slug"], "001-ai-news");
    let out_dir = ws.join("generated").join("001-ai-news");
    for f in [
        "project.vfp.json",
        "manifest.json",
        "captions.srt",
        "source.md",
        "assets/audio/001.wav",
        "assets/audio/003.wav",
    ] {
        assert!(out_dir.join(f).is_file(), "{f}");
    }
    assert!(!out_dir.join("preview.mp4").exists());
    assert!(!ws.join(".generated-tmp").exists());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["source"], "scripts/001-ai-news.md");
    assert_eq!(manifest["dialogues"], 3);
    assert!(manifest["preview"].is_null());

    // second run hits the cache
    let (code, _, stderr) = run(
        &ws,
        &[
            "generate",
            "scripts/001-ai-news.md",
            "--fake-tts",
            "--no-preview",
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("(cached)"), "{stderr}");

    // bundle
    let (code, stdout, stderr) = run(
        &ws,
        &[
            "bundle",
            "ymm4",
            "generated/001-ai-news/project.vfp.json",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let bundle: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let bundle_dir = PathBuf::from(bundle["dir"].as_str().unwrap());
    assert!(bundle_dir.join("template.ymmp").is_file());
    assert!(bundle_dir.join("assets/audio/002.wav").is_file());
    assert!(bundle["zip"].as_str().map(Path::new).unwrap().is_file());

    // export without --force: platform-gated
    let (code, _, stderr) = run(
        &ws,
        &["export", "ymm4", "generated/001-ai-news/project.vfp.json"],
    );
    if cfg!(windows) {
        assert_eq!(code, 0, "{stderr}");
    } else {
        assert_eq!(code, 2);
        assert!(stderr.contains("requires Windows"), "{stderr}");
    }

    // forced export from the bundle (uses the sibling template.ymmp)
    let bundle_project = bundle_dir.join("project.vfp.json");
    let (code, stdout, stderr) = run(
        &ws,
        &[
            "export",
            "ymm4",
            bundle_project.to_str().unwrap(),
            "--force",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let exported: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let ymmp_path = PathBuf::from(exported["output"].as_str().unwrap());
    assert_eq!(ymmp_path, bundle_dir.join("ymm4").join("project.ymmp"));
    let ymmp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&ymmp_path).unwrap()).unwrap();
    let items = ymmp["Timelines"][0]["Items"].as_array().unwrap();
    // 3 audio + 3 captions, no prototypes left
    assert_eq!(items.len(), 6);
    assert!(items
        .iter()
        .all(|i| !i["Remark"].as_str().unwrap().starts_with("VF_PROTO_")));
    let audio: Vec<&serde_json::Value> = items
        .iter()
        .filter(|i| i["$type"].as_str().unwrap().contains("AudioItem"))
        .collect();
    assert_eq!(audio.len(), 3);
    assert!(audio[0]["FilePath"]
        .as_str()
        .unwrap()
        .ends_with(r"\assets\audio\001.wav"));
    let project: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("project.vfp.json")).unwrap())
            .unwrap();
    let start = project["tracks"][0]["clips"][1]["start_ms"]
        .as_u64()
        .unwrap();
    assert_eq!(
        audio[1]["Frame"].as_u64().unwrap(),
        (start as f64 * 30.0 / 1000.0).round() as u64
    );
    assert_eq!(ymmp["Version"], "4.0.0");
}

#[test]
fn validate_reports_unknown_speaker() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, _, _) = run(tmp.path(), &["init"]);
    assert_eq!(code, 0);
    std::fs::write(
        tmp.path().join("scripts/bad.md"),
        "霊夢:\nやあ\n\nアリス:\nだれ？\n",
    )
    .unwrap();
    let (code, stdout, _) = run(tmp.path(), &["validate", "scripts/bad.md"]);
    assert_eq!(code, 2);
    assert!(stdout.contains("unknown speaker `アリス`"), "{stdout}");
}

#[test]
fn outside_workspace_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, _, stderr) = run(tmp.path(), &["validate", "x.md"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("workspace not found"), "{stderr}");
}

#[test]
fn json_errors_carry_a_stable_machine_readable_code() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(tmp.path(), &["--json", "validate", "x.md"]);

    assert_eq!(code, 1, "{stderr}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["code"], "workspace_not_found");
}

#[test]
fn out_of_range_voice_parameters_are_rejected_before_synthesis() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, _, _) = run(tmp.path(), &["init"]);
    assert_eq!(code, 0);

    let config_path = tmp.path().join("videoforge.yaml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    std::fs::write(
        &config_path,
        config.replace("speed_scale: 1.05", "speed_scale: 12.0"),
    )
    .unwrap();

    let (code, stdout, _) = run(
        tmp.path(),
        &["--json", "generate", "scripts/sample.md", "--fake-tts"],
    );

    assert_eq!(code, 1, "{stdout}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    assert_eq!(payload["code"], "invalid_config");
    assert_eq!(
        payload["error"].as_str().unwrap(),
        format!(
            "invalid config {}: speakers.reimu.voice.speed_scale \
             must be between 0.5 and 2 (got 12)",
            // the CLI reports the canonicalized workspace path
            config_path.canonicalize().unwrap().display()
        )
    );
    assert!(
        !tmp.path().join("generated/sample").exists(),
        "nothing may be generated from an invalid config"
    );
}

#[test]
fn directives_become_clips_and_missing_assets_are_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("visual");
    let (code, stdout, stderr) = run(tmp.path(), &["init", "visual"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    for asset in ["assets/image/chart.png", "assets/bgm/main.mp3"] {
        std::fs::write(ws.join(asset), b"not really media").unwrap();
    }
    std::fs::write(
        ws.join("scripts/visual.md"),
        "@bgm assets/bgm/main.mp3[volume=0.6, loop=true]\n@image assets/image/chart.png[role=diagram]\n@transition fade[duration_ms=300]\n霊夢:\nこのグラフを見てください。\n\n@se assets/se/missing.wav\n魔理沙:\nなるほどな。\n",
    )
    .unwrap();

    let (code, stdout, _) = run(&ws, &["validate", "scripts/visual.md"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        stdout.contains("Directives: 3 (1 with a missing asset)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("`@se` asset `assets/se/missing.wav` not found; the clip is skipped"),
        "{stdout}"
    );

    let (code, stdout, stderr) = run(
        &ws,
        &[
            "generate",
            "scripts/visual.md",
            "--fake-tts",
            "--no-preview",
            "--json",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let out_dir = ws.join("generated").join("visual");
    assert!(out_dir.join("assets/image/chart.png").is_file());
    assert!(out_dir.join("assets/bgm/main.mp3").is_file());

    let project: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("project.vfp.json")).unwrap())
            .unwrap();
    let tracks: Vec<&str> = project["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["kind"].as_str().unwrap())
        .collect();
    assert_eq!(tracks, vec!["audio", "caption", "image", "bgm"]);
    let second_start = project["tracks"][0]["clips"][1]["start_ms"]
        .as_u64()
        .unwrap();
    let total = project["tracks"][0]["clips"][1]["start_ms"]
        .as_u64()
        .unwrap()
        + project["tracks"][0]["clips"][1]["duration_ms"]
            .as_u64()
            .unwrap();
    assert!(second_start > 0);
    let image = &project["tracks"][2]["clips"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["source"], "assets/image/chart.png");
    assert_eq!(image["start_ms"], 0);
    assert_eq!(
        image["duration_ms"], total,
        "image runs to the end: no later @image"
    );
    assert_eq!(image["presentation"]["role"], "diagram");
    assert_eq!(image["presentation"]["intent"], "fade");
    assert_eq!(image["presentation"]["intent_duration_ms"], 300);
    let bgm = &project["tracks"][3]["clips"][0];
    assert_eq!(bgm["type"], "bgm");
    assert_eq!(bgm["volume"], 0.6);
    assert_eq!(bgm["looping"], true);
    assert_eq!(bgm["duration_ms"], total);

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("manifest.json")).unwrap())
            .unwrap();
    assert!(
        manifest["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap()
                == "line 7: `@se` asset `assets/se/missing.wav` not found; the clip is skipped"),
        "{manifest}"
    );
}

fn mock_character_dir() -> PathBuf {
    repo_root().join("fixtures/character/mock-character")
}

#[test]
fn character_inspect_reports_voice_and_model() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(
        tmp.path(),
        &[
            "character",
            "inspect",
            mock_character_dir().join("manifest.yaml").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("Mock Character"));
    assert!(stdout.contains("voicevox / Fake / silence"));
    assert!(stdout.contains("smile"));
    assert!(stdout.contains("Wave"));
}

#[test]
fn character_inspect_json_reports_expressions_and_motions() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(
        tmp.path(),
        &[
            "--json",
            "character",
            "inspect",
            mock_character_dir().join("manifest.yaml").to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    let character = &payload["characters"][0];
    assert_eq!(character["id"], "mock");
    assert_eq!(character["voice"]["speaker"], "Fake");
    assert_eq!(character["model"]["expressions"][1], "smile");
    assert_eq!(character["model"]["motions"][1], "Wave");
}

#[test]
fn character_validate_resolves_fake_voice_and_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, stdout, stderr) = run(
        tmp.path(),
        &[
            "character",
            "validate",
            mock_character_dir().join("manifest.yaml").to_str().unwrap(),
            "--fake-tts",
        ],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("speaker_id 0"));
    assert!(stdout.trim_end().ends_with("OK"));
}

#[test]
fn character_validate_fails_on_unknown_speaker_and_missing_model() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest_path = tmp.path().join("bad.yaml");
    std::fs::write(
        &manifest_path,
        "characters:\n  - id: ghost\n    display_name: Ghost\n    voice:\n      provider: voicevox\n      speaker: Nobody\n      style: Nothing\n    model:\n      type: live2d\n      path: ./missing.json\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run(
        tmp.path(),
        &[
            "--json",
            "character",
            "validate",
            manifest_path.to_str().unwrap(),
            "--fake-tts",
        ],
    );
    assert_eq!(code, 2, "{stdout}{stderr}");
    let payload: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    assert_eq!(payload["ok"], false);
    let character = &payload["characters"][0];
    assert_eq!(character["ok"], false);
    assert!(character["voice"]["error"]
        .as_str()
        .unwrap()
        .contains("Nobody"));
    assert!(character["model"]["error"]
        .as_str()
        .unwrap()
        .contains("not found"));
}

#[test]
fn generate_with_a_linked_character_produces_a_performance_track_and_lipsync_file() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("demo");

    let (code, stdout, stderr) = run(tmp.path(), &["init", "demo"]);
    assert_eq!(code, 0, "{stdout}{stderr}");

    std::fs::copy(
        mock_character_dir().join("manifest.yaml"),
        ws.join("characters.yaml"),
    )
    .unwrap();
    std::fs::copy(
        mock_character_dir().join("model3.json"),
        ws.join("model3.json"),
    )
    .unwrap();
    std::fs::write(
        ws.join("videoforge.yaml"),
        "character_manifest: characters.yaml\nspeakers:\n  tsumugi:\n    character_id: mock\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("scripts/character.md"),
        "tsumugi[expression=smile, motion=Wave]:\nこんにちは。春日部つむぎです。\n",
    )
    .unwrap();

    let (code, stdout, stderr) = run(
        &ws,
        &["generate", "scripts/character.md", "--fake-tts", "--json"],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let gen: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let out_dir = PathBuf::from(gen["output_dir"].as_str().unwrap());

    let project: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(out_dir.join("project.vfp.json")).unwrap())
            .unwrap();
    let perf_track = project["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "character_performance")
        .expect("a character_performance track");
    let clip = &perf_track["clips"][0];
    assert_eq!(clip["character"], "mock");
    assert_eq!(clip["expression"], "smile");
    assert_eq!(clip["motion"], "Wave");
    let lip_sync_rel = clip["lip_sync"].as_str().unwrap();
    assert!(out_dir.join(lip_sync_rel).is_file(), "{lip_sync_rel}");
}
