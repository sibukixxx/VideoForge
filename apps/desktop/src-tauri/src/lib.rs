//! VideoForge desktop (design §24–§26, issue #13).
//!
//! This crate is a *composition root*, like `videoforge-cli/src/commands.rs`:
//! it picks the concrete `TtsEngine` / `PreviewRenderer` / `ProjectExporter`
//! and forwards to `videoforge-core`. No validation, pipeline or export logic
//! lives here — if the GUI needs a behaviour the CLI does not have, it goes
//! into core and both front ends get it.

mod commands;
mod error;
mod state;

pub use error::CommandError;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::platform_info,
            commands::doctor,
            commands::create_workspace,
            commands::open_workspace,
            commands::read_script,
            commands::write_script,
            commands::validate_script,
            commands::generate,
            commands::cancel_generate,
            commands::load_generated,
            commands::read_generated_file,
            commands::reveal_path,
            commands::export_ymm4,
            commands::open_in_ymm4,
            commands::bundle_ymm4,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VideoForge");
}
