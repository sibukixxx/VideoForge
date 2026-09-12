//! Script validation against the workspace config (design §7.3).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use videoforge_character::live2d::{self, Live2dModelInfo};
use videoforge_script::{Dialogue, Script, ScriptError};

use crate::character::LoadedManifest;
use crate::directives::{resolve_directives, ResolvedDirective};

use crate::config::{Config, SpeakerConfig, VoiceParams};
use crate::error::AppError;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub line: Option<usize>,
    pub message: String,
}

impl ValidationIssue {
    pub fn new(line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

/// A dialogue whose speaker has been resolved to a config entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedDialogue {
    pub index: usize,
    pub speaker_key: String,
    pub speaker_display: String,
    pub text: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    pub voice: VoiceParams,
    pub line: usize,
    /// Character (design §5) this dialogue performs as, if the speaker links
    /// one via `SpeakerConfig::character_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    /// Resolved `expression=` attribute; `None` means the script did not say
    /// (a default of `"default"` is applied when the performance clip is
    /// built, design §12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    /// Resolved `motion=` attribute; `None` means the script did not say
    /// (a default of `"idle"` is applied when the performance clip is
    /// built, design §12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ValidationReport {
    pub script: String,
    pub title: String,
    pub slug: String,
    pub template: Option<String>,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
    pub dialogues: Vec<ResolvedDialogue>,
    /// Presentation directives with their assets checked (issue #18).
    #[serde(default)]
    pub directives: Vec<ResolvedDirective>,
    pub total_chars: usize,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Turn a failed report into the first error.
    pub fn into_result(self) -> Result<Self, AppError> {
        if let Some(first) = self.errors.first() {
            let msg = match first.line {
                Some(l) => format!("{} (line {l})", first.message),
                None => first.message.clone(),
            };
            return Err(AppError::InvalidScript(msg));
        }
        Ok(self)
    }
}

/// Parse + validate a script file. Parse errors are reported as issues rather
/// than returned, so the CLI can print a uniform report.
pub fn validate_file(workspace: &Workspace, config: &Config, path: &Path) -> ValidationReport {
    let rel = workspace.relative(path);
    match videoforge_script::parse_file(path) {
        Ok(script) => validate_script(&script, config, workspace),
        Err(err) => {
            let line = match &err {
                ScriptError::TextWithoutSpeaker { line } => Some(*line),
                ScriptError::EmptyDialogue { line, .. } => Some(*line),
                _ => None,
            };
            ValidationReport {
                script: rel,
                errors: vec![ValidationIssue::new(line, err.to_string())],
                ..Default::default()
            }
        }
    }
}

pub fn validate_script(
    script: &Script,
    config: &Config,
    workspace: &Workspace,
) -> ValidationReport {
    let mut report = ValidationReport {
        script: script
            .source_path
            .as_deref()
            .map(|p| workspace.relative(p))
            .unwrap_or_default(),
        title: script.title(),
        slug: script.slug(),
        template: script.front_matter.template.clone(),
        ..Default::default()
    };

    for w in &script.warnings {
        report
            .warnings
            .push(ValidationIssue::new(Some(w.line), w.message.clone()));
    }

    // Loaded once per validation, not per dialogue: most scripts either use
    // no character at all (the common case, and this stays `None`) or reuse
    // the same one or two characters throughout.
    let character_manifest = match crate::character::load_manifest(config, workspace) {
        Ok(m) => m,
        Err(e) => {
            report
                .errors
                .push(ValidationIssue::new(None, e.to_string()));
            None
        }
    };
    let mut model_cache: HashMap<String, Result<Live2dModelInfo, String>> = HashMap::new();
    let mut png_cache: HashMap<String, Result<videoforge_character::PngLipsyncAssets, String>> =
        HashMap::new();

    let known = config.known_speaker_names().join(", ");
    for d in &script.dialogues {
        match config.resolve_speaker(&d.speaker) {
            Some(resolved) => {
                let perf = resolve_character_performance(
                    d,
                    resolved.key,
                    resolved.config,
                    character_manifest.as_ref(),
                    &mut model_cache,
                    &mut png_cache,
                    &mut report,
                );
                report.total_chars += d.text.chars().count();
                report.dialogues.push(ResolvedDialogue {
                    index: d.index,
                    speaker_key: resolved.key.to_string(),
                    speaker_display: d.speaker.clone(),
                    text: d.text.clone(),
                    attributes: d.attributes.clone(),
                    voice: resolved.config.voice,
                    line: d.line,
                    character_id: perf.character_id,
                    expression: perf.expression,
                    motion: perf.motion,
                });
            }
            None => report.errors.push(ValidationIssue::new(
                Some(d.line),
                format!("unknown speaker `{}` (known speakers: {known})", d.speaker),
            )),
        }
    }

    let resolution = resolve_directives(script, config, workspace);
    report.errors.extend(resolution.errors);
    report.warnings.extend(resolution.warnings);
    report.directives = resolution.directives;

    if let Some(bg) = &config.preview.background {
        if config.preview.enabled {
            match workspace.resolve(bg) {
                Ok(path) if path.is_file() => {}
                Ok(_) => report.warnings.push(ValidationIssue::new(
                    None,
                    format!("preview background `{bg}` not found; a flat color will be used"),
                )),
                Err(e) => report.errors.push(ValidationIssue::new(
                    None,
                    format!("preview.background `{bg}` is invalid: {e}"),
                )),
            }
        }
    }
    if let Some(font) = &config.preview.font {
        if config.preview.enabled {
            match workspace.resolve_allow_absolute(font) {
                Ok(path) if path.is_file() => {}
                Ok(_) => report.warnings.push(ValidationIssue::new(
                    None,
                    format!("preview font `{font}` not found; FFmpeg default font will be used"),
                )),
                Err(e) => report.errors.push(ValidationIssue::new(
                    None,
                    format!("preview.font `{font}` is invalid: {e}"),
                )),
            }
        }
    }

    report
}

/// What a dialogue's `expression=`/`motion=` attributes resolved to, and
/// which character (if any) it performs as.
struct CharacterPerformance {
    character_id: Option<String>,
    expression: Option<String>,
    motion: Option<String>,
}

/// Validate and resolve one dialogue's character performance attributes
/// (design §9, §12): unrecognized attributes on a plain speaker keep warning
/// exactly as before (v0.1 behavior, unchanged); on a speaker linked to a
/// character, `expression=`/`motion=` are checked against that character's
/// known list (explicit, or read from its Live2D `model3.json`) instead of
/// being warned about, and any other attribute still warns.
#[allow(clippy::too_many_arguments)]
fn resolve_character_performance(
    d: &Dialogue,
    speaker_key: &str,
    speaker_config: &SpeakerConfig,
    character_manifest: Option<&LoadedManifest>,
    model_cache: &mut HashMap<String, Result<Live2dModelInfo, String>>,
    png_cache: &mut HashMap<String, Result<videoforge_character::PngLipsyncAssets, String>>,
    report: &mut ValidationReport,
) -> CharacterPerformance {
    let warn_unsupported = |report: &mut ValidationReport, key: &str| {
        report.warnings.push(ValidationIssue::new(
            Some(d.line),
            format!(
                "attribute `{key}` on `{}` is not supported in v0.1 and is ignored",
                d.speaker
            ),
        ));
    };

    let Some(character_id) = &speaker_config.character_id else {
        for key in d.attributes.keys() {
            warn_unsupported(report, key);
        }
        return CharacterPerformance {
            character_id: None,
            expression: None,
            motion: None,
        };
    };

    // `Config::validate` requires a `character_manifest` whenever any
    // speaker sets `character_id`; if loading it still failed (bad YAML, I/O
    // error) that is already a report-level error pushed by the caller, so
    // just fall through with an unresolved performance rather than
    // duplicating it here.
    let Some(loaded) = character_manifest else {
        return CharacterPerformance {
            character_id: Some(character_id.clone()),
            expression: None,
            motion: None,
        };
    };

    let Some(character) = loaded.manifest.find(character_id) else {
        report.errors.push(ValidationIssue::new(
            Some(d.line),
            format!(
                "speaker `{speaker_key}` links to unknown character `{character_id}` (known characters: {})",
                loaded.manifest.character_ids().join(", ")
            ),
        ));
        return CharacterPerformance {
            character_id: Some(character_id.clone()),
            expression: None,
            motion: None,
        };
    };

    let manifest_dir = loaded.path.parent().unwrap_or_else(|| Path::new("."));
    let model_info: Option<Live2dModelInfo> = match &character.model {
        Some(model) if model.is_live2d() => {
            let result = model_cache.entry(character_id.clone()).or_insert_with(|| {
                let resolved_path = model.resolve_path(manifest_dir);
                live2d::load_model3_json(&resolved_path).map_err(|e| e.to_string())
            });
            match result {
                Ok(info) => Some(info.clone()),
                Err(reason) => {
                    report.errors.push(ValidationIssue::new(
                        Some(d.line),
                        format!("character `{character_id}` Live2D model: {reason}"),
                    ));
                    None
                }
            }
        }
        Some(model) if model.is_png_lipsync() => {
            let result = png_cache.entry(character_id.clone()).or_insert_with(|| {
                videoforge_character::png_lipsync::load_png_lipsync_assets(model, manifest_dir)
                    .map_err(|e| e.to_string())
            });
            if let Err(reason) = result {
                report.errors.push(ValidationIssue::new(
                    Some(d.line),
                    format!("character `{character_id}` PNG sprites: {reason}"),
                ));
            }
            None
        }
        _ => None,
    };

    let known_expressions: Vec<&str> = if !character.expressions.is_empty() {
        character.expressions.iter().map(String::as_str).collect()
    } else {
        model_info
            .as_ref()
            .map(|i| i.expressions.iter().map(String::as_str).collect())
            .unwrap_or_default()
    };
    let known_motions: Vec<&str> = if !character.motions.is_empty() {
        character.motions.iter().map(String::as_str).collect()
    } else {
        model_info
            .as_ref()
            .map(|i| i.motions.iter().map(String::as_str).collect())
            .unwrap_or_default()
    };

    let mut expression = None;
    let mut motion = None;
    for (key, value) in &d.attributes {
        match key.as_str() {
            "expression" => {
                if !known_expressions.is_empty() && !known_expressions.contains(&value.as_str()) {
                    report.errors.push(ValidationIssue::new(
                        Some(d.line),
                        format!(
                            "character `{character_id}` has no expression `{value}` (known: {})",
                            known_expressions.join(", ")
                        ),
                    ));
                }
                expression = Some(value.clone());
            }
            "motion" => {
                if !known_motions.is_empty() && !known_motions.contains(&value.as_str()) {
                    report.errors.push(ValidationIssue::new(
                        Some(d.line),
                        format!(
                            "character `{character_id}` has no motion `{value}` (known: {})",
                            known_motions.join(", ")
                        ),
                    ));
                }
                motion = Some(value.clone());
            }
            other => warn_unsupported(report, other),
        }
    }

    CharacterPerformance {
        character_id: Some(character_id.clone()),
        expression,
        motion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config_yaml;
    use crate::init;

    fn workspace() -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(&default_config_yaml("t"), Path::new("x")).unwrap();
        (dir, ws, cfg)
    }

    #[test]
    fn sample_script_validates() {
        let (_d, ws, cfg) = workspace();
        let report = validate_file(&ws, &cfg, &ws.scripts_dir().join("sample.md"));
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!(report.dialogues.len(), 3);
        assert_eq!(report.dialogues[0].speaker_key, "reimu");
        assert_eq!(report.dialogues[1].speaker_key, "marisa");
        assert_eq!(report.slug, "sample");
        assert_eq!(report.script, "scripts/sample.md");
        // background missing → warning, not error
        assert!(report
            .warnings
            .iter()
            .any(|w| w.message.contains("background")));
    }

    #[test]
    fn unknown_speaker_is_error() {
        let (_d, ws, cfg) = workspace();
        let path = ws.scripts_dir().join("bad.md");
        std::fs::write(&path, "霊夢:\nやあ\n\nアリス:\nだれ？\n").unwrap();
        let report = validate_file(&ws, &cfg, &path);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].line, Some(4));
        assert!(report.errors[0].message.contains("アリス"));
        assert!(report.into_result().is_err());
    }

    #[test]
    fn unknown_presentation_vocabulary_on_directives_is_a_warning_not_an_error() {
        let (_d, ws, cfg) = workspace();
        let path = ws.scripts_dir().join("visual.md");
        std::fs::write(
            &path,
            "@image assets/image/a.png[role=hero, intent=wobble]\n@character reimu[role=character, intent=fade]\n@bgm assets/bgm/a.mp3[role=hero]\n霊夢:\nやあ\n",
        )
        .unwrap();
        let report = validate_file(&ws, &cfg, &path);
        assert!(report.is_ok(), "{:?}", report.errors);
        let vocabulary: Vec<&ValidationIssue> = report
            .warnings
            .iter()
            .filter(|w| w.message.contains("recommended"))
            .collect();
        assert_eq!(
            vocabulary,
            vec![
                &ValidationIssue::new(
                    Some(1),
                    "`role=hero` on `@image` is not a recommended role (known: primary_visual, supporting_visual, diagram, character, background, callout, comparison, emphasis); it is kept as written"
                ),
                &ValidationIssue::new(
                    Some(1),
                    "`intent=wobble` on `@image` is not a recommended intent (known: fade, slide, zoom, emphasis, cut); it is kept as written"
                ),
            ]
        );
    }

    #[test]
    fn parse_error_becomes_issue() {
        let (_d, ws, cfg) = workspace();
        let path = ws.scripts_dir().join("bad.md");
        std::fs::write(&path, "text first\n").unwrap();
        let report = validate_file(&ws, &cfg, &path);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].line, Some(1));
    }

    fn character_workspace() -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/manifest.yaml"
            ),
            dir.path().join("characters.yaml"),
        )
        .unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/model3.json"
            ),
            dir.path().join("model3.json"),
        )
        .unwrap();
        let yaml =
            "character_manifest: characters.yaml\nspeakers:\n  tsumugi:\n    character_id: mock\n";
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(yaml, dir.path()).unwrap();
        (dir, ws, cfg)
    }

    #[test]
    fn character_dialogue_resolves_expression_and_motion() {
        let (_d, ws, cfg) = character_workspace();
        let script =
            videoforge_script::parse_str("tsumugi[expression=smile, motion=Wave]:\nこんにちは\n")
                .unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!(report.dialogues.len(), 1);
        let d = &report.dialogues[0];
        assert_eq!(d.character_id.as_deref(), Some("mock"));
        assert_eq!(d.expression.as_deref(), Some("smile"));
        assert_eq!(d.motion.as_deref(), Some("Wave"));
    }

    #[test]
    fn character_dialogue_without_attributes_leaves_expression_and_motion_unset() {
        let (_d, ws, cfg) = character_workspace();
        let script = videoforge_script::parse_str("tsumugi:\nこんにちは\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!(report.dialogues[0].character_id.as_deref(), Some("mock"));
        assert_eq!(report.dialogues[0].expression, None);
        assert_eq!(report.dialogues[0].motion, None);
    }

    #[test]
    fn unknown_expression_is_a_validation_error() {
        let (_d, ws, cfg) = character_workspace();
        let script =
            videoforge_script::parse_str("tsumugi[expression=angry]:\nこんにちは\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("no expression") && e.message.contains("angry")));
    }

    #[test]
    fn unknown_motion_is_a_validation_error() {
        let (_d, ws, cfg) = character_workspace();
        let script =
            videoforge_script::parse_str("tsumugi[motion=backflip]:\nこんにちは\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("no motion") && e.message.contains("backflip")));
    }

    #[test]
    fn unknown_character_id_is_a_validation_error() {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        std::fs::copy(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/character/mock-character/manifest.yaml"
            ),
            dir.path().join("characters.yaml"),
        )
        .unwrap();
        let yaml =
            "character_manifest: characters.yaml\nspeakers:\n  tsumugi:\n    character_id: ghost\n";
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(yaml, dir.path()).unwrap();
        let script = videoforge_script::parse_str("tsumugi:\nこんにちは\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("unknown character") && e.message.contains("ghost")));
    }

    fn png_character_workspace() -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/character/mock-png-character"
        ));
        let dest_sprites = dir.path().join("characters/sprites");
        std::fs::create_dir_all(&dest_sprites).unwrap();
        std::fs::copy(
            fixture.join("manifest.yaml"),
            dir.path().join("characters/manifest.yaml"),
        )
        .unwrap();
        for name in ["closed.png", "half.png", "open.png"] {
            std::fs::copy(fixture.join("sprites").join(name), dest_sprites.join(name)).unwrap();
        }
        let yaml = "character_manifest: characters/manifest.yaml\nspeakers:\n  mock_a:\n    character_id: mock_a\n  mock_b:\n    character_id: mock_b\n";
        std::fs::write(dir.path().join("videoforge.yaml"), yaml).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(yaml, dir.path()).unwrap();
        (dir, ws, cfg)
    }

    #[test]
    fn png_lipsync_character_dialogue_validates() {
        let (_d, ws, cfg) = png_character_workspace();
        let script =
            videoforge_script::parse_str("mock_a:\nこんにちは\n\nmock_b:\nやあ\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!(report.dialogues[0].character_id.as_deref(), Some("mock_a"));
        assert_eq!(report.dialogues[1].character_id.as_deref(), Some("mock_b"));
    }

    #[test]
    fn png_lipsync_character_with_missing_sprite_is_a_validation_error() {
        let (dir, ws, cfg) = png_character_workspace();
        std::fs::remove_file(dir.path().join("characters/sprites/open.png")).unwrap();
        let script = videoforge_script::parse_str("mock_a:\nこんにちは\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(!report.is_ok());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("PNG sprites")));
    }

    #[test]
    fn plain_speaker_still_warns_on_expression_attribute() {
        // No character_id linked: expression/motion are just unsupported
        // attributes, exactly like any other v0.1 attribute (unchanged
        // behavior for scripts that don't use characters).
        let (_d, ws, cfg) = workspace();
        let script = videoforge_script::parse_str("霊夢[expression=smile]:\nやあ\n").unwrap();
        let report = validate_script(&script, &cfg, &ws);
        assert!(report.is_ok(), "{:?}", report.errors);
        assert_eq!(report.dialogues[0].character_id, None);
        assert!(report
            .warnings
            .iter()
            .any(|w| w.message.contains("attribute `expression`")
                && w.message.contains("not supported")));
    }
}
