//! Script validation against the workspace config (design §7.3).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use videoforge_script::{Script, ScriptError};

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

    if let Some(bg) = &config.preview.background {
        if config.preview.enabled && !workspace.resolve(bg).is_file() {
            report.warnings.push(ValidationIssue::new(
                None,
                format!("preview background `{bg}` not found; a flat color will be used"),
            ));
        }
    }
    if let Some(font) = &config.preview.font {
        if config.preview.enabled && !workspace.resolve(font).is_file() {
            report.warnings.push(ValidationIssue::new(
                None,
                format!("preview font `{font}` not found; FFmpeg default font will be used"),
            ));
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
    fn parse_error_becomes_issue() {
        let (_d, ws, cfg) = workspace();
        let path = ws.scripts_dir().join("bad.md");
        std::fs::write(&path, "text first\n").unwrap();
        let report = validate_file(&ws, &cfg, &path);
        assert_eq!(report.errors.len(), 1);
        assert_eq!(report.errors[0].line, Some(1));
    }
}
