//! Launch-argument parsing: is this a Reader launch or a Studio launch?
//!
//! The decision the whole shell hangs on (ARCHITECTURE §2): a launch that
//! carries a file opens **only** a Reader window; a bare launch opens the
//! Studio hub. The same parser serves the first process and the forwarded
//! argv of every later one (single-instance), so the two can never disagree.

use std::path::PathBuf;

/// Extracts the file to open from a launch argument list (without `argv[0]`).
///
/// Accepts `--open <path>` (the form our file association registers) and a
/// bare `*.poid` positional (drag-onto-exe, `Open with…`, and shells that
/// strip flags). Returns `None` for a bare launch — the Studio hub.
pub fn open_request<I: IntoIterator<Item = String>>(args: I) -> Option<PathBuf> {
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--open" {
            return iter.next().map(PathBuf::from);
        }
        // Webview child processes re-enter main with their own flags; only a
        // positional that looks like a document may claim the launch.
        if !arg.starts_with('-') && arg.to_ascii_lowercase().ends_with(".poid") {
            return Some(PathBuf::from(arg));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::open_request;

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn bare_launch_is_the_hub() {
        assert_eq!(open_request(v(&[])), None);
    }

    #[test]
    fn open_flag_is_a_reader_launch() {
        assert_eq!(
            open_request(v(&["--open", "C:\\docs\\kanban.poid"])),
            Some("C:\\docs\\kanban.poid".into())
        );
    }

    #[test]
    fn open_flag_without_a_path_is_the_hub() {
        assert_eq!(open_request(v(&["--open"])), None);
    }

    #[test]
    fn bare_poid_positional_is_a_reader_launch() {
        assert_eq!(
            open_request(v(&["kanban.POID"])),
            Some("kanban.POID".into())
        );
    }

    #[test]
    fn open_flag_wins_over_a_later_positional() {
        assert_eq!(
            open_request(v(&["--open", "a.poid", "b.poid"])),
            Some("a.poid".into())
        );
    }

    #[test]
    fn unrelated_flags_and_files_are_ignored() {
        assert_eq!(open_request(v(&["--flag", "notes.txt"])), None);
    }

    #[test]
    fn a_flag_valued_path_is_not_mistaken_for_a_positional() {
        // `--open` consumes the next arg even if a shell mangled it; a flag
        // itself must never be treated as a document path.
        assert_eq!(
            open_request(v(&["--verbose", "--open", "x.poid"])),
            Some("x.poid".into())
        );
    }
}
