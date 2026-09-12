//! `videoforge.yaml` model (design §10).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const CONFIG_VERSION: u32 = 1;

// Upper bounds. These are not engine limits; they exist so that a typo in
// `videoforge.yaml` fails at load time instead of turning into an
// out-of-memory FFmpeg invocation, an hour-long hang, or a flood of parallel
// requests to the TTS engine.

/// 16384 = the largest dimension mainstream H.264 encoders accept.
const MAX_VIDEO_DIMENSION: u32 = 16384;
/// Well past any delivery format; anything higher is a typo.
const MAX_FPS: u32 = 240;
/// A minute of silence between two lines is already absurd.
const MAX_DIALOGUE_GAP_MS: u64 = 60_000;
/// VOICEVOX is a local single-process engine; more parallelism only queues.
const MAX_CONCURRENCY: usize = 16;
/// 10 minutes for one line of dialogue.
const MAX_TIMEOUT_SECS: u64 = 600;

// Voice parameter ranges are the VOICEVOX *editor* slider ranges. The engine
// API does not constrain them at all (`audioQuerySchema` types them as bare
// numbers), so values outside these bounds are accepted by the engine and
// produce unusable audio rather than an error.
// https://voicevox.hiroshiba.jp/how_to_use/
const SPEED_SCALE_RANGE: (f32, f32) = (0.5, 2.0);
const PITCH_SCALE_RANGE: (f32, f32) = (-0.15, 0.15);
const INTONATION_SCALE_RANGE: (f32, f32) = (0.0, 2.0);
const VOLUME_SCALE_RANGE: (f32, f32) = (0.0, 2.0);

// A margin past half the frame would leave no room for caption text at all;
// a font scale below/above these bounds is almost certainly a typo rather
// than an intentional micro/giant caption.
const SUBTITLE_MARGIN_FRACTION_RANGE: (f32, f32) = (0.0, 0.45);
const SUBTITLE_FONT_SCALE_RANGE: (f32, f32) = (0.3, 3.0);
const MAX_SUBTITLE_OUTLINE_WIDTH: u32 = 20;

/// Inclusive bounds check that names the offending field.
fn check_range<T>(field: &str, value: T, min: T, max: T) -> Result<(), String>
where
    T: PartialOrd + std::fmt::Display,
{
    if value < min || value > max {
        return Err(format!(
            "{field} must be between {min} and {max} (got {value})"
        ));
    }
    Ok(())
}

/// Inclusive bounds check for a float, rejecting NaN and infinities first —
/// every comparison against NaN is false, so a range check alone lets it pass.
fn check_scale(field: &str, value: f32, (min, max): (f32, f32)) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{field} must be a finite number (got {value})"));
    }
    check_range(field, value, min, max)
}

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
    /// Workspace-relative path to a standalone `videoforge-character`
    /// manifest (design §5). Optional: a workspace that never links a
    /// speaker to a character (`SpeakerConfig::character_id`) needs nothing
    /// here, and the whole character/Live2D pipeline is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_manifest: Option<String>,
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
    /// Explicit opt-in required for `endpoint` to point outside localhost
    /// (VF-004: default-deny to avoid SSRF via an agent-edited config).
    #[serde(default)]
    pub allow_remote_endpoint: bool,
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
            allow_remote_endpoint: false,
        }
    }
}

