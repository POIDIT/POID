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

mod asset_registry;
mod association;
mod cli;
mod commands;
mod connections;
mod connections_state;
mod document;
mod state;
mod vault_state;
mod windows;

use tauri::{Manager, WindowEvent};

use crate::asset_registry::{split_request, SessionAssets};
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
        .manage(SessionAssets::default())
        // The synthetic origin (SPEC §5.2.1): serve the sandboxed app and its
        // relative subresources from `poid://`, and nothing else. Registered
        // per session by the Reader window; the raw container, the vault, and
        // the host are never at any `poid://` URL.
        .register_asynchronous_uri_scheme_protocol("poid", |ctx, request, responder| {
            let assets = ctx.app_handle().state::<SessionAssets>();
            let uri = request.uri();
            let response = match split_request(uri.path()) {
                Some((session, path)) => {
                    // The origin root serves the entry; the reader registers the
                    // injected entry under its own container path AND at "".
                    match assets
                        .get(&session, &path)
                        .or_else(|| assets.get(&session, ""))
                    {
                        // ACAO: the sandboxed app has an *opaque* origin, so a
                        // `<script type="module">` (CORS-mode) fetch of a
                        // subresource is cross-origin and would be blocked
                        // without this header. Safe: these are the app's own
                        // public bytes, and `connect-src 'none'` still stops
                        // the app from reaching anything else.
                        Some(asset) => tauri::http::Response::builder()
                            .status(200)
                            .header("Content-Type", asset.content_type)
                            .header("Access-Control-Allow-Origin", "*")
                            .body(asset.bytes),
                        None => tauri::http::Response::builder()
                            .status(404)
                            .header("Content-Type", "text/plain")
                            .header("Access-Control-Allow-Origin", "*")
                            .body(b"not found".to_vec()),
                    }
                }
                None => tauri::http::Response::builder()
                    .status(404)
                    .header("Content-Type", "text/plain")
                    .body(b"not found".to_vec()),
            };
            match response {
                Ok(r) => responder.respond(r),
                Err(e) => eprintln!("poid-studio: poid:// response build failed: {e}"),
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::reader_bootstrap,
            commands::open_document,
            commands::resolve_copy_conflict,
            commands::synthetic_origin,
            commands::register_session_assets,
            commands::revoke_session_assets,
            commands::vault_hydrate,
            commands::vault_kv_set,
            commands::vault_kv_delete,
            commands::vault_kv_clear,
            commands::vault_switch_slot,
            commands::vault_sql_load,
            commands::vault_sql_save,
            connections::connections_list,
            connections::connection_save,
            connections::connection_rename,
            connections::connection_remove,
            connections::connection_check_credential,
            connections::connection_kinds
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Destroyed = event {
                window.state::<Documents>().remove(window.label());
                window
                    .state::<vault_state::VaultState>()
                    .unbind_window(window.label());
            }
        })
        .setup(move |app| {
            association::ensure(app.handle());
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("no app data dir: {e}"))?;
            let vault = vault_state::VaultState::open(data_dir.clone())
                .map_err(|e| format!("vault store unavailable: {e}"))?;
            app.manage(vault);
            let connections = connections_state::ConnectionsState::open(data_dir)
                .map_err(|e| format!("connection registry unavailable: {e}"))?;
            app.manage(connections);
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
