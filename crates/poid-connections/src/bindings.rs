//! Which connection serves which document (SPEC §7.2.3).
//!
//! A binding is the user's answer to *"where should this POID keep its
//! data?"*, recorded so they are asked once rather than at every launch. It is
//! keyed by `app.id` + `instance.id` (SPEC §3.2), which is the pair that
//! identifies *this copy of this program*: two copies of the same kanban board
//! can point at two different databases, and that is a feature, not an
//! accident.
//!
//! Nothing here is secret — a binding names a connection id, and the id is
//! meaningless without the keychain entry behind it. It still never reaches an
//! application, because knowing *which* backend answered is itself a
//! disclosure (SPEC §7.2.4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use poid_broker::{Binding, ConnectionId};
use serde::{Deserialize, Serialize};

use crate::error::{ConnectionError, Result};

/// What the user chose for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "choice", rename_all = "lowercase")]
pub enum RecordedBinding {
    /// Serve storage from this connection.
    Connection {
        /// The connection's id.
        id: ConnectionId,
    },
    /// Keep the data in this reader's own store.
    ///
    /// A recorded *decline*, which is not the same as never having been asked:
    /// the first opens a prompt, the second must not re-open it at every
    /// launch. Conflating them is how a consent dialog becomes nagware.
    Local,
}

impl RecordedBinding {
    /// The broker's view of this decision.
    #[must_use]
    pub fn to_binding(&self) -> Binding {
        match self {
            Self::Connection { id } => Binding::Connection(id.clone()),
            Self::Local => Binding::KeepLocal,
        }
    }
}

