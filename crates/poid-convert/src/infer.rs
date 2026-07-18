//! Permission inference (M06 §5): read what the code actually uses and
//! request the most restrictive set that still works.
//!
//! Inference runs over the **built** output — after bundling, dead code is
//! gone, so a tree-shaken-away `navigator.clipboard` does not request the
//! clipboard. Network access is deliberately never inferred: an origin
//! allowlist is a statement of intent the author must write down
//! (SPEC §9.1, deny by default).

/// Permissions the converter inferred from the built code.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferredPermissions {
    /// `navigator.clipboard` / `poid.ui.clipboard` usage.
    pub clipboard: bool,
    /// `window.print` / `poid.ui.print` usage.
    pub print: bool,
    /// Notification API usage.
    pub notifications: bool,
    /// `poid.files` (user-initiated file access) usage.
    pub filesystem_user_initiated: bool,
}

/// Scans built JS (and inline HTML) for capability markers.
pub fn infer_permissions(built_sources: &[&str]) -> InferredPermissions {
    let mut out = InferredPermissions::default();
    for text in built_sources {
        if text.contains("navigator.clipboard") || text.contains("poid.ui.clipboard") {
            out.clipboard = true;
        }
        if text.contains("window.print") || text.contains("poid.ui.print") {
            out.print = true;
        }
        if text.contains("new Notification(") || text.contains("Notification.requestPermission") {
            out.notifications = true;
        }
        if text.contains("poid.files") {
            out.filesystem_user_initiated = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_nothing() {
        let inferred = infer_permissions(&["console.log('hello')"]);
        assert_eq!(inferred, InferredPermissions::default());
    }

    #[test]
    fn detects_used_capabilities() {
        let inferred = infer_permissions(&[
            "await navigator.clipboard.writeText(x); window.print();",
            "Notification.requestPermission(); poid.files.open();",
        ]);
        assert!(inferred.clipboard);
        assert!(inferred.print);
        assert!(inferred.notifications);
        assert!(inferred.filesystem_user_initiated);
    }

    #[test]
    fn network_is_never_inferred() {
        // fetch() in code does not put an origin into the manifest — there is
        // no field for it here at all; the author declares origins by hand.
        let inferred = infer_permissions(&["fetch('https://api.example.com')"]);
        assert_eq!(inferred, InferredPermissions::default());
    }
}
