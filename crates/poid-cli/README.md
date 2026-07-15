# poid-cli

`poid` — the command-line tool that makes the spec real. For *developers*;
end users never touch a terminal (the "no terminal" rule protects users, not
people who live in a shell).

```
poid init <dir> [--template web|python|survey]   # scaffold a project
poid pack <dir> -o app.poid                      # build + package
poid validate <file.poid>                        # conformance check, exit 0 = conformant
poid inspect <file.poid> [--json]                # manifest, tree, permissions, sizes
poid extract <file.poid> -o <dir>                # unpack
poid data <file.poid> --export data.json         # extract user data (SECURITY §6)
poid keygen -o key                               # generate an Ed25519 signing key
poid sign <file.poid> --key <key>                # sign in place (SPEC §9.3)
poid verify <file.poid>                          # verify integrity + signature
```

Every command accepts `--json` for scripting and for the MCP server. Errors
carry the stable conformance codes from `poid-core`.

## Building projects

- Plain HTML/CSS/JS with relative imports packs as-is — no build tool, no
  Node.js, no network.
- TypeScript/JSX projects are bundled with a **native esbuild sidecar**:
  place the esbuild binary next to `poid`, or set `POID_ESBUILD`. The version
  used is recorded in `runtime.toolchain`.
- Bare imports (`import x from "react"`) fail with the dependency named until
  the Standard Library lands (M06). **Nothing is ever fetched silently.**

```
cargo run -p poid-cli -- init demo
cargo run -p poid-cli -- pack demo -o demo.poid
cargo run -p poid-cli -- validate demo.poid
```
