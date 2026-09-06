//! Progress events shared by CLI and GUI (design §32).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum GenerationStage {
    Parsing,
    Validating,
    Synthesizing {
        current: usize,
        total: usize,
        index: usize,
        speaker: String,
        cached: bool,
    },
    BuildingTimeline,
    WritingProject,
    WritingCaptions,
    RenderingPreview,
    PreviewSkipped {
        reason: String,
    },
    Completed,
}

pub trait ProgressSink: Send + Sync {
    fn on_stage(&self, stage: &GenerationStage);
}

pub struct NoopProgress;

impl ProgressSink for NoopProgress {
    fn on_stage(&self, _stage: &GenerationStage) {}
}

/// Adapter for closures.
pub struct FnProgress<F: Fn(&GenerationStage) + Send + Sync>(pub F);

impl<F: Fn(&GenerationStage) + Send + Sync> ProgressSink for FnProgress<F> {
    fn on_stage(&self, stage: &GenerationStage) {
        (self.0)(stage)
    }
}
