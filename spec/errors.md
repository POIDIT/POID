# POID Error Code Registry

**Status:** Normative. These codes are part of the public conformance
contract (SPEC §14). Once published, a code's number and meaning never
change; new codes may be appended in free slots.

A conformant implementation **MUST** reject nonconformant containers and
**MUST** be able to report the registry code of the rejection. The registry
code is deliberately coarser than an implementation's internal diagnostics:
several fine-grained causes may map to one registry code. The right-hand
column lists the diagnostic codes of the reference implementation
(`poid-core`, `PoidError::code`) that map to each entry — informative, not
normative.

| Code | Meaning | Spec | Reference diagnostics |
|---|---|---|---|
| POID-001 | Missing or malformed `mimetype` entry | §2.1 | `mimetype-missing`, `mimetype-mismatch` |
| POID-002 | `mimetype` not first, not STORED, streamed, or carrying an extra field | §2.1 | `mimetype-not-first`, `mimetype-not-stored` |
| POID-003 | Not a ZIP archive | §2.1 | `not-zip` |
| POID-004 | Archive entry cannot be decoded | §2.1 | `corrupt-entry` |
| POID-010 | `manifest.json` missing | §2.2 | `manifest-missing` |
| POID-011 | `manifest.json` is not valid JSON | §3 | `manifest-syntax` |
| POID-012 | Manifest fails schema/rule validation | §3.1 | `manifest-unsupported-version`, `manifest-missing-field`, `manifest-invalid-id`, `manifest-invalid-version`, `manifest-invalid-path`, `manifest-invalid-profile`, `manifest-connection-requires`, `manifest-invalid-digest`, `manifest-invalid-number` |
| POID-020 | Prohibited content: native executable or library | §2.3 | `native-code` |
| POID-021 | Prohibited content: symbolic/hard link or special file | §2.3 | `link` |
| POID-022 | Prohibited content: path traversal or malformed path | §2.3 | `path-traversal`, `invalid-path` |
| POID-023 | Decompression ratio limit exceeded | §2.3 | `zip-bomb-ratio` |
| POID-024 | Absolute uncompressed size limit exceeded | §2.3 | `zip-bomb-size` |
| POID-025 | Unsupported compression method | §2.1 | `unsupported-compression` |
| POID-026 | Nested archives exceed the allowed depth | §2.3 | `nested-too-deep` |
| POID-027 | Duplicate entry paths (case-insensitive) | §2.3 | `duplicate-path` |
| POID-028 | Entry count limit exceeded | §2.3 | `too-many-entries` |
| POID-030 | Manifest `entry` not found in the container | §3.1 | `entry-missing` |
| POID-031 | Integrity digest mismatch | §3.3 | `integrity-mismatch` |
| POID-032 | Required `app/` tree missing | §2.2 | `app-tree-missing` |
| POID-040 | `type: data` container contains code trees | §4.2 | `data-container-with-code` |
| POID-041 | `type: workspace` container has no nested POIDs under `apps/` | §4.3 | `workspace-apps-missing` |
| POID-050 | Signature present but does not verify | §9.3.2 | `signature-invalid` |
| POID-051 | Signature file malformed | §9.3.1 | `signature-malformed` |

## Notes

- **Ordering:** implementations may detect several violations in one file;
  the reported code depends on check order. The conformance fixtures are
  constructed so exactly one violation is present per fixture, which makes
  the expected code unambiguous.
- **Missing vs misplaced mimetype:** an archive with no `mimetype` entry at
  all is POID-001; an archive that has one in the wrong place or wrong form
  is POID-002.
- **Signatures are optional:** the absence of `signature/` is not an error.
  POID-050/051 apply only when a signature is present.
