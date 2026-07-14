//! `poid` — the POID command-line tool.
//!
//! `anyhow` is used at this top level only; library crates use `thiserror`
//! (see `CONVENTIONS.md`).

use anyhow::Result;

fn main() -> Result<()> {
    println!(
        "poid {} (spec {})",
        env!("CARGO_PKG_VERSION"),
        poid_core::SPEC_VERSION
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn links_against_core() {
        assert_eq!(poid_core::SPEC_VERSION, "1.0");
    }
}
