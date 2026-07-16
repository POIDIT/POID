# poid-wasm

wasm-bindgen bindings over `poid-core` for the Web Reader (`packages/poid-web`).

The Web Reader must run **the same validation code as the desktop reader** —
this crate compiles `poid-core` to `wasm32-unknown-unknown` and exposes the
minimum surface the SPA needs: `openContainer` (reader-grade validation:
structure, integrity, signature), manifest access, file access, and the
mutations behind *Download updated file* (`setInstanceId`, `setData`,
`toBytes`).

## Build

```
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --locked --no-default-features --version <pinned>
cargo build -p poid-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir packages/poid-web/src/wasm \
    target/wasm32-unknown-unknown/release/poid_wasm.wasm
```

`packages/poid-web` wraps this in its `build:wasm` script — see its README.
The wasm-bindgen CLI version must match the `wasm-bindgen` crate version
pinned in `Cargo.toml` (the tool refuses to run otherwise).

## Test

Logic tests run on the host against the conformance fixtures:

```
cargo test -p poid-wasm
```

The bindings themselves are exercised in a real browser by the Playwright
tier in `packages/poid-web/e2e`.
