# POID trademark policy

The **POID name and logo are trademarks of the maintainer** of the POID project.

The rest of the project is deliberately open:

- **Code** — licensed under the [Apache License 2.0](LICENSE).
- **Specification** — licensed under [CC-BY-4.0](spec/LICENSE).

## What this means in practice

- **Anyone may implement the POID format**, commercially or otherwise, without
  asking for permission. The spec license guarantees this.
- **Only conformant implementations may use the POID name.** An implementation
  is conformant when it passes the POID Conformance Suite (`spec/conformance/`)
  and enforces the normative security requirements of the spec (SPEC §14) —
  in particular sandbox isolation (§5.2), credential isolation (§7.1) and
  user consent (§9.1).
- A reader that does not enforce those requirements **must not** be described
  or marketed as a POID reader, and may not use the POID name or logo.

## Why

The name is the user-facing safety promise: "it is safe to open a POID from a
stranger." A non-conformant implementation wearing the name would break that
promise for everyone. Trademark is the only enforcement mechanism an open
format has — this is the same model used by other open standards.

For permission requests or questions, open an issue in this repository.
