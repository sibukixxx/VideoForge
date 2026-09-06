//! Template patching (VF-060..VF-065).
//!
//! The template is handled as an opaque `serde_json::Value` (key order
//! preserved). Only these fields are touched on cloned items:
//! `Text`, `Frame`, `Length`, `FilePath`, `Remark`, `IsHidden`.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use videoforge_core::project::{millis_to_frame, VideoProject};
use videoforge_core::AppError;

pub const PROTO_PREFIX: &str = "VF_PROTO_";
pub const PROTO_AUDIO: &str = "VF_PROTO_AUDIO";
pub const PROTO_CAPTION: &str = "VF_PROTO_CAPTION";
pub const PROTO_CHARACTER: &str = "VF_PROTO_CHARACTER";
pub const PROTO_BACKGROUND: &str = "VF_PROTO_BACKGROUND";

const REMARK: &str = "Remark";

#[derive(Debug)]
pub struct PatchOutput {
    pub value: Value,
    pub fps: u32,
    pub items_added: usize,
    pub warnings: Vec<String>,
}

/// Patch `template` with the clips of `project`.
///
/// * `materialize` converts a project-relative asset path to the absolute
///   path YMM4 should reference.
/// * `output_path` (Windows absolute) is written to the root `FilePath` when
///   the template has one.
pub fn patch_template(
    mut template: Value,
    project: &VideoProject,
    template_path: &Path,
    materialize: &dyn Fn(&str) -> String,
    output_path: Option<&str>,
) -> Result<PatchOutput, AppError> {
    let invalid = |reason: String| AppError::InvalidTemplate {
        path: template_path.to_path_buf(),
        reason,
    };
    let mut warnings = Vec::new();

    let items_ptr = find_prototype_array(&template, "").ok_or_else(|| {
        AppError::TemplatePrototypeMissing(format!(
            "no timeline item with a `{REMARK}` starting with `{PROTO_PREFIX}` found in {}",
            template_path.display()
        ))
    })?;

    // fps: the template's timeline VideoInfo wins because YMM4 plays at it.
    let timeline_ptr = parent_pointer(&items_ptr);
    let template_fps = template
        .pointer(&timeline_ptr)
        .and_then(find_fps)
        .or_else(|| find_fps(&template));
    let fps = match template_fps {
        Some(f) if f != project.video.fps => {
            warnings.push(format!(
                "template fps {f} differs from project fps {}; frames were computed at {f} fps",
                project.video.fps
            ));
            f
        }
        Some(f) => f,
        None => project.video.fps,
    };
    if fps == 0 {
        return Err(invalid("fps must be > 0".into()));
    }

    // Take prototypes out of the array.
    let items = template
        .pointer_mut(&items_ptr)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("prototype array vanished".into()))?;
    let mut prototypes: BTreeMap<String, Value> = BTreeMap::new();
    items.retain(|item| match remark_of(item) {
        Some(r) if r.starts_with(PROTO_PREFIX) => {
            prototypes.insert(r.to_string(), item.clone());
            false
        }
        _ => true,
    });

    let audio_proto = prototypes.get(PROTO_AUDIO).ok_or_else(|| {
        AppError::TemplatePrototypeMissing(format!("`{PROTO_AUDIO}` (audio item)"))
    })?;

    let mut new_items: Vec<Value> = Vec::new();
    let frames = |ms: u64| millis_to_frame(ms, fps);
    let length = |ms: u64| frames(ms).max(1);

    // Audio
    for clip in project.audio_clips() {
        let mut item = audio_proto.clone();
        set(&mut item, "Frame", frames(clip.start_ms));
        set(&mut item, "Length", length(clip.duration_ms));
        set(&mut item, "FilePath", materialize(clip.source.as_str()));
        set(&mut item, REMARK, format!("VF:{}", clip.id));
        set_if_present(&mut item, "IsHidden", false);
        new_items.push(item);
    }

    // Captions (+ optional character items)
    for clip in project.caption_clips() {
        let key = clip.speaker.to_uppercase();
        let proto_name = format!("{PROTO_CAPTION}_{key}");
        let proto = prototypes
            .get(&proto_name)
            .or_else(|| prototypes.get(PROTO_CAPTION))
            .ok_or_else(|| {
                AppError::TemplatePrototypeMissing(format!(
                    "`{proto_name}` or `{PROTO_CAPTION}` (text item for speaker `{}`)",
                    clip.speaker
                ))
            })?;
        let mut item = proto.clone();
        set(&mut item, "Text", clip.text.clone());
        set(&mut item, "Frame", frames(clip.start_ms));
        set(&mut item, "Length", length(clip.duration_ms));
        set(&mut item, REMARK, format!("VF:{}", clip.id));
        set_if_present(&mut item, "IsHidden", false);
        new_items.push(item);

        if let Some(character) = prototypes.get(&format!("{PROTO_CHARACTER}_{key}")) {
            let mut item = character.clone();
            set(&mut item, "Frame", frames(clip.start_ms));
            set(&mut item, "Length", length(clip.duration_ms));
            set(&mut item, REMARK, format!("VF:character-{}", clip.id));
            set_if_present(&mut item, "IsHidden", false);
            new_items.push(item);
        }
    }

    // Background (optional)
    if let Some(bg_proto) = prototypes.get(PROTO_BACKGROUND) {
        for clip in project.background_clips() {
            let mut item = bg_proto.clone();
            set(&mut item, "Frame", frames(clip.start_ms));
            set(&mut item, "Length", length(clip.duration_ms));
            set(&mut item, "FilePath", materialize(clip.source.as_str()));
            set(&mut item, REMARK, format!("VF:{}", clip.id));
            set_if_present(&mut item, "IsHidden", false);
            new_items.push(item);
        }
    }

    let unused: Vec<&String> = prototypes
        .keys()
        .filter(|k| {
            k.as_str() != PROTO_AUDIO
                && !k.starts_with(PROTO_CAPTION)
                && !k.starts_with(PROTO_CHARACTER)
                && k.as_str() != PROTO_BACKGROUND
        })
        .collect();
    if !unused.is_empty() {
        warnings.push(format!(
            "unknown prototypes removed from template: {}",
            unused
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let items_added = new_items.len();
    items.extend(new_items);

    // Timeline length
    let total_frames = frames(project.total_duration_ms());
    if let Some(timeline) = template
        .pointer_mut(&timeline_ptr)
        .and_then(Value::as_object_mut)
    {
        if let Some(len) = timeline.get("Length") {
            let current = len.as_u64().unwrap_or(0);
            timeline.insert(
                "Length".into(),
                Value::from(current.max(total_frames as u64)),
            );
        }
    }

    // Root FilePath
    if let (Some(out), Some(root)) = (output_path, template.as_object_mut()) {
        if root.contains_key("FilePath") {
            root.insert("FilePath".into(), Value::from(out));
        }
    }

    Ok(PatchOutput {
        value: template,
        fps,
        items_added,
        warnings,
    })
}

fn remark_of(item: &Value) -> Option<&str> {
    item.get(REMARK).and_then(Value::as_str)
}

fn set<V: Into<Value>>(item: &mut Value, key: &str, value: V) {
    if let Some(obj) = item.as_object_mut() {
        obj.insert(key.to_string(), value.into());
    }
}

fn set_if_present<V: Into<Value>>(item: &mut Value, key: &str, value: V) {
    if let Some(obj) = item.as_object_mut() {
        if obj.contains_key(key) {
            obj.insert(key.to_string(), value.into());
        }
    }
}

/// Depth-first search for an array containing at least one object whose
/// `Remark` starts with the prototype prefix. Returns its JSON pointer.
fn find_prototype_array(value: &Value, pointer: &str) -> Option<String> {
    match value {
        Value::Array(items) => {
            if items
                .iter()
                .any(|i| remark_of(i).is_some_and(|r| r.starts_with(PROTO_PREFIX)))
            {
                return Some(pointer.to_string());
            }
            items
                .iter()
                .enumerate()
                .find_map(|(i, v)| find_prototype_array(v, &format!("{pointer}/{i}")))
        }
        Value::Object(map) => map.iter().find_map(|(k, v)| {
            find_prototype_array(v, &format!("{pointer}/{}", escape_pointer(k)))
        }),
        _ => None,
    }
}

fn parent_pointer(pointer: &str) -> String {
    match pointer.rfind('/') {
        Some(i) => pointer[..i].to_string(),
        None => String::new(),
    }
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Look for `VideoInfo.FPS` (or a bare `FPS`) within `value`, shallowly.
fn find_fps(value: &Value) -> Option<u32> {
    let obj: &Map<String, Value> = value.as_object()?;
    if let Some(fps) = obj
        .get("VideoInfo")
        .and_then(|v| v.get("FPS"))
        .and_then(Value::as_f64)
    {
        return Some(fps.round() as u32);
    }
    obj.get("FPS")
        .and_then(Value::as_f64)
        .map(|f| f.round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap as Map2;
    use videoforge_core::project::{
        AudioClip, CaptionClip, Clip, RelativeAssetPath, Track, TrackKind, VideoSettings,
    };

    fn template() -> Value {
        json!({
            "FilePath": "C:\\template.ymmp",
            "Timelines": [{
                "Items": [
                    {"$type": "TextItem", "Text": "keep me", "Frame": 0, "Length": 10, "Layer": 9, "Remark": "", "Custom": {"a": 1}},
                    {"$type": "TextItem", "Text": "REIMU PROTO", "Frame": 0, "Length": 60, "Layer": 3, "Remark": "VF_PROTO_CAPTION_REIMU", "IsHidden": true, "FontColor": "#FF0000"},
                    {"$type": "TextItem", "Text": "GENERIC", "Frame": 0, "Length": 60, "Layer": 3, "Remark": "VF_PROTO_CAPTION", "FontColor": "#FFFFFF"},
                    {"$type": "AudioItem", "FilePath": "C:\\proto.wav", "Frame": 0, "Length": 60, "Layer": 1, "Remark": "VF_PROTO_AUDIO", "Volume": {"Values": [{"Value": 50.0}]}},
                    {"$type": "TachieItem", "Frame": 0, "Length": 60, "Layer": 2, "Remark": "VF_PROTO_CHARACTER_REIMU"},
                    {"$type": "Mystery", "Frame": 0, "Length": 1, "Remark": "VF_PROTO_SOMETHING"}
                ],
                "VideoInfo": {"FPS": 60, "Width": 1920, "Height": 1080},
                "Length": 60
            }],
            "Characters": [{"Name": "霊夢"}]
        })
    }

    fn project() -> VideoProject {
        let mut p = VideoProject::new("s", "S", VideoSettings::default());
        p.tracks.push(Track {
            id: "audio".into(),
            kind: TrackKind::Audio,
            clips: vec![
                Clip::Audio(AudioClip {
                    id: "audio-001".into(),
                    source: RelativeAssetPath::new("assets/audio/001.wav").unwrap(),
                    start_ms: 0,
                    duration_ms: 3410,
                    speaker: "reimu".into(),
                    extra: Map2::new(),
                }),
                Clip::Audio(AudioClip {
                    id: "audio-002".into(),
                    source: RelativeAssetPath::new("assets/audio/002.wav").unwrap(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    extra: Map2::new(),
                }),
            ],
        });
        p.tracks.push(Track {
            id: "caption".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "caption-001".into(),
                    text: "こんにちは".into(),
                    start_ms: 0,
                    duration_ms: 3410,
                    speaker: "reimu".into(),
                    speaker_display: Some("霊夢".into()),
                    extra: Map2::new(),
                }),
                Clip::Caption(CaptionClip {
                    id: "caption-002".into(),
                    text: "やあ".into(),
                    start_ms: 3610,
                    duration_ms: 3680,
                    speaker: "marisa".into(),
                    speaker_display: None,
                    extra: Map2::new(),
                }),
            ],
        });
        p
    }

    fn materialize(rel: &str) -> String {
        format!("C:\\proj\\{}", rel.replace('/', "\\"))
    }

    #[test]
    fn patches_template() {
        let out = patch_template(
            template(),
            &project(),
            Path::new("t.ymmp"),
            &materialize,
            Some("C:\\proj\\ymm4\\project.ymmp"),
        )
        .unwrap();
        assert_eq!(out.fps, 60);
        assert!(out.warnings.iter().any(|w| w.contains("template fps 60")));
        assert!(out
            .warnings
            .iter()
            .any(|w| w.contains("VF_PROTO_SOMETHING")));
        // 2 audio + 2 captions + 1 character (reimu only)
        assert_eq!(out.items_added, 5);

        let v = out.value;
        assert_eq!(v["FilePath"], "C:\\proj\\ymm4\\project.ymmp");
        assert_eq!(v["Characters"][0]["Name"], "霊夢");
        let items = v["Timelines"][0]["Items"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        assert_eq!(items[0]["Text"], "keep me");
        assert_eq!(items[0]["Custom"]["a"], 1);
        assert!(items
            .iter()
            .all(|i| !i["Remark"].as_str().unwrap_or("").starts_with("VF_PROTO_")));

        let audio1 = &items[1];
        assert_eq!(audio1["$type"], "AudioItem");
        assert_eq!(audio1["Frame"], 0);
        assert_eq!(audio1["Length"], 205); // 3.41s * 60
        assert_eq!(audio1["FilePath"], "C:\\proj\\assets\\audio\\001.wav");
        assert_eq!(audio1["Volume"]["Values"][0]["Value"], 50.0);
        assert_eq!(audio1["Remark"], "VF:audio-001");
        let audio2 = &items[2];
        assert_eq!(audio2["Frame"], 217); // 3.61 * 60 = 216.6

        let cap1 = &items[3];
        assert_eq!(cap1["Text"], "こんにちは");
        assert_eq!(cap1["FontColor"], "#FF0000");
        assert_eq!(cap1["IsHidden"], false);
        assert_eq!(cap1["Layer"], 3);
        let chara = &items[4];
        assert_eq!(chara["$type"], "TachieItem");
        assert_eq!(chara["Length"], 205);
        let cap2 = &items[5];
        assert_eq!(cap2["Text"], "やあ");
        assert_eq!(cap2["FontColor"], "#FFFFFF"); // generic fallback

        assert_eq!(v["Timelines"][0]["Length"], 437); // 7.29 * 60 = 437.4
    }

    #[test]
    fn missing_prototypes_are_errors() {
        let mut t = template();
        t["Timelines"][0]["Items"]
            .as_array_mut()
            .unwrap()
            .retain(|i| i["Remark"] != "VF_PROTO_AUDIO");
        let err = patch_template(t, &project(), Path::new("t"), &materialize, None).unwrap_err();
        assert!(
            matches!(err, AppError::TemplatePrototypeMissing(m) if m.contains("VF_PROTO_AUDIO"))
        );

        let mut t = template();
        t["Timelines"][0]["Items"]
            .as_array_mut()
            .unwrap()
            .retain(|i| i["Remark"] != "VF_PROTO_CAPTION");
        let err = patch_template(t, &project(), Path::new("t"), &materialize, None).unwrap_err();
        assert!(matches!(err, AppError::TemplatePrototypeMissing(m) if m.contains("MARISA")));

        let err = patch_template(
            json!({"Items": []}),
            &project(),
            Path::new("t"),
            &materialize,
            None,
        )
        .unwrap_err();
        assert!(matches!(err, AppError::TemplatePrototypeMissing(_)));
    }

    #[test]
    fn falls_back_to_project_fps_without_video_info() {
        let mut t = template();
        t["Timelines"][0]
            .as_object_mut()
            .unwrap()
            .remove("VideoInfo");
        let out = patch_template(t, &project(), Path::new("t"), &materialize, None).unwrap();
        assert_eq!(out.fps, 30);
        assert!(out.warnings.iter().all(|w| !w.contains("fps")));
    }
}
