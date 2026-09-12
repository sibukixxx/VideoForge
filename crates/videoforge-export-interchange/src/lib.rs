//! Cross-platform editor interchange exporters.
//!
//! These exporters intentionally translate from the canonical VideoProject IR.
//! NLE-specific concepts stay in this leaf crate and never leak into core.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use videoforge_core::export::{ExportCapabilities, ExportRequest, ExportResult, ProjectExporter};
use videoforge_core::AppError;
use videoforge_project::{Clip, VideoProject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterchangeFormat {
    Fcpxml,
    Otio,
}

pub struct InterchangeExporter {
    format: InterchangeFormat,
}

impl InterchangeExporter {
    pub fn fcpxml() -> Self {
        Self {
            format: InterchangeFormat::Fcpxml,
        }
    }
    pub fn otio() -> Self {
        Self {
            format: InterchangeFormat::Otio,
        }
    }

    fn default_destination(&self, project_dir: &Path) -> PathBuf {
        match self.format {
            InterchangeFormat::Fcpxml => project_dir.join("interchange/project.fcpxml"),
            InterchangeFormat::Otio => project_dir.join("interchange/project.otio"),
        }
    }
}

#[async_trait]
impl ProjectExporter for InterchangeExporter {
    fn id(&self) -> &'static str {
        match self.format {
            InterchangeFormat::Fcpxml => "fcpxml",
            InterchangeFormat::Otio => "otio",
        }
    }

    fn capabilities(&self) -> ExportCapabilities {
        ExportCapabilities {
            available: true,
            reason: None,
            can_open: false,
        }
    }

    async fn export(&self, request: ExportRequest<'_>) -> Result<ExportResult, AppError> {
        let output = request
            .destination
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.default_destination(request.project_dir));
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::write(parent, e))?;
        }
        let (body, warnings) = match self.format {
            InterchangeFormat::Fcpxml => render_fcpxml(request.project, request.project_dir),
            InterchangeFormat::Otio => (
                render_otio(request.project, request.project_dir)?,
                Vec::new(),
            ),
        };
        std::fs::write(&output, body).map_err(|e| AppError::write(&output, e))?;
        Ok(ExportResult { output, warnings })
    }
}

fn project_duration_ms(project: &VideoProject) -> u64 {
    project
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .map(Clip::end_ms)
        .max()
        .unwrap_or(0)
}

fn asset_uri(project_dir: &Path, clip: &Clip) -> Option<String> {
    clip.asset().map(|p| {
        let path = project_dir.join(p.as_str());
        format!("file://{}", path.to_string_lossy().replace(' ', "%20"))
    })
}

fn esc_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn fcpx_time(ms: u64) -> String {
    format!("{ms}/1000s")
}

fn render_fcpxml(project: &VideoProject, project_dir: &Path) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let mut assets = String::new();
    let mut spine = String::new();
    let mut asset_no = 0usize;

    for track in &project.tracks {
        for clip in &track.clips {
            match clip {
                Clip::Caption(c) => {
                    spine.push_str(&format!("        <title name=\"{}\" offset=\"{}\" duration=\"{}\"><text><text-style ref=\"ts1\">{}</text-style></text></title>\n",
                        esc_xml(&c.id), fcpx_time(c.start_ms), fcpx_time(c.duration_ms), esc_xml(&c.text)));
                }
                _ if clip.asset().is_some() => {
                    asset_no += 1;
                    let id = format!("r{}", asset_no + 2);
                    let uri = asset_uri(project_dir, clip).unwrap_or_default();
                    assets.push_str(&format!("    <asset id=\"{id}\" name=\"{}\" src=\"{}\" start=\"0s\" duration=\"{}\" hasVideo=\"1\" hasAudio=\"1\"/>\n",
                        esc_xml(clip.id()), esc_xml(&uri), fcpx_time(clip.duration_ms())));
                    spine.push_str(&format!("        <asset-clip name=\"{}\" ref=\"{id}\" offset=\"{}\" duration=\"{}\"/>\n",
                        esc_xml(clip.id()), fcpx_time(clip.start_ms()), fcpx_time(clip.duration_ms())));
                }
                _ => warnings.push(format!(
                    "clip `{}` has no FCPXML representation and was skipped",
                    clip.id()
                )),
            }
        }
    }

    let fps = project.video.fps.max(1);
    let frame_duration = format!("1/{fps}s");
    let duration = fcpx_time(project_duration_ms(project));
    let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n<fcpxml version=\"1.10\">\n  <resources>\n    <format id=\"r1\" name=\"VideoForge\" frameDuration=\"{frame_duration}\" width=\"{}\" height=\"{}\"/>\n    <effect id=\"r2\" name=\"Basic Title\" uid=\".../Titles.localized/Bumper:Opener.localized/Basic Title.localized/Basic Title.moti\"/>\n    <text-style-def id=\"ts1\"><text-style font=\"Helvetica\" fontSize=\"48\"/></text-style-def>\n{assets}  </resources>\n  <library><event name=\"VideoForge\"><project name=\"{}\"><sequence format=\"r1\" duration=\"{duration}\"><spine>\n{spine}      </spine></sequence></project></event></library>\n</fcpxml>\n",
        project.video.width, project.video.height, esc_xml(&project.title));
    (xml, warnings)
}

