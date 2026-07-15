//! Serde types for `manifest.json` and validation against SPEC §3.
//!
//! Unknown fields round-trip losslessly at every level: each struct carries a
//! `#[serde(flatten)]` map of extras (SPEC §3: unknown fields MUST be
//! preserved and MUST NOT cause rejection).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::error::ManifestError;
use crate::paths;

/// Unknown fields preserved for forward compatibility (SPEC §3).
pub type ExtraFields = serde_json::Map<String, serde_json::Value>;

/// The parsed `manifest.json` — the contract of a container (SPEC §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Spec version implemented by this manifest (`"1.0"`).
    pub poid: String,
    /// Container type (SPEC §4).
    #[serde(rename = "type")]
    pub container_type: ContainerType,
    /// What program this is. Required for `app` and `workspace` (SPEC §3.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<AppInfo>,
    /// Which copy this is. Required for `app` and `workspace` (SPEC §3.2, §6.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<Instance>,
    /// `true` = opens in Studio, not Reader. Absent means `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<bool>,
    /// Runtime profile and engines. Required for `app` and `workspace` (SPEC §5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    /// Path of the HTML entry point. Required for `app` (SPEC §3.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Where application state lives. Required for `app` and `workspace` (SPEC §6).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<Storage>,
    /// Requested permissions — a request, not a grant (SPEC §9.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<Permissions>,
    /// Shared namespaces for nested apps; `workspace` only (SPEC §10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_scope: Option<Vec<String>>,
    /// Which application a `data` container belongs to. Required for `data` (SPEC §11).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_ref: Option<DataRef>,
    /// Content digests over `app/` and `deps/` (SPEC §3.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<Integrity>,
    /// Unknown fields, preserved verbatim (SPEC §3).
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Container type (SPEC §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerType {
    /// A runnable application. Opens in a Reader window.
    App,
    /// Data only — no code, inert by construction (SPEC §4.2).
    Data,
    /// Multiple nested POIDs under `apps/` (SPEC §10).
    Workspace,
}

/// The `app` block: identity of the program (SPEC §3.1, §3.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    /// Reverse-DNS identifier, stable across versions and copies.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Semantic version of the application.
    pub version: String,
    /// Author, free-form.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Short description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// SPDX license identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Icon path within the container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Reader window hints.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowHints>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Reader window hints (SPEC §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowHints {
    /// Initial width in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Initial height in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Minimum width in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_width: Option<u32>,
    /// Minimum height in logical pixels.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_height: Option<u32>,
    /// Whether the window may be resized.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resizable: Option<bool>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// The `instance` block: identity of this copy (SPEC §3.2, §6.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// `null` in a freshly packed POID; a UUIDv4 written by the reader on
    /// first open. Always serialized, even when `null`.
    pub id: Option<Uuid>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// The `runtime` block (SPEC §3.1, §5.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Runtime {
    /// Runtime profile: `web`, `web+python`, `web+sql`, …
    pub profile: String,
    /// Semver ranges for engines provided by the reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engines: Option<BTreeMap<String, String>>,
    /// Audit trail of dependencies bundled into `deps/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundled_deps: Option<Vec<String>>,
    /// Reproducibility record of the toolchain that built `app/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<ToolchainRecord>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// The `runtime.toolchain` reproducibility record (SPEC §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolchainRecord {
    /// The tool that packed this container, e.g. `poid-cli@1.0.0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub builder: Option<String>,
    /// The pinned esbuild version used for the build.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub esbuild: Option<String>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// The `storage` block (SPEC §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Storage {
    /// Where the data lives (SPEC §6.1).
    pub mode: StorageMode,
    /// `true` = multiple named states under `slots/` (SPEC §6.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots: Option<bool>,
    /// `true` = data encrypted at rest (SPEC §9.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protected: Option<bool>,
    /// Requested storage quota in MiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota_mb: Option<u64>,
    /// Backend requirement; required when `mode` is `connection`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requires: Option<StorageRequires>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Storage mode (SPEC §6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    /// Data lives in `data/` inside the container. The default.
    Embedded,
    /// Data lives in the reader's managed store, keyed by `instance.id`.
    Vault,
    /// Data lives in an external backend the user configured.
    Connection,
}

/// The `storage.requires` block (SPEC §3.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageRequires {
    /// The kind of backend the application needs.
    pub kind: RequireKind,
    /// Suggested provider, e.g. `supabase`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Backend kind for `storage.requires` (SPEC §3.1, §8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequireKind {
    /// Key-value store.
    Kv,
    /// Relational (SQL) store.
    Sql,
    /// Document store.
    Docs,
    /// Blob/file store.
    Files,
}

