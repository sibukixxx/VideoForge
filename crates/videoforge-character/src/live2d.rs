//! Metadata-only Live2D `model3.json` reader.
//!
//! This does **not** load, validate meshes, or render a Live2D model — that
//! needs the (proprietary) Cubism runtime and is out of scope for P0 (see
//! `docs/live2d-renderer-decision.md`). It reads the small amount of JSON
//! structure needed to validate a character manifest before generation
//! starts (design §21): the list of expression names and motion group names
//! a script is allowed to reference.

use std::path::Path;

use serde::Deserialize;

use crate::CharacterError;

/// Expressions and motion groups declared by a `model3.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Live2dModelInfo {
    pub expressions: Vec<String>,
    /// Live2D Cubism groups motions by name (e.g. `"Idle"`, `"TapBody"`);
    /// individual motion files inside a group are not required to have their
    /// own name, so the group name is what a script can reference.
    pub motions: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Model3Json {
    #[serde(default, rename = "FileReferences")]
    file_references: FileReferences,
}

#[derive(Debug, Default, Deserialize)]
struct FileReferences {
    #[serde(default, rename = "Expressions")]
    expressions: Vec<ExpressionEntry>,
    #[serde(default, rename = "Motions")]
    motions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ExpressionEntry {
    #[serde(rename = "Name")]
    name: String,
}

/// Read `expressions`/`motions` out of a `model3.json` at `path`.
pub fn load_model3_json(path: &Path) -> Result<Live2dModelInfo, CharacterError> {
    if !path.is_file() {
        return Err(CharacterError::ModelNotFound {
            path: path.to_path_buf(),
        });
    }
    let text = std::fs::read_to_string(path).map_err(|e| CharacterError::ModelInvalid {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let parsed: Model3Json =
        serde_json::from_str(&text).map_err(|e| CharacterError::ModelInvalid {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
    let mut expressions: Vec<String> = parsed
        .file_references
        .expressions
        .into_iter()
        .map(|e| e.name)
        .collect();
    let mut motions: Vec<String> = parsed.file_references.motions.keys().cloned().collect();
    expressions.sort();
    motions.sort();
    Ok(Live2dModelInfo {
        expressions,
        motions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn reads_expressions_and_motion_groups() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "model3.json",
            r#"{
                "Version": 3,
                "FileReferences": {
                    "Moc": "model.moc3",
                    "Expressions": [
                        {"Name": "smile", "File": "exp/smile.exp3.json"},
                        {"Name": "surprised", "File": "exp/surprised.exp3.json"}
                    ],
                    "Motions": {
                        "Idle": [{"File": "motions/idle_01.motion3.json"}],
                        "TapBody": [{"File": "motions/tap_01.motion3.json"}]
                    }
                }
            }"#,
        );
        let info = load_model3_json(&path).unwrap();
        assert_eq!(info.expressions, vec!["smile", "surprised"]);
        assert_eq!(info.motions, vec!["Idle", "TapBody"]);
    }

    #[test]
    fn missing_file_is_model_not_found() {
        let err = load_model3_json(Path::new("/definitely/not/a/model3.json")).unwrap_err();
        assert!(matches!(err, CharacterError::ModelNotFound { .. }));
    }

    #[test]
    fn garbage_json_is_model_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "model3.json", "not json");
        let err = load_model3_json(&path).unwrap_err();
        assert!(matches!(err, CharacterError::ModelInvalid { .. }));
    }

    #[test]
    fn model_with_no_expressions_or_motions_is_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "model3.json",
            r#"{"Version": 3, "FileReferences": {"Moc": "model.moc3"}}"#,
        );
        let info = load_model3_json(&path).unwrap();
        assert!(info.expressions.is_empty());
        assert!(info.motions.is_empty());
    }
}
