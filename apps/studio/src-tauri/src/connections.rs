//! The connection manager's IPC surface (SPEC §7.2, M11.3).
//!
//! Two rules govern every command in this file.
//!
//! **Nothing here returns a credential.** There is no `connection_secret`
//! command and there must never be one. The manager UI shows whether a
//! credential is stored, never what it is — so the secret field in the UI is
//! write-only, and editing a connection means replacing the credential rather
//! than seeing the old one. That is mildly inconvenient once and removes an
//! entire class of disclosure permanently.
//!
//! **Only the hub may manage connections.** Every command checks the calling
//! window's label. A Reader window hosts a document, and a document's window
//! has no business adding, renaming or deleting the user's database
//! credentials — even though the sandboxed application inside it already
//! cannot reach Tauri IPC at all. Defence in depth is cheap here: one
//! comparison.

use poid_broker::binding::{candidates, BindingRequest};
use poid_broker::{ConnectionId, ConnectionKind, NetworkPolicy, Origin, RequireKind};
use poid_connections::{
    brokered_fetch, ConnectionRecord, FetchLimits, FetchRequest, NewConnection, OriginCredentials,
    RecordedBinding,
};
use tauri::{Manager, State, WebviewWindow};
use uuid::Uuid;

use crate::connections_state::ConnectionsState;

/// The label of the one hub window (see `windows::focus_or_create_studio`).
const HUB_LABEL: &str = "studio";

/// A connection as the manager UI sees it: identity, kind, a human-readable
/// target, and whether a credential is stored. Never the credential.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDto {
    /// Reader-assigned identifier.
    pub id: String,
    /// `sql` or `net`.
    pub kind: String,
    /// The user's own name for it.
    pub label: String,
    /// A non-secret one-line description: `app@db.example.com:5432/appdb`, or
    /// the covered origins.
    pub target: String,
    /// Whether the OS keychain currently holds a credential for it.
    ///
    /// Checked rather than assumed: keychain items can be deleted from
    /// outside Studio, and a connection whose credential has vanished should
    /// say so instead of failing at the moment the user tries to use it.
    pub has_secret: bool,
}

fn dto(record: &ConnectionRecord, has_secret: bool) -> ConnectionDto {
    ConnectionDto {
        id: record.id.to_string(),
        kind: record.kind().as_str().to_owned(),
        label: record.label.clone(),
        target: record.config.display(),
        has_secret,
    }
}

/// Refuses any window that is not the hub.
///
/// Takes the label rather than the window so the rule itself is testable
/// without a running webview — the check is the security-relevant part, and a
/// check nobody can test is a check nobody will notice breaking.
fn hub_only(label: &str) -> Result<(), String> {
    if label == HUB_LABEL {
        Ok(())
    } else {
        Err("connections can only be managed from the Studio hub".to_owned())
    }
}

/// Lists the configured connections.
#[tauri::command]
pub fn connections_list(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
) -> Result<Vec<ConnectionDto>, String> {
    hub_only(window.label())?;
    connections
        .with(|store| {
            Ok(store
                .records()
                .iter()
                .map(|record| {
                    let has = store.has_secret(&record.id).unwrap_or(false);
                    dto(record, has)
                })
                .collect())
        })
        .map_err(|e| e.to_string())
}

/// Adds a connection, or replaces the credential and configuration of one
/// that already exists.
///
/// `secret` is the full DSN for `sql`, or the token for `net`. It arrives
/// here from the hub's own form and goes straight to the keychain; it is not
/// stored anywhere else, echoed back, or logged.
#[tauri::command]
pub fn connection_save(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    id: Option<String>,
    kind: String,
    label: String,
    secret: String,
    origins: Vec<String>,
) -> Result<ConnectionDto, String> {
    hub_only(window.label())?;

    if label.trim().is_empty() {
        return Err("give the connection a name so you can recognise it later".to_owned());
    }

    let input = match kind.as_str() {
        "sql" => NewConnection::Sql { dsn: secret },
        "net" => {
            let parsed: Result<Vec<Origin>, _> = origins.iter().map(|o| Origin::parse(o)).collect();
            NewConnection::Net {
                origins: parsed.map_err(|e| e.to_string())?,
                token: secret,
            }
        }
        other => return Err(format!("`{other}` connections are not supported yet")),
    };

    // A new connection gets a fresh opaque id. The user never types one, so
    // it cannot collide with, or be made to impersonate, an existing entry.
    let id = ConnectionId::new(id.unwrap_or_else(|| Uuid::new_v4().to_string()));

    connections
        .with(|store| {
            store.upsert(id.clone(), label.trim(), input)?;
            let record =
                store
                    .get(&id)
                    .ok_or_else(|| poid_connections::ConnectionError::NotFound {
                        connection: id.to_string(),
                    })?;
            let has = store.has_secret(&id).unwrap_or(false);
            Ok(dto(record, has))
        })
        .map_err(|e| e.to_string())
}

