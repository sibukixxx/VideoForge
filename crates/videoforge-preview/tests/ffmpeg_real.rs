//! Integration tests against a *real* FFmpeg (issue #12).
//!
//! These run only when an FFmpeg binary is found (`$VIDEOFORGE_FFMPEG`, else
//! `ffmpeg` on `PATH`); otherwise each test prints `skipped:` and passes, so
//! the offline `cargo test --workspace` stays green. CI installs FFmpeg on
//! every OS of the matrix, which is what makes these cross-platform.
//!
//! What is covered:
//!
//! * `escaped_paths_are_accepted_by_the_filtergraph_parser` — feeds one path
//!   per special-character class through [`quote_filter_value`] into a real
//!   filtergraph (`amovie=<path>`), which exercises both parsing passes
//!   exactly like `drawtext=fontfile=`/`textfile=` do. Needs no optional
//!   filter, so it runs on any build. On Windows the temp dir gives a drive
//!   path (`C:\...`) for free.
//! * `preview_renders_from_a_workspace_with_special_characters` — the full
//!   pipeline (fake TTS + FFmpeg) inside a workspace whose path contains a
//!   space, Japanese, an apostrophe, brackets and a semicolon, with a font
//!   file copied to a similar path. Needs `drawtext`; skipped without it.
//!
//! UNC paths (`\\server\share\...`) are not exercised automatically — CI
//! runners have no share to point at — see `docs/testing/ffmpeg-path-escaping.md`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;

use videoforge_core::tts::FakeTtsEngine;
use videoforge_core::{generate, init, GenerateDeps, GenerateOptions, Workspace};
use videoforge_preview::{quote_filter_value, FfmpegPreviewRenderer};

fn find_ffmpeg() -> Option<PathBuf> {
    let candidate = std::env::var_os("VIDEOFORGE_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let ok = Command::new(&candidate)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then_some(candidate)
}

fn has_filter(ffmpeg: &Path, name: &str) -> bool {
    Command::new(ffmpeg)
        .args(["-hide_banner", "-filters"])
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l.split_whitespace().nth(1) == Some(name))
        })
        .unwrap_or(false)
}

/// A font any FFmpeg build with `drawtext` can open, if this OS ships one.
fn find_font() -> Option<PathBuf> {
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
            r"C:\Windows\Fonts\arial.ttf",
        ]
    } else if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
    } else {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
        ]
    };
    let from_env = std::env::var_os("VIDEOFORGE_TEST_FONT").map(PathBuf::from);
    from_env
        .into_iter()
        .chain(candidates.iter().map(PathBuf::from))
        .find(|p| p.is_file())
}

/// Every special-character class from issue #12 that can appear in a file
/// name on every OS. `\` and `:` are separately covered by the Windows temp
/// path itself (`C:\Users\...`), since neither is legal in a Windows file name.
const SPECIAL_NAMES: &[&str] = &[
    "plain",
    "with space",
    "日本語 音声",
    "it's",
    "com,ma",
    "semi;colon",
    "br[ack]ets",
    "eq=ual",
    "pct%",
    "hash#",
    " leading",
    "trailing ",
];

#[cfg(not(windows))]
const UNIX_ONLY_NAMES: &[&str] = &["co:lon", "back\\slash"];
#[cfg(windows)]
const UNIX_ONLY_NAMES: &[&str] = &[];

