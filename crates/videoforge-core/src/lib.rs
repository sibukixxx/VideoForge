//! VideoForge core.
//!
//! Platform-independent orchestration: workspace + config, script validation,
//! the generation pipeline (script → TTS → timeline → IR → SRT → preview), and
//! the traits that engines/exporters implement.
//!
//! Dependency rules (see design §28): this crate never depends on Tauri,
//! Windows APIs, or the YMM4 schema.

pub mod capabilities;
pub mod config;
pub mod doctor;
pub mod error;
pub mod export;
pub mod generate;
pub mod init;
pub mod manifest;
pub mod preview;
pub mod progress;
pub mod tts;
pub mod validate;
pub mod wav;
pub mod workspace;

pub use capabilities::Capabilities;
pub use config::Config;
pub use error::AppError;
pub use generate::{generate, GenerateDeps, GenerateOptions, GeneratedProject};
pub use progress::{GenerationStage, ProgressSink};
pub use workspace::Workspace;

pub const GENERATOR_VERSION: &str = env!("CARGO_PKG_VERSION");

// Re-exports so downstream crates need fewer direct dependencies.
pub use tokio_util::sync::CancellationToken;
pub use videoforge_platform as platform;
pub use videoforge_project as project;
pub use videoforge_script as script;
pub use videoforge_timeline as timeline;
