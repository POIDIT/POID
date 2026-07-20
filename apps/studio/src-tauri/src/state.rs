//! Per-window document registry.
//!
//! Security rule 3 in data-structure form: the map is keyed by **window
//! label**, and the only way a window gets a document is by asking for its
//! own label (`reader_bootstrap` reads the label from the calling window,
//! never from a parameter). A window cannot name — and therefore cannot
//! read — another window's document.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::document::DocumentDto;

/// Window-label → document map, managed by Tauri as app state.
#[derive(Default)]
pub struct Documents(Mutex<HashMap<String, DocumentDto>>);

impl Documents {
    /// Registers `dto` for a window label (called before the window exists,
    /// so the window's first ask can never race the insert).
    pub fn insert(&self, label: String, dto: DocumentDto) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(label, dto);
        }
    }

    /// The document for one window label, if any.
    pub fn get(&self, label: &str) -> Option<DocumentDto> {
        self.0.lock().ok().and_then(|map| map.get(label).cloned())
    }

    /// Drops a closed window's document (N readers must not leak N containers).
    pub fn remove(&self, label: &str) {
        if let Ok(mut map) = self.0.lock() {
            map.remove(label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Documents;
    use crate::document::DocumentDto;

    fn dto(name: &str) -> DocumentDto {
        DocumentDto::Rejected {
            registry: None,
            code: "io".into(),
            message: "test".into(),
            file_name: name.into(),
        }
    }

    fn name_of(dto: &DocumentDto) -> &str {
        match dto {
            DocumentDto::Rejected { file_name, .. } | DocumentDto::Loaded { file_name, .. } => {
                file_name
            }
        }
    }

    #[test]
    fn windows_see_only_their_own_document() {
        let docs = Documents::default();
        docs.insert("reader-1".into(), dto("a.poid"));
        docs.insert("reader-2".into(), dto("b.poid"));

        let one = docs.get("reader-1");
        let two = docs.get("reader-2");
        assert_eq!(one.as_ref().map(name_of), Some("a.poid"));
        assert_eq!(two.as_ref().map(name_of), Some("b.poid"));
        assert!(docs.get("reader-3").is_none());
    }

    #[test]
    fn removal_frees_the_slot() {
        let docs = Documents::default();
        docs.insert("reader-1".into(), dto("a.poid"));
        docs.remove("reader-1");
        assert!(docs.get("reader-1").is_none());
    }
}
