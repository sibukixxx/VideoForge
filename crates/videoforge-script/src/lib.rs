//! Script parser.
//!
//! MVP format: Markdown with optional YAML front matter. Dialogue blocks are
//!
//! ```markdown
//! 霊夢:
//! こんにちは。
//!
//! 魔理沙[emotion=surprise]:
//! これはかなりヤバいぜ。
//! ```
//!
//! * A line that is exactly `Name:` (or `Name[key=value,...]:`, ASCII or
//!   full-width colon) starts a new dialogue.
//! * Following non-empty lines are the dialogue text (joined with `\n`).
//! * `#` heading lines are comments.
//! * `@directive ...` lines are reserved for the future DSL; v0.1 records a
//!   warning and skips them (including everything up to `@end` for block
//!   directives such as `@human`).
//! * Text before the first speaker is an error.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ScriptError {
    #[error("failed to read script {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("front matter opened with '---' on line 1 but never closed")]
    UnterminatedFrontMatter,
    #[error("invalid YAML front matter: {0}")]
    InvalidFrontMatter(String),
    #[error("line {line}: text without a speaker (expected `Speaker:` on its own line first)")]
    TextWithoutSpeaker { line: usize },
    #[error("line {line}: speaker `{speaker}` has no text")]
    EmptyDialogue { speaker: String, line: usize },
    #[error("script contains no dialogue")]
    NoDialogue,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FrontMatter {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dialogue {
    /// 1-based dialogue number in script order.
    pub index: usize,
    /// Speaker as written in the script (alias, not yet resolved).
    pub speaker: String,
    /// Inline attributes such as `emotion=surprise`.
    #[serde(default)]
    pub attributes: BTreeMap<String, String>,
    pub text: String,
    /// 1-based line of the speaker header.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub line: usize,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub front_matter: FrontMatter,
    pub dialogues: Vec<Dialogue>,
    pub warnings: Vec<Warning>,
    #[serde(default)]
    pub source_path: Option<PathBuf>,
}

impl Script {
    /// Title from front matter, else the file stem, else "untitled".
    pub fn title(&self) -> String {
        if let Some(t) = &self.front_matter.title {
            if !t.trim().is_empty() {
                return t.trim().to_string();
            }
        }
        self.source_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string())
    }

    /// Slug used for the output directory: the script file stem.
    pub fn slug(&self) -> String {
        self.source_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| slugify(&s.to_string_lossy()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "untitled".to_string())
    }
}

