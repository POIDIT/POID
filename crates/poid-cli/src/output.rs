//! Command output: one result renders both as human text and as JSON.

use poid_core::PoidError;

/// A successful command result.
pub struct Report {
    /// Human-readable text for plain stdout.
    pub human: String,
    /// Structured value printed verbatim with `--json`.
    pub json: serde_json::Value,
    /// Exit with a failure code even though a full report was produced —
    /// used by `conformance` when fixtures fail.
    pub exit_failure: bool,
}

/// A failed command: stable machine-readable code + human message.
///
/// Container rejections carry the conformance codes from `poid-core`
/// (`PoidError::code`); CLI-level failures use their own codes (`io`,
/// `dir-not-empty`, `unresolved-dependency`, …).
pub struct CmdError {
    /// Stable machine-readable code.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Normative conformance registry code (`POID-xxx`, `spec/errors.md`),
    /// when the failure is a container rejection.
    pub poid_code: Option<String>,
}

/// Builds a CLI-level error.
pub fn err(code: &str, message: impl Into<String>) -> CmdError {
    CmdError {
        code: code.to_owned(),
        message: message.into(),
        poid_code: None,
    }
}

impl From<PoidError> for CmdError {
    fn from(e: PoidError) -> Self {
        CmdError {
            code: e.code().to_owned(),
            message: e.to_string(),
            poid_code: e.conformance_code().map(str::to_owned),
        }
    }
}

impl From<poid_convert::ConvertError> for CmdError {
    fn from(e: poid_convert::ConvertError) -> Self {
        CmdError {
            code: e.code,
            message: e.message,
            poid_code: None,
        }
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        CmdError {
            code: "io".to_owned(),
            message: e.to_string(),
            poid_code: None,
        }
    }
}

/// `12.3 KiB` style size formatting.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
