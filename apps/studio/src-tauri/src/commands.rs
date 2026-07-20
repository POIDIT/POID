//! The IPC surface of the host UI (hub + reader chrome).
//!
//! These commands are reachable only from Studio's own windows: the
//! application inside the sandboxed iframe has an opaque origin with no
//! Tauri IPC object, and its one channel is the `postMessage` bridge
//! (`@poid/host`), which brokers a different, capability-checked protocol.
//! Nothing here trusts a scope parameter — see `reader_bootstrap`.

use tauri::{AppHandle, State, WebviewWindow};

use crate::document::DocumentDto;
use crate::state::Documents;
use crate::windows;

/// Returns the document this Reader window was opened for.
///
/// The lookup key is the **calling window's label** (security rule 3: derive
/// scope from the window). There is deliberately no variant that accepts a
/// label or a path from the caller.
#[tauri::command]
pub fn reader_bootstrap(
    window: WebviewWindow,
    documents: State<'_, Documents>,
) -> Result<DocumentDto, String> {
    documents
        .get(window.label())
        .ok_or_else(|| "no document is registered for this window".to_owned())
}

/// Opens `path` in a new Reader window (the hub's Open action).
#[tauri::command]
pub fn open_document(app: AppHandle, path: String) {
    windows::open_reader(&app, std::path::Path::new(&path));
}
