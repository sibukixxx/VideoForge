//! `videoforge.yaml` model (design §10).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub timeline: TimelineConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub speakers: BTreeMap<String, SpeakerConfig>,
    #[serde(default)]
    pub preview: PreviewConfig,
    #[serde(default)]
    pub export: ExportConfig,
}

fn default_version() -> u32 {
    CONFIG_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoConfig {
    #[serde(default = "d_width")]
    pub width: u32,
    #[serde(default = "d_height")]
    pub height: u32,
    #[serde(default = "d_fps")]
    pub fps: u32,
}
fn d_width() -> u32 {
    1920
}
fn d_height() -> u32 {
    1080
}
fn d_fps() -> u32 {
    30
}
impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            width: d_width(),
            height: d_height(),
            fps: d_fps(),
        }
    }
}
impl From<VideoConfig> for videoforge_project::VideoSettings {
    fn from(v: VideoConfig) -> Self {
        Self {
            width: v.width,
            height: v.height,
            fps: v.fps,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimelineConfig {
    #[serde(default = "d_gap")]
    pub dialogue_gap_ms: u64,
}
fn d_gap() -> u64 {
    200
}
impl Default for TimelineConfig {
    fn default() -> Self {
        Self {
            dialogue_gap_ms: d_gap(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsConfig {
    #[serde(default = "d_engine")]
    pub engine: String,
    #[serde(default = "d_endpoint")]
    pub endpoint: String,
    #[serde(default = "d_concurrency")]
    pub concurrency: usize,
    /// Per-request timeout in seconds.
    #[serde(default = "d_timeout")]
    pub timeout_secs: u64,
}
fn d_engine() -> String {
    "voicevox".into()
}
fn d_endpoint() -> String {
    "http://127.0.0.1:50021".into()
}
fn d_concurrency() -> usize {
    1
}
fn d_timeout() -> u64 {
    120
}
impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            engine: d_engine(),
            endpoint: d_endpoint(),
            concurrency: d_concurrency(),
            timeout_secs: d_timeout(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SpeakerConfig {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub voice: VoiceParams,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceParams {
    /// VOICEVOX style id.
    #[serde(default)]
    pub speaker_id: u32,
    #[serde(default = "one")]
    pub speed_scale: f32,
    #[serde(default)]
    pub pitch_scale: f32,
    #[serde(default = "one")]
    pub intonation_scale: f32,
    #[serde(default = "one")]
    pub volume_scale: f32,
}
fn one() -> f32 {
    1.0
}
impl Default for VoiceParams {
    fn default() -> Self {
        Self {
            speaker_id: 0,
            speed_scale: 1.0,
            pitch_scale: 0.0,
            intonation_scale: 1.0,
            volume_scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewConfig {
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Workspace-relative background image. Optional; a flat color is used
    /// when missing.
    #[serde(default)]
    pub background: Option<String>,
    /// Optional font file for caption rendering (workspace-relative or absolute).
    #[serde(default)]
    pub font: Option<String>,
    /// Background color used when no image is available (hex, e.g. `#1e1e2e`).
    #[serde(default = "d_bg_color")]
    pub background_color: String,
}
fn yes() -> bool {
    true
}
fn d_bg_color() -> String {
    "#1e1e2e".into()
}
impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            background: None,
            font: None,
            background_color: d_bg_color(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ExportConfig {
    #[serde(default)]
    pub ymm4: Ymm4ExportConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ymm4ExportConfig {
    /// Workspace-relative path of the YMM4 template project.
    #[serde(default = "d_template")]
    pub template: String,
}
fn d_template() -> String {
    "templates/ymm4/default.ymmp".into()
}
impl Default for Ymm4ExportConfig {
    fn default() -> Self {
        Self {
            template: d_template(),
        }
    }
}

/// Result of resolving a script speaker name against the config.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSpeaker<'a> {
    pub key: &'a str,
    pub config: &'a SpeakerConfig,
}

impl Config {
    pub fn parse(yaml: &str, path: &Path) -> Result<Self, AppError> {
        let config: Config = serde_yaml::from_str(yaml).map_err(|e| AppError::InvalidConfig {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        config.validate(path)?;
        Ok(config)
    }

    pub fn load(path: &Path) -> Result<Self, AppError> {
        let text = std::fs::read_to_string(path).map_err(|e| AppError::read(path, e))?;
        Self::parse(&text, path)
    }

    pub fn validate(&self, path: &Path) -> Result<(), AppError> {
        let invalid = |reason: String| AppError::InvalidConfig {
            path: path.to_path_buf(),
            reason,
        };
        if self.version != CONFIG_VERSION {
            return Err(invalid(format!(
                "unsupported config version {} (expected {CONFIG_VERSION})",
                self.version
            )));
        }
        if self.video.fps == 0 || self.video.width == 0 || self.video.height == 0 {
            return Err(invalid("video.width/height/fps must be > 0".into()));
        }
        if self.tts.concurrency == 0 {
            return Err(invalid("tts.concurrency must be >= 1".into()));
        }
        if self.speakers.is_empty() {
            return Err(invalid("at least one speaker must be configured".into()));
        }
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        for (key, speaker) in &self.speakers {
            if key.trim().is_empty() || key.chars().any(char::is_whitespace) {
                return Err(invalid(format!(
                    "speaker key `{key}` must not contain whitespace"
                )));
            }
            for name in
                std::iter::once(key.as_str()).chain(speaker.aliases.iter().map(String::as_str))
            {
                if let Some(prev) = seen.insert(name.to_string(), key) {
                    if prev != key {
                        return Err(invalid(format!(
                            "speaker name `{name}` is used by both `{prev}` and `{key}`"
                        )));
                    }
                }
            }
            if speaker.voice.speed_scale <= 0.0 {
                return Err(invalid(format!(
                    "speakers.{key}.voice.speed_scale must be > 0"
                )));
            }
        }
        Ok(())
    }

    /// Resolve a name from the script (key or alias) to its canonical key.
    pub fn resolve_speaker(&self, name: &str) -> Option<ResolvedSpeaker<'_>> {
        let name = name.trim();
        if let Some((key, config)) = self.speakers.get_key_value(name) {
            return Some(ResolvedSpeaker { key, config });
        }
        self.speakers
            .iter()
            .find(|(key, s)| {
                key.eq_ignore_ascii_case(name) || s.aliases.iter().any(|a| a.trim() == name)
            })
            .map(|(key, config)| ResolvedSpeaker { key, config })
    }

    /// All names a script may use, for error messages.
    pub fn known_speaker_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for (key, s) in &self.speakers {
            names.push(key.clone());
            names.extend(s.aliases.iter().cloned());
        }
        names
    }
}

/// The `videoforge.yaml` written by `videoforge init`.
pub fn default_config_yaml(project_name: &str) -> String {
    format!(
        r##"# VideoForge workspace configuration
version: 1

project:
  name: {project_name}

video:
  width: 1920
  height: 1080
  fps: 30

timeline:
  # silence inserted between consecutive dialogues
  dialogue_gap_ms: 200

tts:
  engine: voicevox
  endpoint: http://127.0.0.1:50021
  concurrency: 1
  timeout_secs: 120

# Speaker keys are canonical ids used in project.vfp.json.
# Aliases are the names you write in scripts (e.g. `霊夢:`).
# speaker_id is the VOICEVOX *style* id (see `videoforge doctor` / GET /speakers).
speakers:
  reimu:
    aliases:
      - 霊夢
    voice:
      speaker_id: 2
      speed_scale: 1.05
      pitch_scale: 0.0
      intonation_scale: 1.0

  marisa:
    aliases:
      - 魔理沙
    voice:
      speaker_id: 3
      speed_scale: 1.08
      pitch_scale: 0.0
      intonation_scale: 1.0

preview:
  enabled: true
  # optional background image (workspace-relative); a flat color is used if missing
  background: assets/background/default.png
  # optional font file for captions, e.g. assets/fonts/NotoSansJP-Regular.ttf
  # font:
  background_color: "#1e1e2e"

export:
  ymm4:
    template: templates/ymm4/default.ymmp
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_yaml_parses_and_validates() {
        let cfg =
            Config::parse(&default_config_yaml("demo"), Path::new("videoforge.yaml")).unwrap();
        assert_eq!(cfg.project.name, "demo");
        assert_eq!(cfg.video.fps, 30);
        assert_eq!(cfg.timeline.dialogue_gap_ms, 200);
        assert_eq!(cfg.tts.concurrency, 1);
        let r = cfg.resolve_speaker("霊夢").unwrap();
        assert_eq!(r.key, "reimu");
        assert_eq!(r.config.voice.speaker_id, 2);
        assert_eq!(cfg.resolve_speaker("marisa").unwrap().key, "marisa");
        assert_eq!(cfg.resolve_speaker("MARISA").unwrap().key, "marisa");
        assert!(cfg.resolve_speaker("nobody").is_none());
        assert_eq!(cfg.export.ymm4.template, "templates/ymm4/default.ymmp");
    }

    #[test]
    fn minimal_config_uses_defaults() {
        let cfg = Config::parse(
            "speakers:\n  a:\n    voice:\n      speaker_id: 1\n",
            Path::new("x.yaml"),
        )
        .unwrap();
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.video.width, 1920);
        assert_eq!(cfg.tts.endpoint, "http://127.0.0.1:50021");
        assert!(cfg.preview.enabled);
        assert_eq!(cfg.speakers["a"].voice.speed_scale, 1.0);
    }

    #[test]
    fn rejects_bad_configs() {
        let bad = |yaml: &str| {
            Config::parse(yaml, Path::new("x.yaml"))
                .unwrap_err()
                .to_string()
        };
        assert!(bad("speakers: {}").contains("at least one speaker"));
        assert!(bad("video:\n  fps: 0\nspeakers:\n  a: {}\n").contains("fps"));
        assert!(
            bad("speakers:\n  a:\n    aliases: [x]\n  b:\n    aliases: [x]\n")
                .contains("used by both")
        );
        assert!(bad("version: 2\nspeakers:\n  a: {}\n").contains("version"));
        assert!(bad("unknown_key: 1\nspeakers:\n  a: {}\n").contains("unknown"));
    }
}
