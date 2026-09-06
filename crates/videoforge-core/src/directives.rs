//! Resolve script directives (`@image`, `@character`, `@bgm`, `@se`,
//! `@transition`) against the workspace and config (design §17.1, issue #18).
//!
//! Shared by `validate` (reports the issues) and `generate` (copies the
//! assets and places the clips), so both see exactly the same problems.
//! Nothing here is fatal: a missing asset, an unknown attribute or a
//! directive with no dialogue after it becomes a warning and the directive
//! is skipped, mirroring how bundle/export treat missing files. Only a path
//! that could escape the workspace is an error.
//!
//! Character images follow a convention instead of a config key:
//! `@character <name>[expression=<e>]` → `assets/character/<key>/<e>.png`
//! where `<key>` is the canonical speaker key when `<name>` is a speaker or
//! alias, else `<name>` itself, and `<e>` defaults to `default`. `src=<path>`
//! overrides the whole path.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use videoforge_project::{FitMode, Presentation, RelativeAssetPath, Transform};
use videoforge_script::{DirectiveKind, Script};
use videoforge_timeline::{VisualEvent, VisualEventKind};

use crate::config::Config;
use crate::validate::ValidationIssue;
use crate::workspace::Workspace;

/// A directive whose asset path has been checked against the workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedDirective {
    /// 1-based script line.
    pub line: usize,
    /// Workspace-relative asset path. The same relative path is used inside
    /// `generated/<slug>/`, so it doubles as the project asset path.
    pub source: RelativeAssetPath,
    /// Whether the asset exists in the workspace. Missing assets are
    /// reported as warnings and skipped at generate time.
    pub exists: bool,
    pub event: VisualEvent,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DirectiveResolution {
    pub directives: Vec<ResolvedDirective>,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
}

pub fn resolve_directives(
    script: &Script,
    config: &Config,
    workspace: &Workspace,
) -> DirectiveResolution {
    let mut out = DirectiveResolution::default();
    let mut transitions = Vec::new();

    for d in &script.directives {
        let (name, path, attributes) = match &d.kind {
            DirectiveKind::Image { path, attributes } => ("@image", path.clone(), attributes),
            DirectiveKind::Bgm { path, attributes } => ("@bgm", path.clone(), attributes),
            DirectiveKind::Se { path, attributes } => ("@se", path.clone(), attributes),
            DirectiveKind::Character { name, attributes } => (
                "@character",
                character_path(name, attributes, config),
                attributes,
            ),
            DirectiveKind::Transition { .. } => {
                transitions.push(d);
                continue;
            }
        };
        let Some(anchor) = d.anchor_dialogue_index else {
            out.warnings.push(ValidationIssue::new(
                Some(d.line),
                format!("`{name}` has no dialogue after it and was skipped"),
            ));
            continue;
        };
        let mut attrs = Attributes::new(name, d.line, attributes, &mut out.warnings);
        let duration_ms = attrs.u64("duration_ms");
        let kind = match &d.kind {
            DirectiveKind::Image { .. } | DirectiveKind::Character { .. } => {
                let transform = attrs.transform();
                let presentation = attrs.presentation();
                let speaker = match &d.kind {
                    DirectiveKind::Character { name, .. } => {
                        attrs.string("expression");
                        attrs.string("src");
                        config.resolve_speaker(name).map(|s| s.key.to_string())
                    }
                    _ => None,
                };
                (speaker, transform, presentation)
            }
            _ => (None, Transform::default(), None),
        };
        let volume = match &d.kind {
            DirectiveKind::Bgm { .. } | DirectiveKind::Se { .. } => {
                attrs.f32("volume").unwrap_or(1.0)
            }
            _ => 1.0,
        };
        let looping = match &d.kind {
            DirectiveKind::Bgm { .. } => attrs.bool("loop").unwrap_or(false),
            _ => false,
        };
        attrs.finish();

        let source = match RelativeAssetPath::new(&path) {
            Ok(rel) => rel,
            Err(e) => {
                out.errors.push(ValidationIssue::new(
                    Some(d.line),
                    format!("`{name}` asset path `{path}` is invalid: {e}"),
                ));
                continue;
            }
        };
        let exists = match workspace.resolve(source.as_str()) {
            Ok(abs) => abs.is_file(),
            Err(e) => {
                out.errors.push(ValidationIssue::new(
                    Some(d.line),
                    format!("`{name}` asset path `{path}` is invalid: {e}"),
                ));
                continue;
            }
        };
        if !exists {
            out.warnings.push(ValidationIssue::new(
                Some(d.line),
                format!(
                    "`{name}` asset `{}` not found; the clip is skipped",
                    source.as_str()
                ),
            ));
        }

        let (speaker, transform, presentation) = kind;
        let event_kind = match &d.kind {
            DirectiveKind::Image { .. } => VisualEventKind::Image {
                source: source.clone(),
                transform,
                presentation,
            },
            DirectiveKind::Character { .. } => VisualEventKind::Character {
                speaker,
                source: source.clone(),
                transform,
                presentation,
            },
            DirectiveKind::Bgm { .. } => VisualEventKind::Bgm {
                source: source.clone(),
                volume,
                looping,
            },
            DirectiveKind::Se { .. } => VisualEventKind::SoundEffect {
                source: source.clone(),
                volume,
            },
            DirectiveKind::Transition { .. } => unreachable!("collected above"),
        };
        out.directives.push(ResolvedDirective {
            line: d.line,
            source,
            exists,
            event: VisualEvent {
                anchor_dialogue_index: anchor,
                duration_ms,
                kind: event_kind,
            },
        });
    }

    // `@transition <name>` becomes the intent of every image / character
    // that starts with the same dialogue.
    for t in transitions {
        let DirectiveKind::Transition { name, attributes } = &t.kind else {
            continue;
        };
        let mut attrs = Attributes::new("@transition", t.line, attributes, &mut out.warnings);
        let intent_duration_ms = attrs.u64("duration_ms");
        attrs.finish();
        if !Presentation::is_known_intent(name) {
            out.warnings
                .push(unknown_vocabulary(t.line, "@transition", "intent", name));
        }
        let mut applied = false;
        for r in &mut out.directives {
            if r.event.anchor_dialogue_index != t.anchor_dialogue_index.unwrap_or(0) {
                continue;
            }
            let presentation = match &mut r.event.kind {
                VisualEventKind::Image { presentation, .. }
                | VisualEventKind::Character { presentation, .. } => presentation,
                _ => continue,
            };
            let p = presentation.get_or_insert_with(Presentation::default);
            p.intent = Some(name.clone());
            if intent_duration_ms.is_some() {
                p.intent_duration_ms = intent_duration_ms;
            }
            applied = true;
        }
        if !applied {
            out.warnings.push(ValidationIssue::new(
                Some(t.line),
                format!(
                    "`@transition {name}` has no `@image` / `@character` on the same dialogue and was ignored"
                ),
            ));
        }
    }

    out
}

