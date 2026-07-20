//! Per-user `.poid` association repair (Windows).
//!
//! The installers register the association machine-wide; this module writes
//! the same association under `HKCU\Software\Classes` at startup so that
//! (a) a dev build behaves like an installed one, and (b) a broken icon or
//! command — after a move, an update, or another app's land-grab — heals on
//! the next launch. Per-user only, idempotent, silent: no admin rights, no
//! prompts, and no writes when the values are already correct.
//!
//! The document icon is the *format's* icon, deliberately distinct from the
//! application icon (M07: "this is a document" vs "this is a program").

#[cfg(windows)]
mod windows_impl {
    use std::path::Path;
    use tauri::path::BaseDirectory;
    use tauri::{AppHandle, Manager};

    // The same ProgID the NSIS installer registers (FileAssociation.nsh keys
    // the file class by its display name), so the runtime repair and the
    // installer write the SAME registry keys instead of two competing ones.
    const PROG_ID: &str = "POID Document";
    const MEDIA_TYPE: &str = "application/vnd.poid+zip";

    /// Sets `value` as the default string of `key_path` (creating the key),
    /// returning true when the registry actually changed.
    fn set_default(root: &winreg::RegKey, key_path: &str, value: &str) -> std::io::Result<bool> {
        let (key, _) = root.create_subkey(key_path)?;
        let current: Result<String, _> = key.get_value("");
        if current.as_deref().ok() == Some(value) {
            return Ok(false);
        }
        key.set_value("", &value)?;
        Ok(true)
    }

    fn set_named(
        root: &winreg::RegKey,
        key_path: &str,
        name: &str,
        value: &str,
    ) -> std::io::Result<bool> {
        let (key, _) = root.create_subkey(key_path)?;
        let current: Result<String, _> = key.get_value(name);
        if current.as_deref().ok() == Some(value) {
            return Ok(false);
        }
        key.set_value(name, &value)?;
        Ok(true)
    }

    fn register(app: &AppHandle) -> std::io::Result<bool> {
        let exe = std::env::current_exe()?;
        let exe = exe.to_string_lossy();

        // The bundled document icon; falls back to the exe's own icon if the
        // resource is missing (dev builds before the first resource copy).
        let icon = app
            .path()
            .resolve("icons/poid-document.ico", BaseDirectory::Resource)
            .ok()
            .filter(|p: &std::path::PathBuf| Path::new(p).exists())
            // The resolver returns an extended-length path (`\\?\C:\...`);
            // shell icon handlers predate that syntax, so strip the prefix.
            .map(|p| {
                let s = p.to_string_lossy().into_owned();
                s.strip_prefix("\\\\?\\").map(str::to_owned).unwrap_or(s)
            })
            .unwrap_or_else(|| format!("{exe},0"));

        let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
        let classes = hkcu.create_subkey("Software\\Classes")?.0;

        let mut changed = false;
        changed |= set_default(&classes, ".poid", PROG_ID)?;
        changed |= set_named(&classes, ".poid", "Content Type", MEDIA_TYPE)?;
        changed |= set_default(&classes, PROG_ID, "POID Document")?;
        changed |= set_default(&classes, &format!("{PROG_ID}\\DefaultIcon"), &icon)?;
        changed |= set_default(
            &classes,
            &format!("{PROG_ID}\\shell\\open\\command"),
            &format!("\"{exe}\" --open \"%1\""),
        )?;
        Ok(changed)
    }

    /// Ensures the per-user association exists and points at this binary.
    pub fn ensure(app: &AppHandle) {
        match register(app) {
            Ok(true) => {
                // Explorer caches file-type icons; tell it the world changed.
                // SAFETY: SHChangeNotify with SHCNE_ASSOCCHANGED takes no
                // pointers (both item arguments are unused and null).
                unsafe {
                    windows_sys::Win32::UI::Shell::SHChangeNotify(
                        windows_sys::Win32::UI::Shell::SHCNE_ASSOCCHANGED as i32,
                        windows_sys::Win32::UI::Shell::SHCNF_IDLIST,
                        std::ptr::null(),
                        std::ptr::null(),
                    );
                }
            }
            Ok(false) => {}
            Err(e) => eprintln!("poid-studio: file association repair failed: {e}"),
        }
    }
}

#[cfg(windows)]
pub use windows_impl::ensure;

/// No-op outside Windows: macOS reads `Info.plist`, Linux reads the
/// installed `.desktop`/MIME files — neither is repairable from user code.
#[cfg(not(windows))]
pub fn ensure(_app: &tauri::AppHandle) {}
