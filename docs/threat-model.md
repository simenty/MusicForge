# Threat Model

Scope: MusicForge desktop (CLI/GUI), NAS server mode, and the plugin ecosystem.
Out of scope: the optional cloud repo (threat-modeled separately before v1.x).

## Quadrant 1 — Malicious / buggy plugins

| Threat | Vector | Mitigation | Residual |
|:--|:--|:--|:--|
| Plugin deletes/moves/uploads user files | over-privileged manifest | Load-time admission: delete/move/upload permission ⇒ **refuse to load**; plugins can only *suggest*; Core plan layer executes | Malicious suggestions shown to user (human is the gate) |
| Plugin exfiltrates metadata | `network=true` + `data_sent` | Permission manifest displayed before enable; host forwards only declared data fields; adversarial tests assert no `audio_binary`/`absolute_path` in transit | Endpoint choice is user-side (firewall can block) |
| Plugin crashes / hangs | process fault | Process isolation; timeout 10–30 s + kill; task marked failed, retryable; local features 100% unaffected | — |
| ABI break | `api_version` drift | semver-range negotiation handshake (`MF-PLUGIN-API-INCOMPATIBLE`) | — |

## Quadrant 2 — Malicious / corrupt audio files

| Threat | Vector | Mitigation | Residual |
|:--|:--|:--|:--|
| Parser memory corruption | crafted `.ncm`/audio headers | zero-panic core (CI-enforced), `read_exact` + bounded lengths, CRC verification; fuzzing on `header.rs` and future parsers (scheduled, D19) | Fuzz coverage grows with formats |
| Zip-slip via crafted archive names | malicious filenames inside containers | illegal-character sanitizer + reserved-device-name handling (fixture-covered) | — |
| Decompression bombs | huge declared lengths | upper-bound validation on all length fields (streaming decode) | — |

## Quadrant 3 — Path injection / filesystem escapes

| Threat | Vector | Mitigation | Residual |
|:--|:--|:--|:--|
| Template-driven path escape | `../` inside metadata or template | naming-template sanitization (illegal chars, control chars, trailing dots); `.`/`..` segments sanitize to `_`; golden tests | — |
| Output path escape (plugins) | `output_path`/`work_dir` outside allowed roots | host restricts to declared roots; `..`, absolute paths, symlink escapes rejected | — |
| Long-path / reserved names (Windows) | `CON`, `>260` chars | `\\?\` prefixing in `safety/path.rs`; `illegal_name` fixture regression | — |
| GUI dedupe escape | crafted sacrifice path via IPC (`dedupe_apply`) | every path canonicalized and asserted to stay **inside** the library directory; outside paths explicitly rejected and recorded (contract test `dedupe_commands_contract`) | — |
| Tool-state self-consumption | scanner/organizer treating `.musicforge/` (trash, rollback manifests) as library content — phantom duplicate groups, restore breakage | walker prunes any `.musicforge` component (`musicforge_convention_dir_is_invisible_to_scan`); state db lives outside the library by default (`X16`) | — |
| In-place tag overwrite | `genre --apply` clobbering user metadata | `FillMissingOnly` default (existing genre never written); `--replace-all` escalates to high-risk grading (`--yes` required, `MF-OP-NEEDS-YES`) | Tag edits are not trash-restorable — dry-run default is the gate |

## Quadrant 4 — Template injection

| Threat | Vector | Mitigation | Residual |
|:--|:--|:--|:--|
| Code/config injection via template strings | user-supplied `{placeholder}` templates | template engine is a closed substitution DSL (no expression eval); unknown placeholders fall back literally; proptest coverage planned (D19) | — |

## Reporting

See [SECURITY.md](../SECURITY.md). Please include reproduction fixtures — synthetic
malformed files only; never attach copyrighted material.
