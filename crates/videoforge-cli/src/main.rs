//! `videoforge` CLI (design §7). The CLI is the primary agent interface:
//! files in, files out, stable exit codes.
//!
//! Exit codes: 0 ok · 1 error · 2 validation failed / doctor failures.

mod commands;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "videoforge",
    version,
    about = "Compile Markdown scripts into voice, captions, timeline, preview and editable video projects"
)]
struct Cli {
    /// Workspace root (defaults to discovering videoforge.yaml from the given path / cwd)
    #[arg(long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,

    /// Print machine-readable JSON instead of human output
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Prepare or review a source-grounded draft (external AI; no API calls)
    Draft {
        #[command(subcommand)]
        action: DraftAction,
    },
    /// Create a new workspace (videoforge.yaml, AGENTS.md, scripts/, assets/, templates/, generated/)
    Init {
        /// Directory to create (default: current directory)
        dir: Option<PathBuf>,
        /// Project name written to videoforge.yaml
        #[arg(long)]
        name: Option<String>,
    },
    /// Check workspace, VOICEVOX, FFmpeg, output directory, template and platform capabilities
    Doctor(TtsArgs),
    /// Parse a script and resolve speakers without generating anything
    Validate {
        /// Script path (scripts/xxx.md)
        script: PathBuf,
    },
    /// Generate audio, project.vfp.json, captions.srt and preview.mp4 from a script
    Generate {
        /// Script path (scripts/xxx.md)
        script: PathBuf,
        /// Skip preview.mp4 even if enabled in videoforge.yaml
        #[arg(long)]
        no_preview: bool,
        /// Prefix SRT cues with the speaker name
        #[arg(long)]
        srt_speaker: bool,
        /// Do not read/write the TTS cache
        #[arg(long)]
        no_cache: bool,
        /// Keep .generated-tmp on failure for debugging
        #[arg(long)]
        keep_tmp: bool,
        #[command(flatten)]
        tts: TtsArgs,
    },
    /// List speakers/styles from the TTS engine (to fill `speaker_id` in videoforge.yaml)
    Speakers(TtsArgs),
    /// Export a project to an editor format
    Export {
        #[command(subcommand)]
        target: ExportTarget,
    },
    /// Create a portable handoff bundle for another machine
    Bundle {
        #[command(subcommand)]
        target: BundleTarget,
    },
    /// Inspect or validate a character manifest (VOICEVOX + Live2D pipeline)
    Character {
        #[command(subcommand)]
        target: CharacterTarget,
    },
}

#[derive(Subcommand, Debug)]
enum CharacterTarget {
    /// Parse a character manifest and print each character's voice/model
    /// info (offline: no VOICEVOX connection, no workspace required)
    Inspect {
        /// Path to a character manifest YAML file
        manifest: PathBuf,
    },
    /// Like `inspect`, plus resolve each character's VOICEVOX speaker/style
    /// by name and confirm its Live2D model file is readable
    Validate {
        /// Path to a character manifest YAML file
        manifest: PathBuf,
        #[command(flatten)]
        tts: TtsArgs,
    },
}

#[derive(Subcommand, Debug)]
enum DraftAction {
    /// Print the prompt containing your supplied sources for an external AI
    Prompt { brief: String },
    /// Validate an AI response and print the hash of the version to review
    Check { brief: String, response: String },
    /// Export only a reviewed, structurally valid version into scripts/
    Export {
        brief: String,
        response: String,
        #[arg(long)]
        reviewed_hash: String,
        #[arg(long)]
        reviewer: String,
        #[arg(long)]
        out: String,
    },
}

#[derive(Subcommand, Debug)]
enum ExportTarget {
    /// Materialize a YukkuriMovieMaker4 project (.ymmp). Windows only.
    Ymm4 {
        /// Path to project.vfp.json
        project: PathBuf,
        /// Template .ymmp (default: template.ymmp next to the project, else videoforge.yaml export.ymm4.template)
        #[arg(long)]
        template: Option<PathBuf>,
        /// Output .ymmp path (default: <project dir>/ymm4/project.ymmp)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Open the result in YMM4 after export
        #[arg(long)]
        open: bool,
        /// Allow materialization on non-Windows hosts (paths will not be valid for YMM4; spikes/tests only)
        #[arg(long, hide = true)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
enum BundleTarget {
    /// Bundle project + assets + template for `videoforge export ymm4` on Windows
    Ymm4 {
        /// Path to project.vfp.json
        project: PathBuf,
        /// Output directory (default: <generated>/<slug>-ymm4-bundle)
        #[arg(long)]
        out: Option<PathBuf>,
        /// Template .ymmp to include
        #[arg(long)]
        template: Option<PathBuf>,
        /// Skip creating the .zip next to the directory
        #[arg(long)]
        no_zip: bool,
        /// Overwrite an existing --out directory, but only if it already
        /// looks like a VideoForge-generated bundle
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug, Clone, Default)]
struct TtsArgs {
    /// Use an in-process silent TTS engine instead of VOICEVOX (offline testing)
    #[arg(long)]
    fake_tts: bool,
    /// Override tts.endpoint from videoforge.yaml
    #[arg(long, value_name = "URL")]
    endpoint: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = commands::Context {
        workspace_override: cli.workspace,
        json: cli.json,
    };
    let result = match cli.command {
        Command::Draft { action } => commands::draft(&ctx, action),
        Command::Init { dir, name } => commands::init(&ctx, dir, name),
        Command::Doctor(tts) => commands::doctor(&ctx, tts.fake_tts, tts.endpoint).await,
        Command::Validate { script } => commands::validate(&ctx, script),
        Command::Generate {
            script,
            no_preview,
            srt_speaker,
            no_cache,
            keep_tmp,
            tts,
        } => {
            commands::generate(
                &ctx,
                commands::GenerateArgs {
                    script,
                    no_preview,
                    srt_speaker,
                    no_cache,
                    keep_tmp,
                    fake_tts: tts.fake_tts,
                    endpoint: tts.endpoint,
                },
            )
            .await
        }
        Command::Speakers(tts) => commands::speakers(&ctx, tts.fake_tts, tts.endpoint).await,
        Command::Export {
            target:
                ExportTarget::Ymm4 {
                    project,
                    template,
                    out,
                    open,
                    force,
                },
        } => commands::export_ymm4(&ctx, project, template, out, open, force).await,
        Command::Bundle {
            target:
                BundleTarget::Ymm4 {
                    project,
                    out,
                    template,
                    no_zip,
                    force,
                },
        } => commands::bundle_ymm4(&ctx, project, out, template, !no_zip, force),
        Command::Character { target } => match target {
            CharacterTarget::Inspect { manifest } => {
                commands::character_inspect(&ctx, manifest).await
            }
            CharacterTarget::Validate { manifest, tts } => {
                commands::character_validate(&ctx, manifest, tts.fake_tts, tts.endpoint).await
            }
        },
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            if ctx.json {
                // `code` is the stable identifier callers dispatch on; the
                // AppError may be wrapped in anyhow context, so walk the chain.
                let code = e
                    .chain()
                    .find_map(|c| c.downcast_ref::<videoforge_core::AppError>())
                    .map(videoforge_core::AppError::code)
                    .unwrap_or("other");
                let payload = serde_json::json!({
                    "ok": false,
                    "code": code,
                    "error": e.to_string(),
                });
                println!("{payload}");
            } else {
                eprintln!("Error:\n{e}");
            }
            ExitCode::from(1)
        }
    }
}