fn rational_time(ms: u64, fps: u32) -> Value {
    json!({"OTIO_SCHEMA":"RationalTime.1","value": (ms as f64 / 1000.0) * fps as f64,"rate":fps as f64})
}

fn time_range(start_ms: u64, duration_ms: u64, fps: u32) -> Value {
    json!({"OTIO_SCHEMA":"TimeRange.1","start_time":rational_time(start_ms, fps),"duration":rational_time(duration_ms, fps)})
}

fn render_otio(project: &VideoProject, project_dir: &Path) -> Result<String, AppError> {
    let fps = project.video.fps.max(1);
    let tracks: Vec<Value> = project.tracks.iter().map(|track| {
        let children: Vec<Value> = track.clips.iter().map(|clip| {
            let reference = asset_uri(project_dir, clip).map(|url| json!({
                "OTIO_SCHEMA":"ExternalReference.1","target_url":url,"available_range":time_range(0, clip.duration_ms(), fps),"metadata":{}
            })).unwrap_or_else(|| json!({"OTIO_SCHEMA":"MissingReference.1","metadata":{}}));
            let mut metadata = json!({"videoforge":{"clip_id":clip.id(),"start_ms":clip.start_ms()}});
            if let Clip::Caption(c) = clip { metadata["videoforge"]["text"] = json!(c.text); metadata["videoforge"]["speaker"] = json!(c.speaker); }
            json!({"OTIO_SCHEMA":"Clip.2","name":clip.id(),"source_range":time_range(0, clip.duration_ms(), fps),"media_reference":reference,"metadata":metadata,"effects":[],"markers":[],"enabled":true})
        }).collect();
        json!({"OTIO_SCHEMA":"Track.1","name":track.id,"kind": if matches!(track.kind, videoforge_project::TrackKind::Audio | videoforge_project::TrackKind::Bgm | videoforge_project::TrackKind::SoundEffect) {"Audio"} else {"Video"},"children":children,"metadata":{"videoforge":{"track_kind":format!("{:?}", track.kind)}},"effects":[],"markers":[]})
    }).collect();
    let root = json!({"OTIO_SCHEMA":"Timeline.1","name":project.title,"global_start_time":null,"tracks":{"OTIO_SCHEMA":"Stack.1","name":"tracks","children":tracks,"metadata":{},"effects":[],"markers":[]},"metadata":{"videoforge":{"schema_version":project.schema_version,"width":project.video.width,"height":project.video.height,"fps":fps}}});
    serde_json::to_string_pretty(&root).map_err(|e| AppError::serialization("OTIO project", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use videoforge_project::{
        CaptionClip, SourceInfo, Track, TrackKind, VideoSettings, SCHEMA_VERSION,
    };

    fn sample() -> VideoProject {
        VideoProject {
            schema_version: SCHEMA_VERSION,
            id: "p1".into(),
            title: "P3 test".into(),
            video: VideoSettings::default(),
            source: SourceInfo::default(),
            tracks: vec![Track {
                id: "captions".into(),
                kind: TrackKind::Caption,
                clips: vec![Clip::Caption(CaptionClip {
                    id: "c1".into(),
                    text: "Hello & world".into(),
                    start_ms: 0,
                    duration_ms: 1000,
                    speaker: "a".into(),
                    speaker_display: None,
                    extra: BTreeMap::new(),
                })],
            }],
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn fcpxml_escapes_caption() {
        let (s, _) = render_fcpxml(&sample(), Path::new("."));
        assert!(s.contains("Hello &amp; world"));
    }
    #[test]
    fn otio_has_timeline_schema() {
        let s = render_otio(&sample(), Path::new(".")).unwrap();
        assert!(s.contains("Timeline.1"));
        assert!(s.contains("Hello & world"));
    }
}
