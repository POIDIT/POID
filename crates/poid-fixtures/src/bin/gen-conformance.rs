//! Writes the conformance suite to a directory (default: `spec/conformance`).
//!
//! Output is deterministic; CI regenerates the suite and diffs it against the
//! committed files to prove they match this generator.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(count) => {
            println!("wrote {count} fixtures");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("gen-conformance: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<usize, Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spec/conformance".to_owned());
    let fixtures = poid_fixtures::conformance_fixtures()?;
    let mut count = 0usize;
    for fixture in &fixtures {
        let dir = Path::new(&out).join(if fixture.valid { "valid" } else { "invalid" });
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("{}.poid", fixture.name)), &fixture.bytes)?;
        let mut expected = serde_json::to_string_pretty(&fixture.expected_json())?;
        expected.push('\n');
        std::fs::write(
            dir.join(format!("{}.expected.json", fixture.name)),
            expected,
        )?;
        count += 1;
    }
    Ok(count)
}
