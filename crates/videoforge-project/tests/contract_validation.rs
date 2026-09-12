//! Contract Test C (P0 §15): a fixed set of invalid `project.vfp.json`
//! fixtures under `fixtures/video_project/invalid/`, each paired with the
//! specific failure it must produce. This is a change detector for the two
//! validation layers `videoforge_project` exposes:
//!
//! * parse-time (`VideoProject::from_json` → `ProjectError`): malformed or
//!   unsupported input that never becomes a `VideoProject` at all.
//! * structural (`validate_project` → `ProjectValidationReport`): a
//!   well-formed `VideoProject` whose content violates an IR invariant.
//!
//! Asset-existence checking (`core::assets::AssetRegistry`, "存在しない asset
//! 参照" from the P0 brief) is deliberately a third, separate layer that
//! needs filesystem access and therefore lives in `videoforge-core`, not
//! here — see `docs/video-project.md`'s Validation section.

use videoforge_project::{validate_project, ProjectError, VideoProject};

/// The companion valid fixture (Contract Test A) must still validate clean —
/// otherwise these "invalid" fixtures would just be exercising a coincidence.
#[test]
fn the_valid_contract_fixture_has_no_validation_errors() {
    let json = include_str!("../../../fixtures/video_project/basic.vfp.json");
    let project = VideoProject::from_json(json).unwrap();
    let report = validate_project(&project);
    assert!(report.is_ok(), "{:?}", report.errors);
}

#[test]
fn missing_schema_version_is_rejected_at_parse_time() {
    let json = include_str!("../../../fixtures/video_project/invalid/missing_schema_version.json");
    let err = VideoProject::from_json(json).unwrap_err();
    assert!(matches!(err, ProjectError::MissingSchemaVersion));
}

#[test]
fn unsupported_schema_version_is_rejected_at_parse_time() {
    let json =
        include_str!("../../../fixtures/video_project/invalid/unsupported_schema_version.json");
    let err = VideoProject::from_json(json).unwrap_err();
    assert!(matches!(
        err,
        ProjectError::UnsupportedSchema { found: 99, .. }
    ));
}

#[test]
fn absolute_asset_path_is_rejected_at_parse_time() {
    let json = include_str!("../../../fixtures/video_project/invalid/absolute_asset_path.json");
    let err = VideoProject::from_json(json).unwrap_err();
    assert!(matches!(err, ProjectError::Json(_)));
}

#[test]
fn duplicate_clip_id_fails_structural_validation() {
    let json = include_str!("../../../fixtures/video_project/invalid/duplicate_clip_id.json");
    let project = VideoProject::from_json(json).unwrap();
    let report = validate_project(&project);
    assert!(!report.is_ok());
    assert!(report.errors.iter().any(|e| e.code == "duplicate_clip_id"));
}

#[test]
fn duplicate_track_id_fails_structural_validation() {
    let json = include_str!("../../../fixtures/video_project/invalid/duplicate_track_id.json");
    let project = VideoProject::from_json(json).unwrap();
    let report = validate_project(&project);
    assert!(!report.is_ok());
    assert!(report.errors.iter().any(|e| e.code == "duplicate_track_id"));
}

#[test]
fn zero_duration_clip_fails_structural_validation() {
    let json = include_str!("../../../fixtures/video_project/invalid/zero_duration_clip.json");
    let project = VideoProject::from_json(json).unwrap();
    let report = validate_project(&project);
    assert!(!report.is_ok());
    assert!(report.errors.iter().any(|e| e.code == "zero_clip_duration"));
}

#[test]
fn track_clip_kind_mismatch_fails_structural_validation() {
    let json =
        include_str!("../../../fixtures/video_project/invalid/track_clip_kind_mismatch.json");
    let project = VideoProject::from_json(json).unwrap();
    let report = validate_project(&project);
    assert!(!report.is_ok());
    assert!(report
        .errors
        .iter()
        .any(|e| e.code == "track_clip_kind_mismatch"));
}