fn character_path(name: &str, attributes: &BTreeMap<String, String>, config: &Config) -> String {
    if let Some(src) = attributes.get("src") {
        return src.clone();
    }
    let key = config
        .resolve_speaker(name)
        .map(|s| s.key.to_string())
        .unwrap_or_else(|| name.to_string());
    let expression = attributes
        .get("expression")
        .map(String::as_str)
        .unwrap_or("default");
    format!("assets/character/{key}/{expression}.png")
}

fn unknown_vocabulary(line: usize, directive: &str, what: &str, word: &str) -> ValidationIssue {
    let known = match what {
        "role" => videoforge_project::KNOWN_ROLES,
        _ => videoforge_project::KNOWN_INTENTS,
    };
    ValidationIssue::new(
        Some(line),
        format!(
            "`{what}={word}` on `{directive}` is not a recommended {what} (known: {}); it is kept as written",
            known.join(", ")
        ),
    )
}

/// Typed access to a directive's `[k=v]` attributes. Every key read is
/// consumed; whatever is left when `finish` runs is reported as unsupported.
struct Attributes<'a> {
    directive: &'static str,
    line: usize,
    remaining: BTreeMap<&'a str, &'a str>,
    warnings: &'a mut Vec<ValidationIssue>,
}

impl<'a> Attributes<'a> {
    fn new(
        directive: &'static str,
        line: usize,
        attributes: &'a BTreeMap<String, String>,
        warnings: &'a mut Vec<ValidationIssue>,
    ) -> Self {
        Self {
            directive,
            line,
            remaining: attributes
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect(),
            warnings,
        }
    }