#[test]
fn escaped_paths_are_accepted_by_the_filtergraph_parser() {
    let Some(ffmpeg) = find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg (set VIDEOFORGE_FFMPEG or put ffmpeg on PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    // The directory itself carries special characters too, so an absolute
    // path exercises them mid-string, not only in the file name.
    let dir = tmp.path().join("esc test [1]; it's");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = videoforge_core::wav::silent_wav(200, 8000);

    let mut failures = Vec::new();
    for name in SPECIAL_NAMES.iter().chain(UNIX_ONLY_NAMES) {
        let path = dir.join(format!("{name}.wav"));
        std::fs::write(&path, &wav).unwrap();
        let abs = path.to_string_lossy().into_owned();
        let graph = format!("amovie={}", quote_filter_value(&abs));
        let out = Command::new(&ffmpeg)
            .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
            .args(["-f", "lavfi", "-i", &graph, "-t", "0.1", "-f", "null", "-"])
            .stdin(Stdio::null())
            .output()
            .expect("run ffmpeg");
        if !out.status.success() {
            failures.push(format!(
                "{name:?} → {graph}\n    {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "ffmpeg rejected escaped paths:\n  {}",
        failures.join("\n  ")
    );
}

#[tokio::test]
async fn preview_renders_from_a_workspace_with_special_characters() {
    let Some(ffmpeg) = find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg (set VIDEOFORGE_FFMPEG or put ffmpeg on PATH)");
        return;
    };
    if !has_filter(&ffmpeg, "drawtext") {
        eprintln!("skipped: this ffmpeg has no drawtext filter (built without libfreetype)");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("my videos [v2]; 霊夢's channel");
    init::init(&root, Some("special")).unwrap();
    let ws = Workspace::open(&root).unwrap();

    // Font at an absolute path with the same character classes, when one is
    // available; the render must still succeed without it.
    let font_line = match find_font() {
        Some(font) => {
            let font_dir = tmp.path().join("fonts dir; it's [x]");
            std::fs::create_dir_all(&font_dir).unwrap();
            let ext = font.extension().and_then(|e| e.to_str()).unwrap_or("ttf");
            let dst = font_dir.join(format!("日本語 font.{ext}"));
            std::fs::copy(&font, &dst).unwrap();
            format!("  font: {}\n", serde_yaml_quote(&dst.to_string_lossy()))
        }
        None => {
            eprintln!("note: no system font found, rendering with FFmpeg's default font");
            String::new()
        }
    };
    let cfg_path = ws.config_path();
    let mut cfg = std::fs::read_to_string(&cfg_path).unwrap();
    cfg = cfg.replace("preview:\n", &format!("preview:\n{font_line}"));
    assert!(
        cfg.contains("preview:\n"),
        "unexpected default config layout"
    );
    std::fs::write(&cfg_path, cfg).unwrap();

    let mut deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
    deps.preview = Some(Arc::new(FfmpegPreviewRenderer::with_binary(ffmpeg)));
    let out = generate(
        &ws,
        &ws.scripts_dir().join("sample.md"),
        GenerateOptions {
            preview: Some(true),
            ..Default::default()
        },
        deps,
    )
    .await
    .expect("generate with a real ffmpeg");

    let preview = out.preview_path.expect("preview.mp4 must be rendered");
    let size = std::fs::metadata(&preview).unwrap().len();
    assert!(size > 1024, "preview.mp4 is only {size} bytes");
    assert!(
        !out.warnings.iter().any(|w| w.contains("preview skipped")),
        "{:?}",
        out.warnings
    );
    assert!(
        !out.output_dir.join(".preview.mp4.tmp").exists(),
        "scratch dir must be cleaned up"
    );
}

/// Double-quoted YAML scalar (backslashes and quotes escaped).
fn serde_yaml_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Regression test for a real bug found by dogfooding P1 (docs/testing/p1-dogfood-e2e.md):
/// `overlay`'s `x=`/`y=` position expressions (`min(max(0,...),...)`, used by
/// every character overlay and every general visual layer since P0-1/P1-1)
/// contain a raw, unescaped `,` inside `min(max(...))`. FFmpeg's
/// filter-option tokenizer splits an option's value on a bare `,` — it is
/// not just a *decoration* separator, it ends the value — so the whole
/// filtergraph failed to parse ("No option name near ...") the moment a
/// project had more than a background and captions. No unit test caught
/// this because every existing test only asserts on the generated *string*,
/// never feeds it to a real FFmpeg. This test does, on the minimal
/// multi-layer shape (one background + one character overlay) that
/// triggers it.
#[tokio::test]
async fn overlay_position_expressions_parse_in_a_real_filtergraph() {
    let Some(ffmpeg) = find_ffmpeg() else {
        eprintln!("skipped: no ffmpeg (set VIDEOFORGE_FFMPEG or put ffmpeg on PATH)");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("overlay-quoting");
    init::init(&root, Some("overlay-quoting")).unwrap();
    let ws = Workspace::open(&root).unwrap();

    // Link a speaker to the repo's own synthetic png_lipsync fixture so the
    // rendered graph gets a character overlay — the shape that triggers the
    // bug (a background-only project's single-chain fast path never hits
    // `overlay=`  at all).
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/character/mock-png-character")
        .canonicalize()
        .unwrap();
    let manifest_dst = root.join("characters.yaml");
    std::fs::copy(fixture_root.join("manifest.yaml"), &manifest_dst).unwrap();
    let sprites_dst = root.join("sprites");
    std::fs::create_dir_all(&sprites_dst).unwrap();
    for name in ["closed.png", "half.png", "open.png"] {
        std::fs::copy(
            fixture_root.join("sprites").join(name),
            sprites_dst.join(name),
        )
        .unwrap();
    }

    let cfg_path = ws.config_path();
    let mut cfg = std::fs::read_to_string(&cfg_path).unwrap();
    cfg = cfg.replace(
        "speakers:\n  reimu:\n    aliases:\n      - 霊夢\n",
        "character_manifest: characters.yaml\nspeakers:\n  reimu:\n    aliases:\n      - 霊夢\n    character_id: mock_a\n",
    );
    assert!(
        cfg.contains("character_id: mock_a"),
        "unexpected default config layout"
    );
    std::fs::write(&cfg_path, cfg).unwrap();

    let mut deps = GenerateDeps::new(Arc::new(FakeTtsEngine::default()));
    deps.preview = Some(Arc::new(FfmpegPreviewRenderer::with_binary(ffmpeg)));
    let out = generate(
        &ws,
        &ws.scripts_dir().join("sample.md"),
        GenerateOptions {
            preview: Some(true),
            ..Default::default()
        },
        deps,
    )
    .await
    .expect("generate with a character overlay and a real ffmpeg");

    let preview = out.preview_path.expect("preview.mp4 must be rendered");
    let size = std::fs::metadata(&preview).unwrap().len();
    assert!(size > 1024, "preview.mp4 is only {size} bytes");
}