/// The `permissions` block — a request, not a grant (SPEC §9.1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Permissions {
    /// Allowlist of origins; empty or absent = no network.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<String>>,
    /// Filesystem access level.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<FilesystemAccess>,
    /// Clipboard access.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    /// Printing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub print: Option<bool>,
    /// Desktop notifications.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<bool>,
    /// MCP server ids the app may call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<Vec<String>>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Filesystem access level (SPEC §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilesystemAccess {
    /// No filesystem access at all.
    None,
    /// Only through a native file dialog the user opens.
    UserInitiated,
}

/// The `data_ref` block of a `type: data` container (SPEC §11).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataRef {
    /// `app.id` of the application the data belongs to.
    pub app_id: String,
    /// Version of that application.
    pub app_version: String,
    /// Schema identifier of the data, e.g. `responses/v1`.
    pub schema: String,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// The `integrity` block (SPEC §3.1, §3.3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Integrity {
    /// Digest algorithm. Only `sha256` in spec 1.0.
    pub algo: IntegrityAlgo,
    /// Digest of the `app/` tree; absent when the tree is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// Digest of the `deps/` tree; absent when the tree is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deps: Option<String>,
    /// Unknown fields, preserved verbatim.
    #[serde(flatten)]
    pub extra: ExtraFields,
}

/// Integrity digest algorithm (SPEC §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntegrityAlgo {
    /// SHA-256.
    #[serde(rename = "sha256")]
    Sha256,
}

impl Manifest {
    /// Parses `manifest.json` bytes. Unknown fields are collected, not rejected.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        serde_json::from_slice(bytes).map_err(|e| ManifestError::Syntax {
            message: e.to_string(),
        })
    }

    /// Serializes the manifest to pretty-printed JSON.
    ///
    /// Output is deterministic: struct fields in declaration order, unknown
    /// fields appended in sorted order.
    pub fn to_json_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        serde_json::to_vec_pretty(self).map_err(|e| ManifestError::Syntax {
            message: e.to_string(),
        })
    }

    /// A minimal valid `type: app` manifest scaffold.
    pub fn new_app(
        app_id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
        entry: impl Into<String>,
    ) -> Self {
        Self {
            poid: crate::SPEC_VERSION.to_owned(),
            container_type: ContainerType::App,
            app: Some(AppInfo {
                id: app_id.into(),
                name: name.into(),
                version: version.into(),
                author: None,
                description: None,
                license: None,
                icon: None,
                window: None,
                extra: ExtraFields::new(),
            }),
            instance: Some(Instance {
                id: None,
                extra: ExtraFields::new(),
            }),
            draft: None,
            runtime: Some(Runtime {
                profile: "web".to_owned(),
                engines: None,
                bundled_deps: None,
                toolchain: None,
                extra: ExtraFields::new(),
            }),
            entry: Some(entry.into()),
            storage: Some(Storage {
                mode: StorageMode::Embedded,
                slots: None,
                protected: None,
                quota_mb: None,
                requires: None,
                extra: ExtraFields::new(),
            }),
            permissions: Some(Permissions::default()),
            shared_scope: None,
            data_ref: None,
            integrity: Some(Integrity {
                algo: IntegrityAlgo::Sha256,
                app: None,
                deps: None,
                extra: ExtraFields::new(),
            }),
            extra: ExtraFields::new(),
        }
    }

    /// A minimal valid `type: data` manifest scaffold (SPEC §11).
    pub fn new_data(
        app_id: impl Into<String>,
        app_version: impl Into<String>,
        schema: impl Into<String>,
    ) -> Self {
        Self {
            poid: crate::SPEC_VERSION.to_owned(),
            container_type: ContainerType::Data,
            app: None,
            instance: None,
            draft: None,
            runtime: None,
            entry: None,
            storage: None,
            permissions: None,
            shared_scope: None,
            data_ref: Some(DataRef {
                app_id: app_id.into(),
                app_version: app_version.into(),
                schema: schema.into(),
                extra: ExtraFields::new(),
            }),
            integrity: Some(Integrity {
                algo: IntegrityAlgo::Sha256,
                app: None,
                deps: None,
                extra: ExtraFields::new(),
            }),
            extra: ExtraFields::new(),
        }
    }

    /// Validates the manifest against SPEC §3 rules.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.poid != crate::SPEC_VERSION {
            return Err(ManifestError::UnsupportedVersion {
                found: self.poid.clone(),
            });
        }

        match self.container_type {
            ContainerType::App | ContainerType::Workspace => {
                let app = require(self.app.as_ref(), "app")?;
                validate_app(app)?;
                require(self.instance.as_ref(), "instance")?;
                let runtime = require(self.runtime.as_ref(), "runtime")?;
                validate_runtime(runtime)?;
                let storage = require(self.storage.as_ref(), "storage")?;
                validate_storage(storage)?;
                require(self.permissions.as_ref(), "permissions")?;
                if self.container_type == ContainerType::App {
                    let entry = require(self.entry.as_ref(), "entry")?;
                    check_manifest_path("entry", entry)?;
                }
            }
            ContainerType::Data => {
                let data_ref = require(self.data_ref.as_ref(), "data_ref")?;
                if !is_reverse_dns(&data_ref.app_id) {
                    return Err(ManifestError::InvalidId {
                        field: "data_ref.app_id",
                        value: data_ref.app_id.clone(),
                    });
                }
                if !is_semver(&data_ref.app_version) {
                    return Err(ManifestError::InvalidVersion {
                        field: "data_ref.app_version",
                        value: data_ref.app_version.clone(),
                    });
                }
            }
        }

        let integrity = require(self.integrity.as_ref(), "integrity")?;
        for (field, digest) in [
            ("integrity.app", integrity.app.as_ref()),
            ("integrity.deps", integrity.deps.as_ref()),
        ] {
            if let Some(d) = digest {
                if !is_hex64(d) {
                    return Err(ManifestError::InvalidDigest { field });
                }
            }
        }

        Ok(())
    }
}