/// The key identifying one document instance.
///
/// Length-prefixed rather than separated by a delimiter. A delimiter is only
/// unambiguous while no id can contain it, and "no id can contain it" is an
/// assumption about validated input rather than a property of this function —
/// exactly the kind of assumption that stops holding when someone adds a new
/// caller. With the length in front, `("a", "b:c")` and `("a:b", "c")` are
/// distinct keys whatever the ids contain.
fn key(app_id: &str, instance_id: &str) -> String {
    format!("{}:{}{}", app_id.len(), app_id, instance_id)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BindingsFile {
    version: u32,
    bindings: BTreeMap<String, RecordedBinding>,
}

const BINDINGS_VERSION: u32 = 1;

/// Every binding the user has recorded.
#[derive(Debug)]
pub struct BindingStore {
    path: PathBuf,
    bindings: BTreeMap<String, RecordedBinding>,
}

impl BindingStore {
    /// Opens the binding file at `path`, creating an empty set if absent.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Registry`] if the file exists but cannot be read or
    /// parsed.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let bindings = if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|_| ConnectionError::Registry {
                operation: "read",
                path: path.display().to_string(),
            })?;
            let file: BindingsFile =
                serde_json::from_str(&text).map_err(|_| ConnectionError::Registry {
                    operation: "parsed",
                    path: path.display().to_string(),
                })?;
            file.bindings
        } else {
            BTreeMap::new()
        };
        Ok(Self { path, bindings })
    }

    /// The decision recorded for this document, if the user has made one.
    ///
    /// `None` means *not yet asked*, and the reader must ask.
    #[must_use]
    pub fn get(&self, app_id: &str, instance_id: &str) -> Option<&RecordedBinding> {
        self.bindings.get(&key(app_id, instance_id))
    }

    /// Records the user's decision.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Registry`] if the file cannot be written.
    pub fn set(&mut self, app_id: &str, instance_id: &str, binding: RecordedBinding) -> Result<()> {
        self.bindings.insert(key(app_id, instance_id), binding);
        self.persist()
    }

    /// Forgets a decision, so the reader asks again next time.
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Registry`] if the file cannot be written.
    pub fn clear(&mut self, app_id: &str, instance_id: &str) -> Result<()> {
        self.bindings.remove(&key(app_id, instance_id));
        self.persist()
    }

    /// Drops every binding that names `connection`.
    ///
    /// Called when a connection is deleted: leaving bindings that point at a
    /// connection which no longer exists would send every affected document to
    /// an error on open. Forgetting them sends it to the prompt instead, which
    /// is the honest response to "the database you chose is gone".
    ///
    /// # Errors
    ///
    /// [`ConnectionError::Registry`] if the file cannot be written.
    pub fn forget_connection(&mut self, connection: &ConnectionId) -> Result<usize> {
        let before = self.bindings.len();
        self.bindings.retain(|_, b| match b {
            RecordedBinding::Connection { id } => id != connection,
            RecordedBinding::Local => true,
        });
        let removed = before - self.bindings.len();
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    fn persist(&self) -> Result<()> {
        let file = BindingsFile {
            version: BINDINGS_VERSION,
            bindings: self.bindings.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|_| ConnectionError::Registry {
            operation: "written",
            path: self.path.display().to_string(),
        })?;
        write_atomically(&self.path, text.as_bytes()).map_err(|_| ConnectionError::Registry {
            operation: "written",
            path: self.path.display().to_string(),
        })
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &tempfile::TempDir) -> BindingStore {
        BindingStore::open(dir.path().join("bindings.json")).expect("opens")
    }

    #[test]
    fn not_yet_asked_is_distinct_from_declined() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);

        // Never asked: the reader must prompt.
        assert!(store.get("com.example.kanban", "inst-1").is_none());

        // Declined: the reader must NOT prompt again. Treating these alike is
        // how a consent dialog turns into nagware the user clicks through.
        store
            .set("com.example.kanban", "inst-1", RecordedBinding::Local)
            .expect("records");
        assert_eq!(
            store.get("com.example.kanban", "inst-1"),
            Some(&RecordedBinding::Local)
        );
    }

    #[test]
    fn two_copies_of_one_app_bind_independently() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        store
            .set(
                "com.example.kanban",
                "inst-1",
                RecordedBinding::Connection {
                    id: ConnectionId::new("work-db"),
                },
            )
            .expect("records");
        store
            .set("com.example.kanban", "inst-2", RecordedBinding::Local)
            .expect("records");

        // Same program, different copies, different answers (SPEC §3.2).
        assert!(matches!(
            store.get("com.example.kanban", "inst-1"),
            Some(RecordedBinding::Connection { .. })
        ));
        assert_eq!(
            store.get("com.example.kanban", "inst-2"),
            Some(&RecordedBinding::Local)
        );
    }

    #[test]
    fn the_key_cannot_be_forged_by_choosing_an_id() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);

        // Every way of splitting the same concatenation must stay distinct,
        // whatever characters the ids contain — otherwise one document could
        // be made to read another's binding by choosing its instance id.
        for (app, instance) in [
            ("a", "b\u{0}c"),
            ("a\u{0}b", "c"),
            ("a", ":bc"),
            ("a:b", "c"),
            ("ab", "c"),
            ("a", "bc"),
        ] {
            store
                .set(app, instance, RecordedBinding::Local)
                .expect("records");
        }
        assert_eq!(store.bindings.len(), 6, "two id pairs shared a key");
    }

    #[test]
    fn deleting_a_connection_forgets_the_documents_bound_to_it() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut store = store(&dir);
        let gone = ConnectionId::new("work-db");
        store
            .set(
                "app.a",
                "i1",
                RecordedBinding::Connection { id: gone.clone() },
            )
            .expect("records");
        store
            .set(
                "app.b",
                "i2",
                RecordedBinding::Connection {
                    id: ConnectionId::new("other-db"),
                },
            )
            .expect("records");
        store
            .set("app.c", "i3", RecordedBinding::Local)
            .expect("records");

        assert_eq!(store.forget_connection(&gone).expect("forgets"), 1);
        // The affected document goes back to the prompt, not to an error.
        assert!(store.get("app.a", "i1").is_none());
        // Everything else is untouched.
        assert!(store.get("app.b", "i2").is_some());
        assert_eq!(store.get("app.c", "i3"), Some(&RecordedBinding::Local));
    }

    #[test]
    fn decisions_survive_a_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("bindings.json");
        {
            let mut store = BindingStore::open(&path).expect("opens");
            store
                .set(
                    "com.example.kanban",
                    "inst-1",
                    RecordedBinding::Connection {
                        id: ConnectionId::new("work-db"),
                    },
                )
                .expect("records");
        }
        let store = BindingStore::open(&path).expect("reopens");
        assert_eq!(
            store.get("com.example.kanban", "inst-1"),
            Some(&RecordedBinding::Connection {
                id: ConnectionId::new("work-db")
            })
        );
    }

    #[test]
    fn a_recorded_binding_converts_to_the_brokers_view() {
        assert_eq!(RecordedBinding::Local.to_binding(), Binding::KeepLocal);
        assert_eq!(
            RecordedBinding::Connection {
                id: ConnectionId::new("c1")
            }
            .to_binding(),
            Binding::Connection(ConnectionId::new("c1"))
        );
    }
}
