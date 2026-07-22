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

use poid_broker::{ConnectionId, ConnectionKind, Origin};
use poid_connections::{ConnectionRecord, NewConnection};
use tauri::{State, WebviewWindow};
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

/// Removes a connection and its credential.
#[tauri::command]
pub fn connection_remove(
    window: WebviewWindow,
    connections: State<'_, ConnectionsState>,
    id: String,
) -> Result<(), String> {
    hub_only(window.label())?;
    connections
        .with(|store| store.remove(&ConnectionId::new(id)))
        .map_err(|e| e.to_string())
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