/// Renames a connection. The credential is untouched.
#[tauri::command]
pub fn connection_rename(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    id: String,
    label: String,
) -> Result<(), String> {
    hub_only(window.label())?;
    if label.trim().is_empty() {
        return Err("give the connection a name so you can recognise it later".to_owned());
    }
    connections
        .with(|store| store.rename(&ConnectionId::new(id), label.trim()))
        .map_err(|e| e.to_string())
}

/// Removes a connection, its credential, and every binding that named it.
#[tauri::command]
pub fn connection_remove(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    id: String,
) -> Result<(), String> {
    hub_only(window.label())?;
    let id = ConnectionId::new(id);
    connections
        .with(|store| store.remove(&id))
        .map_err(|e| e.to_string())?;
    // Bindings that pointed here would otherwise send every affected document
    // to an error on open. Forgetting them sends it to the prompt instead,
    // which is the honest answer to "the database you chose is gone".
    connections
        .with_bindings(|bindings| bindings.forget_connection(&id))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Reports whether the OS keychain still holds this connection's credential.
///
/// Deliberately *not* named `connection_test`: it checks storage, not
/// reachability. Actually opening the backend arrives with the driver
/// (M11.7), and calling this a connection test before then would be a claim
/// the code does not support.
#[tauri::command]
pub fn connection_check_credential(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    id: String,
) -> Result<bool, String> {
    hub_only(window.label())?;
    connections
        .with(|store| store.has_secret(&ConnectionId::new(id)))
        .map_err(|e| e.to_string())
}

/// The connection kinds this build can configure.
///
/// Reported rather than hard-coded in the UI so the hub cannot offer a kind
/// the backend has not implemented — the honest failure is an absent option,
/// not an error after the user has typed a credential.
#[tauri::command]
pub fn connection_kinds() -> Vec<&'static str> {
    vec![ConnectionKind::Sql.as_str(), ConnectionKind::Net.as_str()]
}

// ----------------------------------------------------------------- binding
//
// The commands below serve **Reader** windows, not the hub, and are read-only
// about connections: they list what could serve this document and record which
// the user picked (SPEC §7.2.3). None of them can reach a credential.
//
// Scope discipline: the document is resolved through the calling window's
// label, exactly as every vault command is. There is deliberately no parameter
// naming an app id, an instance id, or another window — so no window can read
// or overwrite another document's binding.

/// The identity of the document a Reader window holds.
fn window_document(window: &WebviewWindow) -> Result<(String, String), String> {
    let dto = window
        .app_handle()
        .state::<crate::state::Documents>()
        .get(window.label())
        .ok_or("this window has no document")?;
    let crate::document::DocumentDto::Loaded { manifest_json, .. } = dto else {
        return Err("a rejected document has no storage binding".to_owned());
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_json).map_err(|_| "unreadable manifest".to_owned())?;
    let app_id = manifest
        .pointer("/app/id")
        .and_then(|v| v.as_str())
        .ok_or("this document has no app id")?
        .to_owned();
    let instance_id = manifest
        .pointer("/instance/id")
        .and_then(|v| v.as_str())
        .ok_or("this document has no instance id")?
        .to_owned();
    Ok((app_id, instance_id))
}

/// What the reader needs to draw the binding prompt (SPEC §7.2.3).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingOptions {
    /// Connections that can serve this document's declared need, in the order
    /// to offer them.
    pub candidates: Vec<ConnectionDto>,
    /// The recorded decision: `"unset"`, `"local"`, or a connection id.
    ///
    /// `"unset"` and `"local"` are different on purpose. The first means the
    /// user has not been asked; the second means they were asked and said no,
    /// and re-asking would turn a consent prompt into nagware.
    pub recorded: String,
}

