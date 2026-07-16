//! Minimal wasm-bindgen driver.
//!
//! Equivalent to `wasm-bindgen --target web --out-dir <out> <input>`, built
//! from source in this repository so the toolchain does not depend on a
//! prebuilt `wasm-bindgen-cli` binary. (On the maintainer's Windows/GNU
//! toolchain the upstream CLI binary crashes at startup; the underlying
//! `wasm-bindgen-cli-support` library works fine.)

use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use wasm_bindgen_cli_support::Bindgen;

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let (Some(input), Some(out_dir), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: wasm-bindgen-driver <input.wasm> <out-dir>");
    };
    let input = PathBuf::from(input);
    let out_dir = PathBuf::from(out_dir);

    Bindgen::new()
        .input_path(&input)
        .web(true)
        .context("configuring web target")?
        .typescript(true)
        .remove_name_section(true)
        .remove_producers_section(true)
        // The library default omits the default module path (the CLI enables
        // it); the Web Reader relies on the no-argument `init()` resolving
        // `poid_wasm_bg.wasm` next to the module.
        .omit_default_module_path(false)
        .generate(&out_dir)
        .with_context(|| format!("generating bindings into {}", out_dir.display()))?;

    println!(
        "wasm-bindgen-driver: wrote bindings for {} into {}",
        input.display(),
        out_dir.display()
    );
    Ok(())
}
