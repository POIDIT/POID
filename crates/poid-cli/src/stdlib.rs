//! Loading Standard Library bundles from disk for the CLI's native build.
//!
//! Resolution and checksum verification are the shared converter's job
//! (`poid_convert::resolve` / `verify_bundle`); this module only knows *where*
//! the CLI keeps its library — `POID_STDLIB`, or `stdlib/` next to the
//! executable — and loads a resolved [`Selection`] from there, verified,
//! returning the path esbuild aliases to.

use std::path::{Path, PathBuf};

use poid_convert::Selection;

use crate::output::{err, CmdError};

/// Locates the Standard Library directory: `POID_STDLIB`, then `stdlib/`
/// next to the executable. Never downloads anything.
pub fn locate_dir() -> Result<PathBuf, CmdError> {
    let candidate = match std::env::var_os("POID_STDLIB") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stdlib"),
    };
    if !candidate.is_dir() {
        return Err(err(
            "stdlib-missing",
            format!(
                "this project imports Standard Library packages, but no library was found at \
                 `{}`. Point the POID_STDLIB environment variable at a built library \
                 (pnpm --filter @poid/stdlib build:lib produces one in \
                 packages/poid-stdlib/lib). Nothing is ever downloaded silently.",
                candidate.display()
            ),
        ));
    }
    Ok(candidate)
}

/// Loads one selection from the library directory, verifying its bytes against
/// the catalog checksum (via the shared converter), and returns its path for
/// esbuild to alias.
pub fn load_verified(dir: &Path, selection: &Selection) -> Result<PathBuf, CmdError> {
    let path = dir.join(&selection.rel);
    let content = std::fs::read(&path).map_err(|_| {
        err(
            "stdlib-incomplete",
            format!(
                "the Standard Library at `{}` has no `{}` — rebuild it \
                 (pnpm --filter @poid/stdlib build:lib)",
                dir.display(),
                selection.rel
            ),
        )
    })?;
    poid_convert::verify_bundle(selection, &content)?;
    Ok(path)
}
