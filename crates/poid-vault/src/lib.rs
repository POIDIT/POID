//! POID vault: managed storage keyed by `instance.id`.
//!
//! The vault is a CRDT (Automerge), not a last-write-wins blob — SPEC §6.5.
//! This is a load-bearing decision: with CRDT semantics from day one,
//! offline editing on two devices merges losslessly, and synchronisation
//! (M12) becomes a transport problem instead of a storage rewrite.
//!
//! Crate layout:
//! - [`doc`] — the platform-pure document engine (slots, atomic kv values,
//!   merge, the operation log). This is all the WASM build gets.
//! - [`store`] — the desktop file store: one atomically-written file per
//!   instance, quota enforcement (`fs` feature).

#[cfg(feature = "fs")]
pub mod convert;
pub mod doc;
mod error;
#[cfg(feature = "fs")]
pub mod index;
pub mod protect;
#[cfg(feature = "fs")]
pub mod store;

pub use doc::{InstanceDoc, DEFAULT_SLOT};
pub use error::{Result, VaultError};
#[cfg(feature = "fs")]
pub use index::{hash_bytes, Disposition, IndexEntry, InstanceIndex};
pub use protect::{Envelope, KdfParams, PROTECTED_PATH};
#[cfg(feature = "fs")]
pub use store::{atomic_write, Vault, VaultInstance, DEFAULT_QUOTA_BYTES};

/// Name of the vault storage engine mandated by SPEC §6.5.
pub const STORAGE_ENGINE: &str = "automerge";
