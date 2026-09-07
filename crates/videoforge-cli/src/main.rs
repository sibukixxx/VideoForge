//! `videoforge` CLI (design §7). The CLI is the primary agent interface:
//! files in, files out, stable exit codes.

mod commands;
mod interchange_commands;

use std::path::PathBuf;
use std::process::ExitCode;
use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "videoforge", version, about = "Compile Markdown scripts into voice, captions, timeline, preview and editable video projects")]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Init { dir: Option<PathBuf>, #[arg(long)] name: Option<String> },
    Doctor(TtsArgs),
    Validate { script: PathBuf },
    Generate {
        script: PathBuf,
        #[arg(long)] no_preview: bool,
        #[arg(long)] srt_speaker: bool,
        #[arg(long)] no_cache: bool,
        #[arg(long)] keep_tmp: bool,
        #[command(flatten)] tts: TtsArgs,
    },
    Speakers(TtsArgs),
    Export { #[command(subcommand)] target: ExportTarget },
    Bundle { #[command(subcommand)] target: BundleTarget },
}

#[derive(Subcommand, Debug)]
enum ExportTarget {
    /// Materialize a YukkuriMovieMaker4 project (.ymmp). Windows only.
    Ymm4 {
        project: PathBuf,
        #[arg(long)] template: Option<PathBuf>,
        #[arg(long)] out: Option<PathBuf>,
        #[arg(long)] open: bool,
        #[arg(long, hide = true)] force: bool,
    },
    /// Export Final Cut Pro XML for handoff to Final Cut Pro / compatible NLEs.
    Fcpxml { project: PathBuf, #[arg(long)] out: Option<PathBuf> },
    /// Export OpenTimelineIO JSON for NLE-agnostic interchange and automation.
    Otio { project: PathBuf, #[arg(long)] out: Option<PathBuf> },
}

#[derive(Subcommand, Debug)]
enum BundleTarget {
    Ymm4 {
        project: PathBuf,
        #[arg(long)] out: Option<PathBuf>,
        #[arg(long)] template: Option<PathBuf>,
        #[arg(long)] no_zip: bool,
        #[arg(long)] force: bool,
    },
}

#[derive(Args, Debug, Clone, Default)]
struct TtsArgs {
    #[arg(long)] fake_tts: bool,
    #[arg(long, value_name = "URL")] endpoint: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let ctx = commands::Context { workspace_override: cli.workspace, json: cli.json };
    let result = match cli.command {
        Command::Init { dir, name } => commands::init(&ctx, dir, name),
        Command::Doctor(tts) => commands::doctor(&ctx, tts.fake_tts, tts.endpoint).await,
        Command::Validate { script } => commands::validate(&ctx, script),
        Command::Generate { script, no_preview, srt_speaker, no_cache, keep_tmp, tts } => {
            commands::generate(&ctx, commands::GenerateArgs { script, no_preview, srt_speaker, no_cache, keep_tmp, fake_tts: tts.fake_tts, endpoint: tts.endpoint }).await
        }
        Command::Speakers(tts) => commands::speakers(&ctx, tts.fake_tts, tts.endpoint).await,
        Command::Export { target: ExportTarget::Ymm4 { project, template, out, open, force } } => commands::export_ymm4(&ctx, project, template, out, open, force).await,
        Command::Export { target: ExportTarget::Fcpxml { project, out } } => interchange_commands::export(project, out, "fcpxml", ctx.json).await,
        Command::Export { target: ExportTarget::Otio { project, out } } => interchange_commands::export(project, out, "otio", ctx.json).await,
        Command::Bundle { target: BundleTarget::Ymm4 { project, out, template, no_zip, force } } => commands::bundle_ymm4(&ctx, project, out, template, !no_zip, force),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            if ctx.json {
                let code = e.chain().find_map(|c| c.downcast_ref::<videoforge_core::AppError>()).map(videoforge_core::AppError::code).unwrap_or("other");
                println!("{}", serde_json::json!({"ok":false,"code":code,"error":e.to_string()}));
            } else { eprintln!("Error:\n{e}"); }
            ExitCode::from(1)
        }
    }
}