/// Lists the connections that could serve this document, plus what the user
/// already decided.
///
/// `require` is the manifest's `storage.requires.kind`, read by the Reader
/// from its own manifest — an application cannot influence it, because the
/// application never speaks to Tauri IPC at all.
#[tauri::command]
pub fn connection_choices(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    require: String,
    hint: Option<String>,
) -> Result<BindingOptions, String> {
    let (app_id, instance_id) = window_document(&window)?;
    let require = parse_require(&require)?;
    let request = BindingRequest { require, hint };

    let refs_and_records = connections
        .with(|store| {
            let refs = store.refs();
            let candidates = candidates(&request, &refs);
            Ok(candidates
                .iter()
                .filter_map(|c| {
                    store
                        .get(&c.id)
                        .map(|record| (record.clone(), c.id.clone()))
                })
                .map(|(record, id)| {
                    let has = store.has_secret(&id).unwrap_or(false);
                    dto(&record, has)
                })
                .collect::<Vec<_>>())
        })
        .map_err(|e| e.to_string())?;

    let recorded = connections
        .with_bindings(|bindings| {
            Ok(match bindings.get(&app_id, &instance_id) {
                None => "unset".to_owned(),
                Some(RecordedBinding::Local) => "local".to_owned(),
                Some(RecordedBinding::Connection { id }) => id.to_string(),
            })
        })
        .map_err(|e| e.to_string())?;

    Ok(BindingOptions {
        candidates: refs_and_records,
        recorded,
    })
}

/// Records the user's binding decision for this document.
///
/// `choice` is `"local"` or a connection id. A named connection is checked
/// against the document's declared need before it is recorded, so a UI bug
/// cannot bind a kanban board to a connection that could never serve it.
#[tauri::command]
pub fn connection_bind(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    require: String,
    choice: String,
) -> Result<(), String> {
    let (app_id, instance_id) = window_document(&window)?;
    let require = parse_require(&require)?;

    let binding = if choice == "local" {
        RecordedBinding::Local
    } else {
        let id = ConnectionId::new(choice);
        let usable = connections
            .with(|store| {
                Ok(store
                    .get(&id)
                    .map(|record| record.to_ref().satisfies(require)))
            })
            .map_err(|e| e.to_string())?;
        match usable {
            None => return Err("that connection is no longer configured".to_owned()),
            Some(false) => return Err("that connection cannot store this kind of data".to_owned()),
            Some(true) => RecordedBinding::Connection { id },
        }
    };

    connections
        .with_bindings(|bindings| bindings.set(&app_id, &instance_id, binding))
        .map_err(|e| e.to_string())
}

/// Brings the Studio hub forward so the user can add a connection without
/// losing the document window they are in.
#[tauri::command]
pub fn open_hub(app: tauri::AppHandle) {
    crate::windows::focus_or_create_studio(&app);
}

// -------------------------------------------------------------- poid.net
//
// The first outbound request POID makes for an application. Everything that
// decides whether it may happen is in `poid-broker`; everything that performs
// it is in `poid-connections`. This command's only job is to assemble the two
// from *this window's* document — never from a parameter.

/// A brokered response, shaped for the host-side broker.
///
/// No echo of the request: an application must not be able to read back the
/// headers that were actually sent, because those carry the credential.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchResponseDto {
    /// HTTP status code.
    pub status: u16,
    /// HTTP reason phrase.
    pub status_text: String,
    /// Response headers, minus the ones the broker withholds.
    pub headers: std::collections::HashMap<String, String>,
    /// Response body as text.
    pub body: String,
}