    fn string(&mut self, key: &str) -> Option<&'a str> {
        self.remaining.remove(key)
    }

    fn parsed<T: std::str::FromStr>(&mut self, key: &str, expected: &str) -> Option<T> {
        let raw = self.string(key)?;
        match raw.parse::<T>() {
            Ok(v) => Some(v),
            Err(_) => {
                self.warnings.push(ValidationIssue::new(
                    Some(self.line),
                    format!(
                        "`{key}={raw}` on `{}` is not {expected}; the default is used",
                        self.directive
                    ),
                ));
                None
            }
        }
    }

    fn u64(&mut self, key: &str) -> Option<u64> {
        self.parsed(key, "a whole number of milliseconds")
    }

    fn f32(&mut self, key: &str) -> Option<f32> {
        self.parsed::<f32>(key, "a number")
            .filter(|v| v.is_finite())
    }

    fn bool(&mut self, key: &str) -> Option<bool> {
        self.parsed(key, "`true` or `false`")
    }

    fn transform(&mut self) -> Transform {
        let mut t = Transform::default();
        if let Some(v) = self.f32("x") {
            t.x = v;
        }
        if let Some(v) = self.f32("y") {
            t.y = v;
        }
        if let Some(v) = self.f32("scale") {
            t.scale = v;
        }
        if let Some(v) = self.f32("rotation_deg") {
            t.rotation_deg = v;
        }
        if let Some(v) = self.f32("opacity") {
            t.opacity = v;
        }
        if let Some(v) = self.parsed::<i32>("layer", "a whole number") {
            t.layer = v;
        }
        if let Some(raw) = self.string("fit") {
            match raw {
                "contain" => t.fit = FitMode::Contain,
                "cover" => t.fit = FitMode::Cover,
                "stretch" => t.fit = FitMode::Stretch,
                "none" => t.fit = FitMode::None,
                other => self.warnings.push(ValidationIssue::new(
                    Some(self.line),
                    format!(
                        "`fit={other}` on `{}` is not one of contain, cover, stretch, none; the default is used",
                        self.directive
                    ),
                )),
            }
        }
        t
    }

    fn presentation(&mut self) -> Option<Presentation> {
        let role = self.string("role").map(str::to_string);
        let intent = self.string("intent").map(str::to_string);
        let intent_duration_ms = self.u64("intent_duration_ms");
        if let Some(r) = role.as_deref().filter(|r| !Presentation::is_known_role(r)) {
            self.warnings
                .push(unknown_vocabulary(self.line, self.directive, "role", r));
        }
        if let Some(i) = intent
            .as_deref()
            .filter(|i| !Presentation::is_known_intent(i))
        {
            self.warnings
                .push(unknown_vocabulary(self.line, self.directive, "intent", i));
        }
        if role.is_none() && intent.is_none() && intent_duration_ms.is_none() {
            return None;
        }
        Some(Presentation {
            role,
            intent,
            intent_duration_ms,
        })
    }

    fn finish(self) {
        for key in self.remaining.keys() {
            self.warnings.push(ValidationIssue::new(
                Some(self.line),
                format!(
                    "attribute `{key}` on `{}` is not supported and is ignored",
                    self.directive
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::default_config_yaml;
    use crate::init;
    use std::path::Path;

    fn workspace() -> (tempfile::TempDir, Workspace, Config) {
        let dir = tempfile::tempdir().unwrap();
        init::init(dir.path(), Some("t")).unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let cfg = Config::parse(&default_config_yaml("t"), Path::new("x")).unwrap();
        (dir, ws, cfg)
    }

    fn touch(ws: &Workspace, rel: &str) {
        let path = ws.root().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    fn resolve(ws: &Workspace, cfg: &Config, src: &str) -> DirectiveResolution {
        resolve_directives(&videoforge_script::parse_str(src).unwrap(), cfg, ws)
    }

    #[test]
    fn image_with_attributes_becomes_an_anchored_event() {
        let (_d, ws, cfg) = workspace();
        touch(&ws, "assets/image/a.png");
        let r = resolve(
            &ws,
            &cfg,
            "@image assets/image/a.png[role=diagram, duration_ms=1500, x=0.25, layer=2, fit=cover]\n霊夢:\nやあ\n",
        );
        assert_eq!(r.errors, vec![]);
        assert_eq!(r.warnings, vec![]);
        assert_eq!(
            r.directives,
            vec![ResolvedDirective {
                line: 1,
                source: RelativeAssetPath::new("assets/image/a.png").unwrap(),
                exists: true,
                event: VisualEvent {
                    anchor_dialogue_index: 1,
                    duration_ms: Some(1500),
                    kind: VisualEventKind::Image {
                        source: RelativeAssetPath::new("assets/image/a.png").unwrap(),
                        transform: Transform {
                            x: 0.25,
                            layer: 2,
                            fit: FitMode::Cover,
                            ..Transform::default()
                        },
                        presentation: Some(Presentation {
                            role: Some("diagram".into()),
                            intent: None,
                            intent_duration_ms: None,
                        }),
                    },
                },
            }]
        );
    }

    #[test]
    fn character_resolves_speaker_alias_to_the_conventional_path() {
        let (_d, ws, cfg) = workspace();
        touch(&ws, "assets/character/reimu/happy.png");
        touch(&ws, "assets/character/alice/default.png");
        let r = resolve(
            &ws,
            &cfg,
            "@character 霊夢[expression=happy]\n@character alice\n霊夢:\nやあ\n",
        );
        assert_eq!(r.warnings, vec![]);
        let resolved: Vec<(&str, bool, Option<&str>)> = r
            .directives
            .iter()
            .map(|d| {
                let speaker = match &d.event.kind {
                    VisualEventKind::Character { speaker, .. } => speaker.as_deref(),
                    _ => None,
                };
                (d.source.as_str(), d.exists, speaker)
            })
            .collect();
        assert_eq!(
            resolved,
            vec![
                ("assets/character/reimu/happy.png", true, Some("reimu")),
                ("assets/character/alice/default.png", true, None),
            ]
        );
    }

    #[test]
    fn missing_asset_and_unknown_attribute_are_warnings_not_errors() {
        let (_d, ws, cfg) = workspace();
        let r = resolve(
            &ws,
            &cfg,
            "@bgm assets/bgm/none.mp3[volume=0.5, fade=slow]\n霊夢:\nやあ\n",
        );
        assert_eq!(r.errors, vec![]);
        assert_eq!(
            r.warnings,
            vec![
                ValidationIssue::new(
                    Some(1),
                    "attribute `fade` on `@bgm` is not supported and is ignored"
                ),
                ValidationIssue::new(
                    Some(1),
                    "`@bgm` asset `assets/bgm/none.mp3` not found; the clip is skipped"
                ),
            ]
        );
        assert_eq!(r.directives.len(), 1);
        assert!(!r.directives[0].exists);
    }

    #[test]
    fn path_escaping_the_workspace_is_an_error() {
        let (_d, ws, cfg) = workspace();
        let r = resolve(&ws, &cfg, "@se ../outside.wav\n霊夢:\nやあ\n");
        assert_eq!(r.directives, vec![]);
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].line, Some(1));
        assert!(r.errors[0]
            .message
            .starts_with("`@se` asset path `../outside.wav` is invalid"));
    }

    #[test]
    fn trailing_directive_is_skipped_with_a_warning() {
        let (_d, ws, cfg) = workspace();
        touch(&ws, "assets/se/pop.wav");
        let r = resolve(&ws, &cfg, "霊夢:\nやあ\n\n@se assets/se/pop.wav\n");
        assert_eq!(r.directives, vec![]);
        assert_eq!(
            r.warnings,
            vec![ValidationIssue::new(
                Some(4),
                "`@se` has no dialogue after it and was skipped"
            )]
        );
    }

    #[test]
    fn transition_sets_the_intent_of_the_visual_on_the_same_dialogue() {
        let (_d, ws, cfg) = workspace();
        touch(&ws, "assets/image/a.png");
        let r = resolve(
            &ws,
            &cfg,
            "@image assets/image/a.png\n@transition fade[duration_ms=300]\n霊夢:\nやあ\n\n@transition zoom\n魔理沙:\nおう\n",
        );
        let VisualEventKind::Image { presentation, .. } = &r.directives[0].event.kind else {
            panic!("image expected");
        };
        assert_eq!(
            presentation,
            &Some(Presentation {
                role: None,
                intent: Some("fade".into()),
                intent_duration_ms: Some(300),
            })
        );
        assert_eq!(
            r.warnings,
            vec![ValidationIssue::new(
                Some(6),
                "`@transition zoom` has no `@image` / `@character` on the same dialogue and was ignored"
            )]
        );
    }

    #[test]
    fn invalid_numbers_fall_back_to_defaults_with_a_warning() {
        let (_d, ws, cfg) = workspace();
        touch(&ws, "assets/se/pop.wav");
        let r = resolve(
            &ws,
            &cfg,
            "@se assets/se/pop.wav[volume=loud, duration_ms=1.5]\n霊夢:\nやあ\n",
        );
        let VisualEventKind::SoundEffect { volume, .. } = &r.directives[0].event.kind else {
            panic!("se expected");
        };
        assert_eq!(*volume, 1.0);
        assert_eq!(r.directives[0].event.duration_ms, None);
        let messages: Vec<&str> = r.warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            messages,
            vec![
                "`duration_ms=1.5` on `@se` is not a whole number of milliseconds; the default is used",
                "`volume=loud` on `@se` is not a number; the default is used",
            ]
        );
    }
}