/// Reject a TTS `endpoint` that is neither `localhost` nor a loopback address
/// unless `allow_remote` (`tts.allow_remote_endpoint`) explicitly opts in.
///
/// This is a config-time guard against SSRF in workspace setups where an
/// agent may generate or edit `videoforge.yaml`; [`AppError::RemoteEndpointNotAllowed`]
/// is also enforced at TTS-engine construction time as defense in depth for
/// callers (e.g. `--endpoint` CLI overrides) that bypass config validation.
/// It does not protect against DNS rebinding of the `localhost` name itself.
pub fn check_endpoint_allowed(endpoint: &str, allow_remote: bool) -> Result<(), AppError> {
    if allow_remote {
        return Ok(());
    }
    let url = url::Url::parse(endpoint)
        .map_err(|e| AppError::Other(format!("endpoint `{endpoint}` is not a valid URL: {e}")))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    if loopback {
        Ok(())
    } else {
        Err(AppError::RemoteEndpointNotAllowed {
            endpoint: endpoint.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SpeakerConfig {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub voice: VoiceParams,
    /// Links this speaker to an entry in `Config::character_manifest`
    /// (design §5). When set and that character declares a `voice`, the
    /// character's VOICEVOX speaker/style *name* is resolved to a numeric
    /// `voice.speaker_id` at generate/doctor time instead of requiring one
    /// to be hard-coded here (`core::character::resolve_character_voices`).
    /// When absent, this speaker behaves exactly as before: a plain voice
    /// with no character/Live2D performance data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_id: Option<String>,
    /// Per-speaker caption text color override (P1-3), e.g. to give each
    /// character a distinct subtitle color. Falls back to
    /// `preview.subtitle.font_color` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_color: Option<String>,
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

impl VoiceParams {
    /// Check every scale against its VOICEVOX range. `speaker_key` only shapes
    /// the error message (`speakers.reimu.voice.speed_scale`).
    pub fn validate(&self, speaker_key: &str) -> Result<(), String> {
        let field = |name: &str| format!("speakers.{speaker_key}.voice.{name}");
        check_scale(&field("speed_scale"), self.speed_scale, SPEED_SCALE_RANGE)?;
        check_scale(&field("pitch_scale"), self.pitch_scale, PITCH_SCALE_RANGE)?;
        check_scale(
            &field("intonation_scale"),
            self.intonation_scale,
            INTONATION_SCALE_RANGE,
        )?;
        check_scale(
            &field("volume_scale"),
            self.volume_scale,
            VOLUME_SCALE_RANGE,
        )?;
        Ok(())
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
    /// Caption/subtitle styling (P1-3): position, margin, colors, outline,
    /// background box. Per-speaker color overrides live on `SpeakerConfig`.
    #[serde(default)]
    pub subtitle: SubtitleConfig,
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
            subtitle: SubtitleConfig::default(),
        }
    }
}

/// Which edge of the frame captions are anchored to. Whichever edge is
/// chosen also becomes the character-overlay "safe area" edge
/// (`videoforge_core::preview::SubtitleStyle` / `caption_safe_area_px` in
/// `videoforge-preview`), so a character can never be placed under the
/// captions regardless of which edge they render from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubtitlePosition {
    #[default]
    Bottom,
    Top,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubtitleConfig {
    #[serde(default)]
    pub position: SubtitlePosition,
    /// Fraction of frame height reserved as margin between the chosen edge
    /// and the caption text's baseline (same role `0.20` played as a
    /// hard-coded constant before P1-3).
    #[serde(default = "d_subtitle_margin_fraction")]
    pub margin_fraction: f32,
    /// Default caption text color (hex or FFmpeg color name); overridden
    /// per-speaker by `SpeakerConfig::caption_color`.
    #[serde(default = "d_subtitle_font_color")]
    pub font_color: String,
    #[serde(default = "d_subtitle_outline_color")]
    pub outline_color: String,
    #[serde(default = "d_subtitle_outline_width")]
    pub outline_width: u32,
    /// Draw a solid box behind the caption text itself (the speaker-name
    /// label above it always has one).
    #[serde(default)]
    pub background: bool,
    #[serde(default = "d_subtitle_background_color")]
    pub background_color: String,
    /// Multiplies the frame-height-relative base font size; also scales the
    /// wrap width so long lines still fit.
    #[serde(default = "one_f32")]
    pub font_scale: f32,
}
fn d_subtitle_margin_fraction() -> f32 {
    0.20
}
fn d_subtitle_font_color() -> String {
    "white".into()
}
fn d_subtitle_outline_color() -> String {
    "black".into()
}
fn d_subtitle_outline_width() -> u32 {
    3
}
fn d_subtitle_background_color() -> String {
    "0x000000AA".into()
}
fn one_f32() -> f32 {
    1.0
}
impl Default for SubtitleConfig {
    fn default() -> Self {
        Self {
            position: SubtitlePosition::default(),
            margin_fraction: d_subtitle_margin_fraction(),
            font_color: d_subtitle_font_color(),
            outline_color: d_subtitle_outline_color(),
            outline_width: d_subtitle_outline_width(),
            background: false,
            background_color: d_subtitle_background_color(),
            font_scale: one_f32(),
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
        check_range("video.width", self.video.width, 1, MAX_VIDEO_DIMENSION).map_err(invalid)?;
        check_range("video.height", self.video.height, 1, MAX_VIDEO_DIMENSION).map_err(invalid)?;
        check_range("video.fps", self.video.fps, 1, MAX_FPS).map_err(invalid)?;
        check_range(
            "timeline.dialogue_gap_ms",
            self.timeline.dialogue_gap_ms,
            0,
            MAX_DIALOGUE_GAP_MS,
        )
        .map_err(invalid)?;
        check_range("tts.concurrency", self.tts.concurrency, 1, MAX_CONCURRENCY)
            .map_err(invalid)?;
        check_range(
            "tts.timeout_secs",
            self.tts.timeout_secs,
            1,
            MAX_TIMEOUT_SECS,
        )
        .map_err(invalid)?;
        if let Err(e) = check_endpoint_allowed(&self.tts.endpoint, self.tts.allow_remote_endpoint) {
            return Err(invalid(format!("tts.endpoint: {e}")));
        }
        check_scale(
            "preview.subtitle.margin_fraction",
            self.preview.subtitle.margin_fraction,
            SUBTITLE_MARGIN_FRACTION_RANGE,
        )
        .map_err(invalid)?;
        check_scale(
            "preview.subtitle.font_scale",
            self.preview.subtitle.font_scale,
            SUBTITLE_FONT_SCALE_RANGE,
        )
        .map_err(invalid)?;
        check_range(
            "preview.subtitle.outline_width",
            self.preview.subtitle.outline_width,
            0,
            MAX_SUBTITLE_OUTLINE_WIDTH,
        )
        .map_err(invalid)?;
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
            speaker.voice.validate(key).map_err(invalid)?;
            if let Some(character_id) = &speaker.character_id {
                if character_id.trim().is_empty() {
                    return Err(invalid(format!(
                        "speakers.{key}.character_id must not be empty"
                    )));
                }
            }
        }
        if self.speakers.values().any(|s| s.character_id.is_some())
            && self
                .character_manifest
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
        {
            return Err(invalid(
                "a speaker sets character_id but no top-level character_manifest is configured"
                    .into(),
            ));
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
  # endpoint must be localhost/127.0.0.1/::1 unless you opt in here:
  # allow_remote_endpoint: true

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

# Optional: link a speaker to a character (design §5, `docs/character-system.md`).
# The character manifest is a separate, reusable file — see
# `videoforge character inspect <path>` — that names a VOICEVOX voice by
# speaker/style *name* (resolved at generate/doctor time, never a hard-coded
# id) and, optionally, a local Live2D model. Nothing below is required for a
# plain voice-only speaker.
# character_manifest: characters.yaml
#
#   tsumugi:
#     aliases:
#       - つむぎ
#       - 春日部つむぎ
#     character_id: tsumugi
#     voice:
#       speaker_id: 0   # ignored once character_id resolves a named voice

preview:
  enabled: true
  # optional background image (workspace-relative); a flat color is used if missing
  background: assets/background/default.png
  # optional font file for captions, e.g. assets/fonts/NotoSansJP-Regular.ttf
  # font:
  background_color: "#1e1e2e"
  # subtitle styling (P1-3); all fields optional, shown here at their defaults
  # subtitle:
  #   position: bottom   # or: top
  #   margin_fraction: 0.20
  #   font_color: white
  #   outline_color: black
  #   outline_width: 3
  #   background: false
  #   background_color: "0x000000AA"
  #   font_scale: 1.0

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

    #[test]
    fn character_id_requires_a_character_manifest() {
        let err = Config::parse(
            "speakers:\n  a:\n    character_id: tsumugi\n",
            Path::new("x.yaml"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("character_manifest"), "{err}");
    }

    #[test]
    fn character_id_with_manifest_is_accepted() {
        let cfg = Config::parse(
            "character_manifest: characters.yaml\nspeakers:\n  a:\n    character_id: tsumugi\n",
            Path::new("x.yaml"),
        )
        .unwrap();
        assert_eq!(cfg.speakers["a"].character_id.as_deref(), Some("tsumugi"));
        assert_eq!(cfg.character_manifest.as_deref(), Some("characters.yaml"));
    }

    #[test]
    fn plain_speakers_do_not_need_a_character_manifest() {
        // Existing configs with no character concept at all keep working
        // exactly as before.
        Config::parse(
            "speakers:\n  a:\n    voice:\n      speaker_id: 1\n",
            Path::new("x.yaml"),
        )
        .unwrap();
    }

    #[test]
    fn rejects_remote_endpoint_by_default() {
        let err = Config::parse(
            "tts:\n  endpoint: http://example.com:50021\nspeakers:\n  a: {}\n",
            Path::new("x.yaml"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("tts.endpoint"), "{err}");
        assert!(err.contains("not allowed"), "{err}");
    }

    #[test]
    fn allows_remote_endpoint_when_opted_in() {
        let cfg = Config::parse(
            "tts:\n  endpoint: http://example.com:50021\n  allow_remote_endpoint: true\nspeakers:\n  a: {}\n",
            Path::new("x.yaml"),
        )
        .unwrap();
        assert_eq!(cfg.tts.endpoint, "http://example.com:50021");
    }

    #[test]
    fn accepts_loopback_variants() {
        for endpoint in [
            "http://127.0.0.1:50021",
            "http://localhost:50021",
            "http://LOCALHOST:50021",
            "http://[::1]:50021",
        ] {
            check_endpoint_allowed(endpoint, false).unwrap_or_else(|e| {
                panic!("expected {endpoint} to be allowed, got {e}");
            });
        }
    }

    #[test]
    fn rejects_non_loopback_hosts() {
        for endpoint in [
            "http://example.com:50021",
            "http://169.254.169.254/latest/meta-data",
            "http://0.0.0.0:50021",
        ] {
            assert!(
                check_endpoint_allowed(endpoint, false).is_err(),
                "expected {endpoint} to be rejected"
            );
        }
    }

    /// `speakers.a` with `voice.<field>: <value>` spliced in.
    fn config_with_voice(field: &str, value: &str) -> Result<Config, AppError> {
        Config::parse(
            &format!("speakers:\n  a:\n    voice:\n      {field}: {value}\n"),
            Path::new("x.yaml"),
        )
    }

    fn voice_error(field: &str, value: &str) -> String {
        config_with_voice(field, value)
            .expect_err(&format!("{field}: {value} must be rejected"))
            .to_string()
    }

    #[test]
    fn voice_scales_accept_their_documented_bounds() {
        for (field, value) in [
            ("speed_scale", "0.5"),
            ("speed_scale", "2.0"),
            ("pitch_scale", "-0.15"),
            ("pitch_scale", "0.15"),
            ("intonation_scale", "0.0"),
            ("intonation_scale", "2.0"),
            ("volume_scale", "0.0"),
            ("volume_scale", "2.0"),
        ] {
            assert!(
                config_with_voice(field, value).is_ok(),
                "{field}: {value} is a documented bound and must be accepted"
            );
        }
    }

    #[test]
    fn voice_scales_reject_values_past_their_bounds() {
        for (field, value) in [
            ("speed_scale", "0.49"),
            ("speed_scale", "2.01"),
            ("speed_scale", "0.0"),
            ("pitch_scale", "-0.16"),
            ("pitch_scale", "0.16"),
            ("intonation_scale", "-0.01"),
            ("intonation_scale", "2.01"),
            ("volume_scale", "-0.01"),
            ("volume_scale", "2.01"),
        ] {
            let message = voice_error(field, value);
            assert!(
                message.contains(&format!("speakers.a.voice.{field}")),
                "error for {field}: {value} must name the field, got: {message}"
            );
        }
    }

    #[test]
    fn voice_scales_reject_nan_and_infinity() {
        for (field, value) in [
            ("speed_scale", ".nan"),
            ("speed_scale", ".inf"),
            ("speed_scale", "-.inf"),
            ("pitch_scale", ".nan"),
            ("intonation_scale", ".nan"),
            ("volume_scale", "-.inf"),
            // f32 overflow: serde_yaml parses this as f64 and narrows to inf.
            ("speed_scale", "1e40"),
        ] {
            let message = voice_error(field, value);
            assert_eq!(
                message,
                format!(
                    "invalid config x.yaml: speakers.a.voice.{field} must be a finite number (got {})",
                    if value.contains("nan") { "NaN" } else if value.starts_with('-') { "-inf" } else { "inf" }
                ),
                "unexpected message for {field}: {value}"
            );
        }
    }

    #[test]
    fn numeric_settings_reject_out_of_range_values() {
        let bad = |yaml: &str| {
            Config::parse(
                &format!("{yaml}\nspeakers:\n  a: {{}}\n"),
                Path::new("x.yaml"),
            )
            .unwrap_err()
            .to_string()
        };
        assert_eq!(
            bad("video:\n  fps: 0"),
            "invalid config x.yaml: video.fps must be between 1 and 240 (got 0)"
        );
        assert_eq!(
            bad("video:\n  fps: 241"),
            "invalid config x.yaml: video.fps must be between 1 and 240 (got 241)"
        );
        assert_eq!(
            bad("video:\n  width: 16385"),
            "invalid config x.yaml: video.width must be between 1 and 16384 (got 16385)"
        );
        assert_eq!(
            bad("video:\n  height: 0"),
            "invalid config x.yaml: video.height must be between 1 and 16384 (got 0)"
        );
        assert_eq!(
            bad("tts:\n  concurrency: 0"),
            "invalid config x.yaml: tts.concurrency must be between 1 and 16 (got 0)"
        );
        assert_eq!(
            bad("tts:\n  concurrency: 17"),
            "invalid config x.yaml: tts.concurrency must be between 1 and 16 (got 17)"
        );
        assert_eq!(
            bad("tts:\n  timeout_secs: 0"),
            "invalid config x.yaml: tts.timeout_secs must be between 1 and 600 (got 0)"
        );
        assert_eq!(
            bad("timeline:\n  dialogue_gap_ms: 60001"),
            "invalid config x.yaml: timeline.dialogue_gap_ms must be between 0 and 60000 (got 60001)"
        );
    }

    #[test]
    fn numeric_settings_accept_their_bounds() {
        let ok = |yaml: &str| {
            Config::parse(
                &format!("{yaml}\nspeakers:\n  a: {{}}\n"),
                Path::new("x.yaml"),
            )
            .is_ok()
        };
        assert!(ok("video:\n  fps: 1\n  width: 1\n  height: 1"));
        assert!(ok("video:\n  fps: 240\n  width: 16384\n  height: 16384"));
        assert!(ok("tts:\n  concurrency: 16\n  timeout_secs: 600"));
        assert!(ok("timeline:\n  dialogue_gap_ms: 0"));
    }

    #[test]
    fn subtitle_config_defaults_when_omitted() {
        let cfg = Config::parse(&default_config_yaml("demo"), Path::new("x.yaml")).unwrap();
        assert_eq!(cfg.preview.subtitle.position, SubtitlePosition::Bottom);
        assert_eq!(cfg.preview.subtitle.margin_fraction, 0.20);
        assert_eq!(cfg.preview.subtitle.font_color, "white");
        assert_eq!(cfg.preview.subtitle.outline_color, "black");
        assert_eq!(cfg.preview.subtitle.outline_width, 3);
        assert!(!cfg.preview.subtitle.background);
        assert_eq!(cfg.preview.subtitle.font_scale, 1.0);
    }

    #[test]
    fn subtitle_config_can_be_fully_overridden() {
        let yaml = "speakers:\n  a: {}\npreview:\n  subtitle:\n    position: top\n    margin_fraction: 0.1\n    font_color: yellow\n    outline_color: blue\n    outline_width: 5\n    background: true\n    background_color: \"0x000000FF\"\n    font_scale: 1.5\n";
        let cfg = Config::parse(yaml, Path::new("x.yaml")).unwrap();
        assert_eq!(cfg.preview.subtitle.position, SubtitlePosition::Top);
        assert_eq!(cfg.preview.subtitle.margin_fraction, 0.1);
        assert_eq!(cfg.preview.subtitle.font_color, "yellow");
        assert_eq!(cfg.preview.subtitle.outline_color, "blue");
        assert_eq!(cfg.preview.subtitle.outline_width, 5);
        assert!(cfg.preview.subtitle.background);
        assert_eq!(cfg.preview.subtitle.background_color, "0x000000FF");
        assert_eq!(cfg.preview.subtitle.font_scale, 1.5);
    }

    #[test]
    fn subtitle_config_rejects_out_of_range_values() {
        let bad = |yaml: &str| {
            Config::parse(
                &format!("speakers:\n  a: {{}}\n{yaml}"),
                Path::new("x.yaml"),
            )
            .unwrap_err()
            .to_string()
        };
        assert!(bad("preview:\n  subtitle:\n    margin_fraction: 0.9\n")
            .contains("preview.subtitle.margin_fraction"));
        assert!(bad("preview:\n  subtitle:\n    font_scale: 10.0\n")
            .contains("preview.subtitle.font_scale"));
        assert!(bad("preview:\n  subtitle:\n    outline_width: 100\n")
            .contains("preview.subtitle.outline_width"));
    }

    #[test]
    fn speaker_caption_color_override_is_optional() {
        let cfg = Config::parse(
            "speakers:\n  a:\n    caption_color: \"#ff0000\"\n  b: {}\n",
            Path::new("x.yaml"),
        )
        .unwrap();
        assert_eq!(cfg.speakers["a"].caption_color.as_deref(), Some("#ff0000"));
        assert_eq!(cfg.speakers["b"].caption_color, None);
    }
}