/// The origins this document may reach, from its own manifest.
///
/// The manifest is a *request* (SPEC §9.1); the grant is the user's consent to
/// run it. Today's consent screen is one Run/Cancel decision, so approving the
/// run approves the allowlist it declared — `runGrant` in `@poid/host` says
/// the same thing on the other side. Per-permission toggles are a later
/// milestone, and when they arrive the approved set arrives here instead of
/// being derived.
fn declared_origins(window: &WebviewWindow) -> Result<Vec<Origin>, String> {
    let dto = window
        .app_handle()
        .state::<crate::state::Documents>()
        .get(window.label())
        .ok_or("this window has no document")?;
    let crate::document::DocumentDto::Loaded { manifest_json, .. } = dto else {
        return Err("a rejected document has no network permission".to_owned());
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&manifest_json).map_err(|_| "unreadable manifest".to_owned())?;

    let mut origins = Vec::new();
    if let Some(list) = manifest
        .pointer("/permissions/network")
        .and_then(|v| v.as_array())
    {
        for entry in list.iter().filter_map(|v| v.as_str()) {
            // A malformed allowlist entry is dropped rather than failing the
            // whole request: the other entries are still a valid grant, and
            // the effect of dropping one is that it is unreachable.
            if let Ok(origin) = Origin::parse(entry) {
                origins.push(origin);
            }
        }
    }
    Ok(origins)
}

/// Performs `poid.net.fetch` on the application's behalf (SPEC §7.2.5).
#[tauri::command]
pub async fn net_fetch(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: Option<String>,
) -> Result<FetchResponseDto, String> {
    let declared = declared_origins(&window)?;
    let policy = NetworkPolicy::new(&declared, &declared);

    // Gather the credentials for the allowlisted origins *before* any await:
    // the registry lock is synchronous, and holding it across a network round
    // trip would stall every other window for the length of the request.
    let credentials = connections
        .with(|store| {
            let mut set = OriginCredentials::new();
            for record in store.records() {
                let reference = record.to_ref();
                if reference.kind != ConnectionKind::Net {
                    continue;
                }
                for origin in declared.iter().filter(|o| reference.covers(o)) {
                    if let Ok(secret) = store.secret(&record.id) {
                        set.insert(origin.clone(), secret);
                    }
                }
            }
            Ok(set)
        })
        .map_err(|e| e.to_string())?;

    let response = brokered_fetch(
        FetchRequest {
            url,
            method,
            headers,
            body,
        },
        &policy,
        &credentials,
        FetchLimits::default(),
    )
    .await
    // The reason is for the user's log; the host-side broker turns whatever
    // reaches it into a §9 code before the application sees anything.
    .map_err(|e| e.to_string())?;

    Ok(FetchResponseDto {
        status: response.status,
        status_text: response.status_text,
        headers: response.headers.into_iter().collect(),
        body: response.body,
    })
}

fn parse_require(value: &str) -> Result<RequireKind, String> {
    match value {
        "kv" => Ok(RequireKind::Kv),
        "sql" => Ok(RequireKind::Sql),
        "docs" => Ok(RequireKind::Docs),
        "files" => Ok(RequireKind::Files),
        other => Err(format!("`{other}` is not a storage requirement")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_hub_window_may_manage_connections() {
        assert!(hub_only(HUB_LABEL).is_ok());
        // Reader windows are labelled `reader-N` (see `windows::open_reader`).
        // A document window has no business touching the user's credentials,
        // even though the application inside it cannot reach IPC at all.
        for label in ["reader-1", "reader-42", "", "Studio", "studio-2"] {
            assert!(
                hub_only(label).is_err(),
                "`{label}` must not be able to manage connections"
            );
        }
    }

    #[test]
    fn the_dto_has_nowhere_to_put_a_credential() {
        // Serialising the whole DTO and checking the key set is the executable
        // form of "this type carries no secret": adding a field that did would
        // fail here rather than at review time.
        let dto = ConnectionDto {
            id: "c1".to_owned(),
            kind: "sql".to_owned(),
            label: "my-supabase".to_owned(),
            target: "app@db.example.com:5432/appdb".to_owned(),
            has_secret: true,
        };
        let json: serde_json::Value = serde_json::to_value(&dto).expect("the DTO serialises");
        // serde_json keeps object keys sorted, so the expectation is too.
        let keys: Vec<&str> = json
            .as_object()
            .map(|o| o.keys().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(keys, vec!["hasSecret", "id", "kind", "label", "target"]);
    }

    #[test]
    fn only_implemented_kinds_are_offered() {
        // The hub builds its dropdown from this. Offering a kind the backend
        // cannot store would fail only after the user typed a credential.
        assert_eq!(connection_kinds(), vec!["sql", "net"]);
    }
}
