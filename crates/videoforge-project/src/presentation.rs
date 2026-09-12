//! Semantic presentation metadata for visual clips (design §11.6, issue #16).
//!
//! Two questions an exporter, a renderer or an agent may ask about an image
//! or character clip, kept separate from the concrete [`Transform`]:
//!
//! * **role** — *what is this for?* (`primary_visual`, `diagram`, `callout`, …)
//! * **intent** — *how should it appear?* (`fade`, `slide`, `zoom`, …)
//!
//! Both are free-form strings, not closed enums: scripts and agents may use
//! words outside the recommended vocabulary and validation only warns, the
//! same policy as unknown script directives. The recommended words are
//! [`KNOWN_ROLES`] and [`KNOWN_INTENTS`]; renderers decide what each means
//! and how `intent_duration_ms` is interpolated.
//!
//! [`Transform`]: crate::project::Transform

use serde::{Deserialize, Serialize};

/// Recommended `role` vocabulary.
pub const KNOWN_ROLES: &[&str] = &[
    "primary_visual",
    "supporting_visual",
    "diagram",
    "character",
    "background",
    "callout",
    "comparison",
    "emphasis",
    // An overlay (watermark, lower-third, logo) — see `Clip`'s doc comment
    // for why this is an `ImageClip` with this role, not a separate
    // "OverlayClip" type (P1-1).
    "overlay",
];

/// Recommended `intent` vocabulary. `videoforge-preview` (P1-5) renders
/// `"fade"` as an alpha ramp over `intent_duration_ms` at the clip's
/// start/end, `"slide"`/`"zoom"` as a Ken Burns-style pan/zoom across the
/// clip's own `duration_ms` (no separate "pan" word — a slide *is* a pan),
/// and `"cut"`/`"emphasis"`/anything else as a plain cut (P1-5's own
/// "don't build a complex editor" scope: only what plays back the same way
/// every time from `project.vfp.json` alone, no manual keyframing).
pub const KNOWN_INTENTS: &[&str] = &["fade", "slide", "zoom", "emphasis", "cut"];

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Presentation {
    /// Why the clip is on screen. See [`KNOWN_ROLES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// How the clip should come in / behave. See [`KNOWN_INTENTS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Length of the `intent` transition, interpreted by the renderer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_duration_ms: Option<u64>,
}

impl Presentation {
    pub fn is_known_role(role: &str) -> bool {
        KNOWN_ROLES.contains(&role)
    }

    pub fn is_known_intent(intent: &str) -> bool {
        KNOWN_INTENTS.contains(&intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_lookup_distinguishes_known_from_unknown_words() {
        assert!(Presentation::is_known_role("primary_visual"));
        assert!(!Presentation::is_known_role("hero"));
        assert!(Presentation::is_known_intent("fade"));
        assert!(!Presentation::is_known_intent("wobble"));
    }

    #[test]
    fn empty_presentation_serializes_to_an_empty_object() {
        assert_eq!(
            serde_json::to_string(&Presentation::default()).unwrap(),
            "{}"
        );
        assert_eq!(
            serde_json::from_str::<Presentation>("{}").unwrap(),
            Presentation::default()
        );
    }
}