fn require<'a, T>(value: Option<&'a T>, field: &'static str) -> Result<&'a T, ManifestError> {
    value.ok_or(ManifestError::MissingField { field })
}

fn validate_app(app: &AppInfo) -> Result<(), ManifestError> {
    if !is_reverse_dns(&app.id) {
        return Err(ManifestError::InvalidId {
            field: "app.id",
            value: app.id.clone(),
        });
    }
    if app.name.is_empty() {
        return Err(ManifestError::MissingField { field: "app.name" });
    }
    if !is_semver(&app.version) {
        return Err(ManifestError::InvalidVersion {
            field: "app.version",
            value: app.version.clone(),
        });
    }
    if let Some(icon) = &app.icon {
        check_manifest_path("app.icon", icon)?;
    }
    if let Some(window) = &app.window {
        for (field, value) in [
            ("app.window.width", window.width),
            ("app.window.height", window.height),
            ("app.window.min_width", window.min_width),
            ("app.window.min_height", window.min_height),
        ] {
            if value == Some(0) {
                return Err(ManifestError::InvalidNumber { field });
            }
        }
    }
    Ok(())
}

fn validate_runtime(runtime: &Runtime) -> Result<(), ManifestError> {
    if !is_profile(&runtime.profile) {
        return Err(ManifestError::InvalidProfile {
            value: runtime.profile.clone(),
        });
    }
    Ok(())
}

fn validate_storage(storage: &Storage) -> Result<(), ManifestError> {
    if storage.mode == StorageMode::Connection && storage.requires.is_none() {
        return Err(ManifestError::ConnectionRequires);
    }
    if storage.quota_mb == Some(0) {
        return Err(ManifestError::InvalidNumber {
            field: "storage.quota_mb",
        });
    }
    Ok(())
}

fn check_manifest_path(field: &'static str, value: &str) -> Result<(), ManifestError> {
    paths::check_container_path(value).map_err(|_| ManifestError::InvalidPath {
        field,
        value: value.to_owned(),
    })
}

/// `com.example.kanban` — first label starts with a letter, at least two labels.
fn is_reverse_dns(s: &str) -> bool {
    let mut labels = s.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    let mut chars = first.chars();
    let starts_alpha = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if !starts_alpha || !chars.all(|c| c.is_ascii_alphanumeric()) {
        return false;
    }
    let mut rest = 0usize;
    for label in labels {
        rest += 1;
        if label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return false;
        }
    }
    rest >= 1
}

/// `X.Y.Z` with optional `-prerelease` and `+build` parts.
fn is_semver(s: &str) -> bool {
    let (rest, build) = match s.split_once('+') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    let (core, pre) = match rest.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (rest, None),
    };
    let suffix_ok = |part: Option<&str>| {
        part.is_none_or(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        })
    };
    if !suffix_ok(pre) || !suffix_ok(build) {
        return false;
    }
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// 64 lowercase hex characters.
fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

