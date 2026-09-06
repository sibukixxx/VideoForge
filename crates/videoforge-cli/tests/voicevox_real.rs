//! End-to-end CLI test against a *real* VOICEVOX Engine (issue #10):
//! init → validate → doctor → generate → generated artifacts → cache hit.
//!
//! Runs only when `videoforge doctor` reports the engine as reachable at
//! `$VIDEOFORGE_VOICEVOX_ENDPOINT` (default `http://127.0.0.1:50021`);
//! otherwise it prints `skipped:` and passes. FFmpeg is used when `doctor`
//! finds one, so `preview.mp4` is asserted only in that case. This is the
//! automated half of `docs/testing/voicevox-manual-e2e.md`.

use std::path::{Path, PathBuf};
use std::process::Command;

use videoforge_core::wav::parse_wav_info;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_videoforge"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn endpoint_args() -> Vec<String> {
    match std::env::var("VIDEOFORGE_VOICEVOX_ENDPOINT") {
        Ok(endpoint) => vec!["--endpoint".into(), endpoint],
        Err(_) => Vec::new(),
    }
}

fn run(cwd: &Path, cache_dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin())
        .args(args)
        .args(endpoint_args())
        .current_dir(cwd)
        .env("VIDEOFORGE_CACHE_DIR", cache_dir)
        .output()
        .expect("run videoforge");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn full_pipeline_with_real_voicevox() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_dir = tmp.path().join(".cache");
    let ws = tmp.path().join("demo");

    let (code, stdout, stderr) = run(tmp.path(), &cache_dir, &["init", "demo"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    std::fs::copy(
        repo_root().join("fixtures/scripts/sample.md"),
        ws.join("scripts/sample.md"),
    )
    .unwrap();

    // doctor decides whether this test runs at all, and whether a preview is expected.
    let (_, stdout, stderr) = run(&ws, &cache_dir, &["doctor", "--json"]);
    let doctor: serde_json::Value = serde_json::from_str(&stdout).expect(&stderr);
    if doctor["capabilities"]["voicevox_available"] != true {
        let detail = doctor["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["name"] == "VOICEVOX connection")
            .map(|c| c["detail"].to_string())
            .unwrap_or_default();
        eprintln!("skipped: VOICEVOX not reachable {detail}; set VIDEOFORGE_VOICEVOX_ENDPOINT");
        return;
    }
    let ffmpeg_available = doctor["capabilities"]["ffmpeg_available"] == true;
    eprintln!(
        "{}",
        doctor["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| format!("{}: {}", c["name"], c["detail"]))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let (code, stdout, _) = run(&ws, &cache_dir, &["validate", "scripts/sample.md"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("Dialogues: 3"), "{stdout}");

    let (code, stdout, stderr) = run(
        &ws,
        &cache_dir,
        &["generate", "scripts/sample.md", "--json"],
    );
    assert_eq!(code, 0, "{stdout}{stderr}");
    let generated: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(generated["slug"], "sample");
    let out_dir = ws.join("generated").join("sample");

    // project.vfp.json: one audio clip per dialogue, each backed by a real WAV
    // whose header duration is what the timeline used.
    let project = read_json(&out_dir.join("project.vfp.json"));
    let audio_track = project["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["kind"] == "audio")
        .expect("audio track");
    let clips = audio_track["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 3);
    let mut previous_end = 0;
    for clip in clips {
        let wav_path = out_dir.join(clip["source"].as_str().unwrap());
        let wav = parse_wav_info(&std::fs::read(&wav_path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", wav_path.display()));
        assert!(wav.duration_ms > 0, "{}", wav_path.display());
        assert_eq!(
            clip["duration_ms"],
            wav.duration_ms,
            "{}",
            wav_path.display()
        );
        let start = clip["start_ms"].as_u64().unwrap();
        assert!(start >= previous_end, "clips must not overlap: {clip}");
        previous_end = start + wav.duration_ms;
    }

    // captions.srt: three cues, in order, with the script text.
    let srt = std::fs::read_to_string(out_dir.join("captions.srt")).unwrap();
    assert_eq!(srt.matches(" --> ").count(), 3, "{srt}");
    assert!(
        srt.contains("半年前までは、最先端のAIでもかなり苦戦していました。"),
        "{srt}"
    );

    // manifest.json: dialogues, total duration and the preview decision.
    let manifest = read_json(&out_dir.join("manifest.json"));
    assert_eq!(manifest["dialogues"], 3);
    assert_eq!(manifest["duration_ms"], previous_end);
    assert!(
        !manifest["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("TTS cache disabled")),
        "{manifest}"
    );
    if ffmpeg_available {
        let preview = out_dir.join("preview.mp4");
        assert!(
            preview.is_file(),
            "doctor found FFmpeg, so preview.mp4 must exist"
        );
        assert!(std::fs::metadata(&preview).unwrap().len() > 1024);
        assert_eq!(manifest["preview"], "preview.mp4");
    } else {
        assert!(manifest["preview"].is_null(), "{manifest}");
    }

    // second run: every dialogue comes from the version-keyed cache.
    let (code, _, stderr) = run(
        &ws,
        &cache_dir,
        &["generate", "scripts/sample.md", "--no-preview"],
    );
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stderr.matches("(cached)").count(), 3, "{stderr}");
}