/// Make a filesystem-safe slug. Keeps unicode letters/digits, `-`, `_`.
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.trim().chars() {
        if ch.is_alphanumeric() || ch == '_' {
            out.push(ch);
            last_dash = false;
        } else if (ch == '-' || ch.is_whitespace() || ch == '.') && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn parse_file(path: &Path) -> Result<Script, ScriptError> {
    let text = std::fs::read_to_string(path).map_err(|source| ScriptError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut script = parse_str(&text)?;
    script.source_path = Some(path.to_path_buf());
    Ok(script)
}

pub fn parse_str(input: &str) -> Result<Script, ScriptError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let (front_matter, body, body_offset) = split_front_matter(input)?;

    let mut dialogues: Vec<Dialogue> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();
    let mut current: Option<(Dialogue, Vec<String>)> = None;
    let mut skipping_block: Option<(String, usize)> = None;

    let flush = |current: &mut Option<(Dialogue, Vec<String>)>,
                 dialogues: &mut Vec<Dialogue>|
     -> Result<(), ScriptError> {
        if let Some((mut d, lines)) = current.take() {
            let text = lines.join("\n").trim().to_string();
            if text.is_empty() {
                return Err(ScriptError::EmptyDialogue {
                    speaker: d.speaker,
                    line: d.line,
                });
            }
            d.text = text;
            d.index = dialogues.len() + 1;
            dialogues.push(d);
        }
        Ok(())
    };

    for (i, raw_line) in body.lines().enumerate() {
        let line_no = body_offset + i + 1;
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if let Some((name, start)) = &skipping_block {
            if trimmed == "@end" {
                warnings.push(Warning {
                    line: *start,
                    message: format!(
                        "block directive `@{name}` is not supported in v0.1 and was skipped"
                    ),
                });
                skipping_block = None;
            }
            continue;
        }

        if trimmed.is_empty() {
            if let Some((_, lines)) = current.as_mut() {
                if !lines.is_empty() {
                    lines.push(String::new());
                }
            }
            continue;
        }

        if trimmed.starts_with('#') {
            // Markdown heading: treated as a comment.
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix('@') {
            let name = rest
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                warnings.push(Warning {
                    line: line_no,
                    message: "empty directive `@` was ignored".into(),
                });
                continue;
            }
            if BLOCK_DIRECTIVES.contains(&name.as_str()) {
                skipping_block = Some((name, line_no));
            } else {
                warnings.push(Warning {
                    line: line_no,
                    message: format!(
                        "directive `@{name}` is not supported in v0.1 and was skipped"
                    ),
                });
            }
            continue;
        }

        if let Some((speaker, attributes)) = parse_speaker_header(trimmed) {
            flush(&mut current, &mut dialogues)?;
            current = Some((
                Dialogue {
                    index: 0,
                    speaker,
                    attributes,
                    text: String::new(),
                    line: line_no,
                },
                Vec::new(),
            ));
            continue;
        }

        match current.as_mut() {
            Some((_, lines)) => lines.push(trimmed.to_string()),
            None => return Err(ScriptError::TextWithoutSpeaker { line: line_no }),
        }
    }

    if let Some((name, start)) = skipping_block {
        warnings.push(Warning {
            line: start,
            message: format!(
                "block directive `@{name}` was never closed with `@end`; rest of file skipped"
            ),
        });
    }

    flush(&mut current, &mut dialogues)?;

    if dialogues.is_empty() {
        return Err(ScriptError::NoDialogue);
    }

    Ok(Script {
        front_matter,
        dialogues,
        warnings,
        source_path: None,
    })
}

const BLOCK_DIRECTIVES: &[&str] = &["human"];

/// Split off a leading `---\n...\n---` block. Returns (front matter, body,
/// number of lines consumed before the body).
fn split_front_matter(input: &str) -> Result<(FrontMatter, &str, usize), ScriptError> {
    let mut lines = input.split_inclusive('\n');
    let first = match lines.next() {
        Some(l) => l,
        None => return Ok((FrontMatter::default(), input, 0)),
    };
    if first.trim_end() != "---" {
        return Ok((FrontMatter::default(), input, 0));
    }

    let mut consumed = first.len();
    let mut line_count = 1;
    for line in lines {
        line_count += 1;
        if line.trim_end() == "---" {
            let yaml = &input[first.len()..consumed];
            let body = &input[consumed + line.len()..];
            let fm: FrontMatter = if yaml.trim().is_empty() {
                FrontMatter::default()
            } else {
                serde_yaml::from_str(yaml)
                    .map_err(|e| ScriptError::InvalidFrontMatter(e.to_string()))?
            };
            return Ok((fm, body, line_count));
        }
        consumed += line.len();
    }
    Err(ScriptError::UnterminatedFrontMatter)
}

/// `Name:` / `Name：` / `Name[k=v, k2=v2]:` → (name, attributes)
fn parse_speaker_header(line: &str) -> Option<(String, BTreeMap<String, String>)> {
    let head = line
        .strip_suffix(':')
        .or_else(|| line.strip_suffix('：'))?
        .trim_end();
    if head.is_empty() {
        return None;
    }
    // Reject things that look like URLs or prose ("see http", "注意 事項").
    let (name, attrs) = match head.find('[') {
        Some(open) => {
            let close = head.rfind(']')?;
            if close < open {
                return None;
            }
            (head[..open].trim(), Some(&head[open + 1..close]))
        }
        None => (head, None),
    };
    if name.is_empty() || name.chars().any(char::is_whitespace) || name.len() > 64 {
        return None;
    }
    if name.contains(['/', '\\', '@', '"', '\'', '<', '>']) {
        return None;
    }

    let mut attributes = BTreeMap::new();
    if let Some(attrs) = attrs {
        for pair in attrs.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            match pair.split_once('=') {
                Some((k, v)) => {
                    attributes.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
                }
                None => {
                    attributes.insert(pair.to_string(), "true".to_string());
                }
            }
        }
    }
    Some((name.to_string(), attributes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\ntitle: 半年前、AIエージェントはまだ苦戦していた\ntemplate: yukkuri-tech\n---\n\n霊夢:\n半年前までは、最先端のAIでもかなり苦戦していました。\n\n魔理沙:\nところが今は状況がかなり変わっているぜ。\n\n霊夢:\n今日は何が変わったのか整理してみます。\n";

    #[test]
    fn parses_front_matter_and_dialogues() {
        let s = parse_str(SAMPLE).unwrap();
        assert_eq!(
            s.front_matter.title.as_deref(),
            Some("半年前、AIエージェントはまだ苦戦していた")
        );
        assert_eq!(s.front_matter.template.as_deref(), Some("yukkuri-tech"));
        assert_eq!(s.dialogues.len(), 3);
        assert_eq!(s.dialogues[0].speaker, "霊夢");
        assert_eq!(s.dialogues[0].index, 1);
        assert_eq!(s.dialogues[0].line, 6);
        assert_eq!(
            s.dialogues[1].text,
            "ところが今は状況がかなり変わっているぜ。"
        );
        assert_eq!(s.dialogues[2].index, 3);
        assert!(s.warnings.is_empty());
    }

    #[test]
    fn works_without_front_matter() {
        let s = parse_str("霊夢:\nこんにちは。\n").unwrap();
        assert_eq!(s.front_matter, FrontMatter::default());
        assert_eq!(s.dialogues.len(), 1);
        assert_eq!(s.title(), "untitled");
    }

    #[test]
    fn multiline_text_is_joined() {
        let s = parse_str("霊夢:\n一行目。\n二行目。\n\n三行目。\n").unwrap();
        assert_eq!(s.dialogues[0].text, "一行目。\n二行目。\n\n三行目。");
    }

    #[test]
    fn headings_are_comments() {
        let s = parse_str("# 導入\n霊夢:\nこんにちは。\n## 本編\n魔理沙:\nやあ。\n").unwrap();
        assert_eq!(s.dialogues.len(), 2);
    }

    #[test]
    fn full_width_colon_and_attributes() {
        let s = parse_str("魔理沙[emotion=surprise, loud]：\nヤバいぜ。\n").unwrap();
        let d = &s.dialogues[0];
        assert_eq!(d.speaker, "魔理沙");
        assert_eq!(
            d.attributes.get("emotion").map(String::as_str),
            Some("surprise")
        );
        assert_eq!(d.attributes.get("loud").map(String::as_str), Some("true"));
    }

    #[test]
    fn text_before_speaker_is_error() {
        let err = parse_str("こんにちは。\n霊夢:\nやあ\n").unwrap_err();
        assert!(matches!(err, ScriptError::TextWithoutSpeaker { line: 1 }));
    }

    #[test]
    fn empty_dialogue_is_error() {
        let err = parse_str("霊夢:\n\n魔理沙:\nやあ\n").unwrap_err();
        assert!(matches!(err, ScriptError::EmptyDialogue { line: 1, .. }));
        let err = parse_str("霊夢:\n").unwrap_err();
        assert!(matches!(err, ScriptError::EmptyDialogue { .. }));
    }

    #[test]
    fn no_dialogue_is_error() {
        assert!(matches!(
            parse_str("# only heading\n"),
            Err(ScriptError::NoDialogue)
        ));
    }

    #[test]
    fn unknown_directives_warn_and_are_skipped() {
        let src = "霊夢:\nこんにちは。\n\n@pause 500ms\n\n@image src=\"assets/image/chart.png\"\n\n@human id=\"x\"\nここに本人の経験\n@end\n\n魔理沙:\nやあ。\n";
        let s = parse_str(src).unwrap();
        assert_eq!(s.dialogues.len(), 2);
        assert_eq!(s.warnings.len(), 3);
        assert!(s.warnings[0].message.contains("@pause"));
        assert!(s.warnings[2].message.contains("@human"));
        assert_eq!(s.dialogues[0].text, "こんにちは。");
    }

    #[test]
    fn unterminated_front_matter() {
        assert!(matches!(
            parse_str("---\ntitle: x\n霊夢:\nやあ\n"),
            Err(ScriptError::UnterminatedFrontMatter)
        ));
    }

    #[test]
    fn invalid_front_matter_yaml() {
        assert!(matches!(
            parse_str("---\ntitle: [unclosed\n---\n霊夢:\nやあ\n"),
            Err(ScriptError::InvalidFrontMatter(_))
        ));
    }

    #[test]
    fn prose_with_trailing_colon_is_not_a_speaker() {
        // Contains whitespace → treated as text, not a speaker header.
        let s = parse_str("霊夢:\n次の 項目:\n続き\n").unwrap();
        assert_eq!(s.dialogues[0].text, "次の 項目:\n続き");
    }

    #[test]
    fn bom_is_stripped() {
        let s = parse_str("\u{feff}霊夢:\nやあ\n").unwrap();
        assert_eq!(s.dialogues[0].speaker, "霊夢");
    }

    #[test]
    fn slug_and_title_from_path() {
        let mut s = parse_str("霊夢:\nやあ\n").unwrap();
        s.source_path = Some(PathBuf::from("scripts/001 AI News.md"));
        assert_eq!(s.slug(), "001-AI-News");
        assert_eq!(s.title(), "001 AI News");
        assert_eq!(slugify("  --a..b  "), "a-b");
    }
}
