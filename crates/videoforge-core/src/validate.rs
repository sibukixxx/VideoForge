//! Script validation against the workspace config (design §7.3).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use videoforge_project::{Presentation, KNOWN_INTENTS, KNOWN_ROLES};
use videoforge_script::{DirectiveKind, Script, ScriptError};

use crate::config::{Config, VoiceParams};
use crate::error::AppError;
use crate::workspace::Workspace;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub line: Option<usize>,
    pub message: String,
}

impl ValidationIssue {
    fn new(line: Option<usize>, message: impl Into<String>) -> Self {
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

    let known = config.known_speaker_names().join(", ");
    for d in &script.dialogues {
        match config.resolve_speaker(&d.speaker) {
            Some(resolved) => {
                for key in d.attributes.keys() {
                    report.warnings.push(ValidationIssue::new(
                        Some(d.line),
                        format!(
                            "attribute `{key}` on `{}` is not supported in v0.1 and is ignored",
                            d.speaker
                        ),
                    ));
                }
                report.total_chars += d.text.chars().count();
                report.dialogues.push(ResolvedDialogue {
                    index: d.index,
                    speaker_key: resolved.key.to_string(),
                    speaker_display: d.speaker.clone(),
                    text: d.text.clone(),
                    attributes: d.attributes.clone(),
                    voice: resolved.config.voice,
                    line: d.line,
                });
            }
            None => report.errors.push(ValidationIssue::new(
                Some(d.line),
                format!("unknown speaker `{}` (known speakers: {known})", d.speaker),
            )),
        }
    }

    for d in &script.directives {
        let (directive, attributes) = match &d.kind {
            DirectiveKind::Image { attributes, .. } => ("@image", attributes),
            DirectiveKind::Character { attributes, .. } => ("@character", attributes),
            DirectiveKind::Bgm { .. }
            | DirectiveKind::Se { .. }
            | DirectiveKind::Transition { .. } => continue,
        };
        if let Some(role) = attributes
            .get("role")
            .filter(|r| !Presentation::is_known_role(r))
        {
            report.warnings.push(ValidationIssue::new(
                Some(d.line),
                format!(
                    "`role={role}` on `{directive}` is not a recommended role (known: {}); it is kept as written",
                    KNOWN_ROLES.join(", ")
                ),
            ));
        }
        if let Some(intent) = attributes
            .get("intent")
            .filter(|i| !Presentation::is_known_intent(i))
        {
            report.warnings.push(ValidationIssue::new(
                Some(d.line),
                format!(
                    "`intent={intent}` on `{directive}` is not a recommended intent (known: {}); it is kept as written",
                    KNOWN_INTENTS.join(", ")
                ),
            ));
        }
    }

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
}
