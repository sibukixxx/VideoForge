//! Source-grounded, agent-assisted drafts. No network, TTS or publication.
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppError, Config, Workspace};

pub const PROMPT: &str = include_str!("../../../prompts/script-draft-p0.md");

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    pub topic: String,
    pub audience: String,
    pub target_seconds: u32,
    pub sources: Vec<Source>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub id: String,
    pub locator: String,
    pub published_at: String,
    pub checked_at: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub title: String,
    pub dialogues: Vec<DraftDialogue>,
    pub unresolved: Vec<String>,
    pub material_requests: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftDialogue {
    pub speaker: String,
    pub text: String,
    pub kind: DialogueKind,
    pub evidence: Vec<Evidence>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DialogueKind {
    Fact,
    Opinion,
    Question,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source_id: String,
    pub quote: String,
}

#[derive(Debug, Serialize)]
pub struct DraftReport {
    pub structurally_valid: bool,
    pub review_hash: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub dialogue_count: usize,
    pub estimated_seconds: u64,
    pub target_seconds: u32,
    pub markdown: String,
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::InvalidScript(message.into())
}

impl Brief {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.topic.trim().is_empty() || self.audience.trim().is_empty()
            || !(10..=1800).contains(&self.target_seconds) || self.sources.is_empty()
        {
            return Err(invalid("topic, audience, sources and target_seconds (10..1800) required"));
        }
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            if source.id.trim().is_empty() || !ids.insert(&source.id)
                || source.locator.trim().is_empty() || source.text.trim().is_empty()
                || source.checked_at.trim().is_empty() || source.published_at.trim().is_empty()
            {
                return Err(invalid("sources require unique IDs, locator, text and dates (or unknown)"));
            }
        }
        Ok(())
    }
}

pub fn prompt(brief: &Brief, config: &Config) -> Result<String, AppError> {
    brief.validate()?;
    let data = serde_json::json!({
        "brief": brief,
        "allowed_speakers": config.known_speaker_names(),
    });
    Ok(format!("{PROMPT}\n\nUNTRUSTED_INPUT_JSON:\n{data}\n"))
}

/// Exact quotations prove traceability, NOT entailment or truth. Humans review both.
pub fn check(brief: &Brief, draft: &Draft, config: &Config, ws: &Workspace)
    -> Result<DraftReport, AppError>
{
    brief.validate()?;
    let mut errors = Vec::new();
    let mut warnings = vec![
        "Structural checks are not fact checking. Review every claim, source and title.".into(),
        "Timing estimate assumes 5 characters/second; only synthesized audio gives actual duration.".into(),
    ];
    if draft.title.trim().is_empty() || draft.title.contains(['\n', '\r']) {
        errors.push("title must be nonempty and single-line".into());
    }
    if !draft.unresolved.is_empty() {
        errors.push("unresolved issues must be resolved before export".into());
    }
    if !draft.material_requests.is_empty() {
        errors.push("material requests must be resolved before export; P0 does not insert assets".into());
    }
    let mut markdown = format!("---\ntitle: {}\n---\n\n", serde_json::json!(draft.title));
    let mut chars = 0usize;
    for (index, d) in draft.dialogues.iter().enumerate() {
        let n = index + 1;
        if config.resolve_speaker(&d.speaker).is_none() {
            errors.push(format!("dialogue {n}: unknown speaker"));
        }
        if d.text.trim().is_empty() || d.text.contains(['\n', '\r'])
            || d.speaker.contains(['\n', '\r'])
        {
            errors.push(format!("dialogue {n}: speaker/text must be nonempty single lines"));
        }
        if d.text.chars().count() > 120 {
            errors.push(format!("dialogue {n}: split text into at most 120 characters"));
        }
        if d.kind == DialogueKind::Fact && d.evidence.is_empty() {
            errors.push(format!("dialogue {n}: facts require evidence"));
        }
        for evidence in &d.evidence {
            let valid = !evidence.quote.trim().is_empty() && brief.sources.iter().any(|s| {
                s.id == evidence.source_id && s.text.contains(&evidence.quote)
            });
            if !valid {
                errors.push(format!("dialogue {n}: missing source or non-verbatim evidence"));
            }
        }
        chars += d.text.chars().count();
        markdown.push_str(&format!("{}:\n{}\n\n", d.speaker, d.text));
    }
    match crate::script::parse_str(&markdown) {
        Ok(script) => {
            if script.dialogues.len() != draft.dialogues.len()
                || !script.directives.is_empty() || !script.warnings.is_empty()
                || script.dialogues.iter().zip(&draft.dialogues).any(|(a, b)| {
                    a.speaker != b.speaker || a.text != b.text
                })
            {
                errors.push("generated Markdown does not round-trip exactly; reserved syntax rejected".into());
            }
            let report = crate::validate::validate_script(&script, config, ws);
            errors.extend(report.errors.into_iter().map(|e| e.message));
            warnings.extend(report.warnings.into_iter().map(|w| w.message));
        }
        Err(e) => errors.push(e.to_string()),
    }
    let estimated_seconds = (chars as u64).div_ceil(5);
    if estimated_seconds.abs_diff(u64::from(brief.target_seconds))
        > u64::from(brief.target_seconds) / 5
    {
        warnings.push("estimated duration is outside target +/-20%; measure with real TTS".into());
    }
    // Bind review to source text, draft, configuration AND policy version.
    let bytes = serde_json::to_vec(&(PROMPT, brief, draft, config))
        .map_err(|e| invalid(e.to_string()))?;
    let review_hash = hex::encode(Sha256::digest(bytes));
    Ok(DraftReport {
        structurally_valid: errors.is_empty(), review_hash, errors, warnings,
        dialogue_count: draft.dialogues.len(), estimated_seconds,
        target_seconds: brief.target_seconds, markdown,
    })
}
