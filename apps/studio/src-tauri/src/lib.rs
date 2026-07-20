//! POID Studio — the one executable (M07: shell, windows, file association).
//!
//! Launch modes (ARCHITECTURE §2):
//! - `poid-studio --open <path>` (or a bare `*.poid` argument) → a Reader
//!   window for that document, and **only** that. The hub never appears.
//! - `poid-studio` → the Studio hub.
//!
//! One process serves every window: the single-instance plugin forwards the
//! argv of each later launch here, and this process answers with a new
//! Reader window. N documents = N windows, one process — like Excel.

mod association;
mod cli;
mod commands;
mod document;
mod state;
mod windows;

use tauri::{Manager, WindowEvent};

use crate::state::Documents;

/// Builds and runs the Studio application. Exits the process on a fatal
/// startup error (there is no UI to report into at that point).
pub fn run() {
    let launch = cli::open_request(std::env::args().skip(1));

    let builder = tauri::Builder::default()
        // Registered first so a second launch is forwarded before anything
        // else initializes (plugin docs require first position).
        .plugin(tauri_plugin_single_instance::init(
            |app, argv, _cwd| match cli::open_request(argv.into_iter().skip(1)) {
                Some(path) => windows::open_reader(app, &path),
                None => windows::focus_or_create_studio(app),
            },
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(Documents::default())
        .invoke_handler(tauri::generate_handler![
            commands::reader_bootstrap,
            commands::open_document
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Documents>().remove(window.label());
            }
        })
        .setup(move |app| {
            association::ensure(app.handle());
            match &launch {
                Some(path) => windows::open_reader(app.handle(), path),
                None => windows::focus_or_create_studio(app.handle()),
            }
            Ok(())
        });

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("poid-studio: failed to start: {e}");
            std::process::exit(1);
        }
    };

    app.run(|app_handle, event| {
        // macOS delivers double-clicked files as Opened events, not argv.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Opened { urls } = &event {
            for url in urls {
                if let Ok(path) = url.to_file_path() {
                    windows::open_reader(app_handle, &path);
                }
            }
        }
        let _ = (&app_handle, &event);
    });
}
