# Privacy Policy

**MusicForge collects nothing. Ever.**

## Core promises

- **No telemetry.** The core, CLI, and GUI contain no analytics, usage statistics, or tracking of any kind.
- **No crash reporting.** Crashes are never uploaded; diagnostics stay on your machine.
- **No account.** No sign-up, no login, no device registration.
- **No background upload.** Your audio files, tags, covers, and lyrics never leave your machine through MusicForge.
- **No hidden network calls.** The core has zero network code paths — enforced by CI (dependency-tree scan, source scan, offline behavior test).
- **No update checks by default.** Update checking is off; if ever added, it will be manual-only.

## What stays on your machine

| Data | Where it lives | Who can read it |
|:--|:--|:--|
| Audio files & conversion output | Your directories | You |
| Scan index / hash cache / task history (`library.db`) | Local config directory only (never on network mounts) | You |
| Operation manifests & reports | `.musicforge/` inside your chosen output directory | You |
| Playlist files (`playlist export`) | The output directory you choose | You |
| Cover similarity hashes (`dedupe --covers`) | Computed in memory from embedded cover bytes; not persisted | You |

Library governance commands (`scan` / `clean` / `dedupe` / `organize` / `playlist` /
`genre`) operate purely on local files. Cover comparison reads **embedded cover bytes
only** and computes a 64-bit perceptual hash in memory — covers are never uploaded,
transmitted, or written anywhere by the analysis itself. The tool's own `.musicforge/`
state directory is excluded from scans so trashed copies never resurface as duplicates.

## Plugins (opt-in only)

Core MusicForge cannot access the network. Network capability exists **only inside explicitly installed and explicitly enabled plugins**:

- Every plugin ships a `plugin.json` permission manifest declaring `network`, which data fields are sent (`data_sent`) and — just as important — which are **never** sent (`data_not_sent`).
- The GUI shows the "will send / will never send" list **before** you enable a plugin, and logs every network-capable plugin's activity boundary.
- AI plugins may send **metadata only** (title/artist/album/duration/normalized filename). Audio binaries, cover binaries, and absolute paths are **never** sent — enforced at the plugin-host layer, verified by adversarial tests.
- Plugins are sandboxed by capability allowlists; a plugin that requests file deletion/move/upload permissions is **refused at load time**.

## Contact

Security or privacy concerns: see [SECURITY.md](SECURITY.md).
