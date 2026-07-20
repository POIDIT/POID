//! The desktop synthetic origin's backing store (SPEC §5.2.1).
//!
//! The Reader window builds the served assets in TypeScript
//! (`ContainerServer.assets()` — one implementation for both readers) and
//! registers them here keyed by session. The `poid://` protocol handler then
//! serves them: a relative `<script src="app/main.js">` in the sandboxed
//! iframe resolves to `poid://localhost/<session>/app/main.js`, which this
//! registry answers with the exact bytes — and nothing else. The raw `.poid`
//! file, the vault, and the host are never at any URL the app can reach.

use std::collections::HashMap;
use std::sync::Mutex;

/// One served asset: its bytes and MIME type.
#[derive(Clone)]
pub struct Asset {
    /// MIME type for the `Content-Type` header.
    pub content_type: String,
    /// Response bytes.
    pub bytes: Vec<u8>,
}

/// Session → (container path → asset). Managed by Tauri as app state.
#[derive(Default)]
pub struct SessionAssets(Mutex<HashMap<String, HashMap<String, Asset>>>);

impl SessionAssets {
    /// Registers (replacing) all assets for a session.
    pub fn set(&self, session: String, assets: HashMap<String, Asset>) {
        if let Ok(mut map) = self.0.lock() {
            map.insert(session, assets);
        }
    }

    /// Looks up one asset by session and normalized path.
    pub fn get(&self, session: &str, path: &str) -> Option<Asset> {
        self.0
            .lock()
            .ok()
            .and_then(|map| map.get(session).and_then(|a| a.get(path).cloned()))
    }

    /// Drops a closed session's assets.
    pub fn remove(&self, session: &str) {
        if let Ok(mut map) = self.0.lock() {
            map.remove(session);
        }
    }
}

/// Splits a `poid://` request path into `(session, container_path)`.
///
/// The request URI is `poid://localhost/<session>/<path...>` (macOS/Linux) or
/// `http://poid.localhost/<session>/<path...>` (Windows/WebView2). Only the
/// path portion is passed here, already stripped of scheme+host. An empty
/// container path (the origin root) maps to the entry, handled by the caller.
pub fn split_request(path: &str) -> Option<(String, String)> {
    let trimmed = path.trim_start_matches('/');
    let (session, rest) = match trimmed.split_once('/') {
        Some((s, r)) => (s, r),
        None => (trimmed, ""),
    };
    if session.is_empty() {
        return None;
    }
    Some((session.to_owned(), rest.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{split_request, Asset, SessionAssets};

    #[test]
    fn splits_session_and_path() {
        assert_eq!(
            split_request("/s1/app/main.js"),
            Some(("s1".to_owned(), "app/main.js".to_owned()))
        );
        assert_eq!(
            split_request("s1/app/index.html"),
            Some(("s1".to_owned(), "app/index.html".to_owned()))
        );
        // Origin root: session only, empty container path (→ entry).
        assert_eq!(
            split_request("/s1/"),
            Some(("s1".to_owned(), "".to_owned()))
        );
        assert_eq!(split_request("/s1"), Some(("s1".to_owned(), "".to_owned())));
        assert_eq!(split_request("/"), None);
    }

    #[test]
    fn sessions_are_isolated() {
        let store = SessionAssets::default();
        let mut a = std::collections::HashMap::new();
        a.insert(
            "app/x.js".to_owned(),
            Asset {
                content_type: "text/javascript".to_owned(),
                bytes: b"A".to_vec(),
            },
        );
        store.set("s1".to_owned(), a);
        assert!(store.get("s1", "app/x.js").is_some());
        // A different session cannot read s1's assets.
        assert!(store.get("s2", "app/x.js").is_none());
        store.remove("s1");
        assert!(store.get("s1", "app/x.js").is_none());
    }
}
