//! The two window classes (ARCHITECTURE §2) — and nothing that blurs them.
//!
//! A Reader window is a document: it exists because a file was opened, and
//! it shows only that file. The Studio window is the hub, reached only by a
//! bare launch (or, later, an explicit menu action). `open_reader` never
//! opens the hub; `focus_or_create_studio` never opens a document.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::document::{self, DocumentDto};
use crate::state::Documents;

/// Monotonic suffix for reader window labels; labels must be unique for the
/// life of the process, not just among live windows.
static READER_SEQ: AtomicU64 = AtomicU64::new(0);

/// Opens a new Reader window for `path`. A rejected container still gets a
/// window (with the explanation panel) — double-clicking a file must always
/// produce a visible, honest result.
pub fn open_reader(app: &AppHandle, path: &Path) {
    let (dto, bind) = document::load(path, &app.state::<crate::vault_state::VaultState>());
    let title = match &dto {
        DocumentDto::Rejected { file_name, .. } => file_name.clone(),
        DocumentDto::Loaded { file_name, .. } => file_name.clone(),
    };

    let label = format!("reader-{}", READER_SEQ.fetch_add(1, Ordering::Relaxed) + 1);
    app.state::<Documents>().insert(label.clone(), dto);
    if let Some(id) = bind {
        app.state::<crate::vault_state::VaultState>()
            .bind_window(&label, id);
    }

    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("reader.html".into()))
        .title(title)
        .inner_size(1000.0, 720.0)
        .min_inner_size(480.0, 320.0)
        .build();

    if let Err(e) = built {
        // No window means no UI to speak through; free the slot and log.
        app.state::<Documents>().remove(&label);
        eprintln!("poid-studio: failed to create a reader window: {e}");
    }
}

/// Brings the Studio hub to the front, creating it on first use. One hub,
/// ever — a second bare launch focuses the existing window (like Word's
/// start screen, not like a document).
pub fn focus_or_create_studio(app: &AppHandle) {
    if let Some(existing) = app.get_webview_window("studio") {
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return;
    }

    let built = WebviewWindowBuilder::new(app, "studio", WebviewUrl::App("index.html".into()))
        .title("POID Studio")
        .inner_size(1100.0, 760.0)
        .min_inner_size(640.0, 420.0)
        .build();

    if let Err(e) = built {
        eprintln!("poid-studio: failed to create the studio window: {e}");
    }
}
