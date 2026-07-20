//! The desktop vault: one store for the whole process, shared by every
//! Reader window (ARCHITECTURE: one process, one vault).
//!
//! Scope discipline (security rule 3): windows register here keyed by their
//! **label**; every vault command resolves the instance through the calling
//! window's label. No IPC parameter can name another window's instance.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use poid_vault::{InstanceIndex, Vault, VaultInstance};
use uuid::Uuid;

/// Everything a Reader window's vault session needs.
pub struct VaultState {
    vault: Vault,
    index: Mutex<InstanceIndex>,
    /// Open instances, shared when two windows address one memory
    /// (Share memory).
    instances: Mutex<HashMap<Uuid, VaultInstance>>,
    /// Window label → the instance its document is keyed by.
    windows: Mutex<HashMap<String, Uuid>>,
}

impl VaultState {
    /// Opens the store and the index under `root` (the app data dir).
    pub fn open(root: PathBuf) -> poid_vault::Result<Self> {
        let vault = Vault::open(root.join("vault"))?;
        let index = InstanceIndex::load(vault.root())?;
        Ok(Self {
            vault,
            index: Mutex::new(index),
            instances: Mutex::new(HashMap::new()),
            windows: Mutex::new(HashMap::new()),
        })
    }

    /// The underlying store (conversion, fork copies).
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// Runs `f` with the instance index, persisting it afterwards.
    pub fn with_index<T>(&self, f: impl FnOnce(&mut InstanceIndex) -> T) -> poid_vault::Result<T> {
        let mut index = self
            .index
            .lock()
            .map_err(|_| poid_vault::VaultError::Corrupt {
                message: "index lock poisoned".to_owned(),
            })?;
        let out = f(&mut index);
        index.save()?;
        Ok(out)
    }

    /// Binds a window label to an instance id.
    pub fn bind_window(&self, label: &str, id: Uuid) {
        if let Ok(mut map) = self.windows.lock() {
            map.insert(label.to_owned(), id);
        }
    }

    /// Unbinds a closed window.
    pub fn unbind_window(&self, label: &str) {
        if let Ok(mut map) = self.windows.lock() {
            map.remove(label);
        }
    }

    /// The instance a window is bound to, if any.
    pub fn window_instance(&self, label: &str) -> Option<Uuid> {
        self.windows.lock().ok().and_then(|m| m.get(label).copied())
    }

    /// Runs `f` with the (lazily opened) instance for `id`. The instance is
    /// kept open for the life of the process so two Share-memory windows see
    /// one document.
    pub fn with_instance<T>(
        &self,
        id: Uuid,
        quota_bytes: u64,
        f: impl FnOnce(&mut VaultInstance) -> poid_vault::Result<T>,
    ) -> poid_vault::Result<T> {
        let mut instances = self
            .instances
            .lock()
            .map_err(|_| poid_vault::VaultError::Corrupt {
                message: "instance lock poisoned".to_owned(),
            })?;
        let inst = match instances.entry(id) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(self.vault.instance(id, quota_bytes)?)
            }
        };
        f(inst)
    }
}
