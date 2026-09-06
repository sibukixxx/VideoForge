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