/// `web` optionally followed by `+engine` segments (SPEC §5.4).
fn is_profile(s: &str) -> bool {
    let mut parts = s.split('+');
    parts.next() == Some("web")
        && parts.all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_app_manifest_is_valid() {
        let m = Manifest::new_app("com.example.kanban", "Kanban", "1.0.0", "app/index.html");
        m.validate().expect("minimal app manifest must validate");
    }

    #[test]
    fn minimal_data_manifest_is_valid() {
        let m = Manifest::new_data("com.example.survey", "1.0.0", "responses/v1");
        m.validate().expect("minimal data manifest must validate");
    }

    #[test]
    fn unknown_fields_round_trip() {
        let json = r#"{
            "poid": "1.0",
            "type": "app",
            "x_top_level": {"a": 1},
            "app": {"id": "com.example.x", "name": "X", "version": "0.1.0", "x_nested": true},
            "instance": {"id": null},
            "runtime": {"profile": "web"},
            "entry": "app/index.html",
            "storage": {"mode": "embedded"},
            "permissions": {"network": [], "x_perm": "keep-me"},
            "integrity": {"algo": "sha256"}
        }"#;
        let m = Manifest::parse(json.as_bytes()).expect("parses");
        m.validate().expect("validates");
        let out = m.to_json_bytes().expect("serializes");
        let round: serde_json::Value = serde_json::from_slice(&out).expect("json");
        assert_eq!(round["x_top_level"]["a"], 1);
        assert_eq!(round["app"]["x_nested"], true);
        assert_eq!(round["permissions"]["x_perm"], "keep-me");
        let again = Manifest::parse(&out).expect("reparses");
        assert_eq!(m, again);
    }

    #[test]
    fn instance_id_serializes_as_null_when_absent() {
        let m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        let v: serde_json::Value =
            serde_json::from_slice(&m.to_json_bytes().expect("json")).expect("value");
        assert!(v["instance"].get("id").is_some());
        assert!(v["instance"]["id"].is_null());
    }

    #[test]
    fn missing_required_fields_have_stable_codes() {
        let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        m.entry = None;
        let err = m.validate().expect_err("entry required");
        assert_eq!(err.code(), "manifest-missing-field");

        let mut m = Manifest::new_data("com.example.x", "1.0.0", "v1");
        m.data_ref = None;
        let err = m.validate().expect_err("data_ref required");
        assert_eq!(err.code(), "manifest-missing-field");
    }

    #[test]
    fn version_and_id_rules() {
        let mut m = Manifest::new_app("no-dots", "X", "1.0.0", "app/index.html");
        assert_eq!(m.validate().expect_err("id").code(), "manifest-invalid-id");
        m = Manifest::new_app("com.example.x", "X", "1.0", "app/index.html");
        assert_eq!(
            m.validate().expect_err("semver").code(),
            "manifest-invalid-version"
        );
        m = Manifest::new_app(
            "com.example.x",
            "X",
            "1.0.0-beta.1+build5",
            "app/index.html",
        );
        m.validate().expect("prerelease+build is fine");
    }

    #[test]
    fn entry_path_rules() {
        let m = Manifest::new_app("com.example.x", "X", "1.0.0", "../evil.html");
        assert_eq!(
            m.validate().expect_err("traversal").code(),
            "manifest-invalid-path"
        );
    }

    #[test]
    fn connection_requires_requires() {
        let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        if let Some(s) = &mut m.storage {
            s.mode = StorageMode::Connection;
        }
        assert_eq!(
            m.validate().expect_err("requires").code(),
            "manifest-connection-requires"
        );
    }

    #[test]
    fn unsupported_version_rejected() {
        let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        m.poid = "2.0".to_owned();
        assert_eq!(
            m.validate().expect_err("version").code(),
            "manifest-unsupported-version"
        );
    }

    #[test]
    fn bad_digest_rejected() {
        let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
        if let Some(i) = &mut m.integrity {
            i.app = Some("XYZ".to_owned());
        }
        assert_eq!(
            m.validate().expect_err("digest").code(),
            "manifest-invalid-digest"
        );
    }

    #[test]
    fn profile_rules() {
        for good in ["web", "web+python", "web+sql", "web+python+sql"] {
            let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
            if let Some(r) = &mut m.runtime {
                r.profile = good.to_owned();
            }
            m.validate().unwrap_or_else(|_| panic!("{good} must be ok"));
        }
        for bad in ["", "python", "web+", "web+Python"] {
            let mut m = Manifest::new_app("com.example.x", "X", "1.0.0", "app/index.html");
            if let Some(r) = &mut m.runtime {
                r.profile = bad.to_owned();
            }
            assert!(m.validate().is_err(), "{bad} must fail");
        }
    }
}
