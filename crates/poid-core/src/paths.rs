//! Container path validation (SPEC §2.3).
//!
//! A container path is relative, uses `/` separators, and never escapes the
//! container: no `..` or `.` segments, no absolute paths, no drive letters,
//! no backslashes, no NUL bytes, no empty segments.

use crate::error::PoidError;

/// Checks that `path` is a well-formed container path that cannot escape.
pub(crate) fn check_container_path(path: &str) -> Result<(), PoidError> {
    if path.is_empty() {
        return Err(invalid(path, "empty path"));
    }
    if path.contains('\0') {
        return Err(invalid(path, "NUL byte"));
    }
    if path.contains('\\') {
        return Err(invalid(path, "backslash separator"));
    }
    if path.starts_with('/') {
        return Err(traversal(path, "absolute path"));
    }
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(traversal(path, "drive letter"));
    }
    for segment in path.split('/') {
        match segment {
            "" => return Err(invalid(path, "empty path segment")),
            "." => return Err(invalid(path, "`.` segment")),
            ".." => return Err(traversal(path, "`..` segment")),
            _ => {}
        }
    }
    Ok(())
}

fn invalid(path: &str, why: &'static str) -> PoidError {
    PoidError::InvalidPath {
        path: path.to_owned(),
        why,
    }
}

fn traversal(path: &str, why: &'static str) -> PoidError {
    PoidError::PathTraversal {
        path: path.to_owned(),
        why,
    }
}

#[cfg(test)]
mod tests {
    use super::check_container_path;

    #[test]
    fn accepts_normal_paths() {
        for p in ["app/index.html", "assets/icon.svg", "data/store.json", "a"] {
            assert!(check_container_path(p).is_ok(), "{p}");
        }
    }

    #[test]
    fn rejects_escapes_and_malformed() {
        for p in [
            "",
            "../evil",
            "app/../../evil",
            "/etc/passwd",
            "C:/evil",
            "c:evil",
            "app\\evil",
            "app//x",
            "./x",
            "app/./x",
            "app/",
            "a\0b",
        ] {
            assert!(check_container_path(p).is_err(), "{p:?} should be rejected");
        }
    }
}
