/**
 * Builds the validation core for the Web Reader:
 *   1. `poid-wasm` → wasm32-unknown-unknown (release)
 *   2. wasm-bindgen (via the in-repo driver) → src/wasm/ (gitignored)
 *
 * The driver (`tools/wasm-bindgen-driver`) is compiled from source, so the
 * only prerequisites are a Rust toolchain and the wasm32 target:
 *   rustup target add wasm32-unknown-unknown
 */

import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..");

function run(cmd, args) {
  console.log(`> ${cmd} ${args.join(" ")}`);
  const result = spawnSync(cmd, args, { cwd: repoRoot, stdio: "inherit", shell: false });
  if (result.error?.code === "ENOENT") {
    console.error(
      `\nbuild:wasm needs \`${cmd}\` on PATH. Install Rust (https://rustup.rs) and run:\n` +
        "  rustup target add wasm32-unknown-unknown\n",
    );
    process.exit(1);
  }
  if (result.status !== 0) process.exit(result.status ?? 1);
}

run("cargo", ["build", "-p", "poid-wasm", "--target", "wasm32-unknown-unknown", "--release"]);

const wasm = join(repoRoot, "target", "wasm32-unknown-unknown", "release", "poid_wasm.wasm");
if (!existsSync(wasm)) {
  console.error(`expected ${wasm} after the cargo build`);
  process.exit(1);
}

run("cargo", [
  "run",
  "--manifest-path",
  join("tools", "wasm-bindgen-driver", "Cargo.toml"),
  "--release",
  "--",
  wasm,
  join("packages", "poid-web", "src", "wasm"),
]);
