# Dependency Policy

Rule of thumb: **MusicForge core stays small, offline, and permissively licensed.**
Every dependency is a long-term liability; add them deliberately.

## 1. Hard bans (`musicforge-core`)

| Category | Examples | Why |
|:--|:--|:--|
| Network clients | reqwest, hyper, ureq, isahc, surf, tokio-tungstenite | zero-network brand (CI-enforced) |
| DB *server* client libs | postgres, mysql, redis, mongodb | core has no server dependency |
| GUI / windowing | winit, iced, egui | core is headless |
| GPL / nonfree licensed | libsqlite3-sys features pulling GPL variants, ffmpeg-sys with GPL | MIT project hygiene |
| Large async runtimes | tokio (in core) | core is sync + bounded threads |
| Platform-specific APIs | winreg, libc (in core) | portability; platform code lives in shells |

**Clarification (2026-09-05):** *embedded* SQLite via `rusqlite` (bundled) is **allowed** —
it is a renewable local cache, not a database *service*. The bundled C SQLite is
public-domain; `rusqlite` is MIT.

## 2. Allowlist (pre-approved for core)

serde, serde_json, thiserror, crc32fast, sha2, blake3, aes, base64, lofty, hound, claxon,
rusqlite (bundled), camino, smallvec, ignore, notify (runtime gating only — watcher is a
shell/CLI feature, not core-path networking), tempfile (dev-dependency).

**Approved 2026-09-07 (P4):** `image` with `default-features = false`, features =
`["jpeg", "png"]` — decode of *embedded* cover bytes only, for the similar-cover aHash
grouping. Justification: no smaller maintained crate covers both codecs; dual
MIT/Apache-2.0 license; decoders are pure Rust (no C toolchain). Also approved as a
**dev-dependency** of `musicforge-cli` (png) for test fixtures — production CLI never
links it.

Anything not on this list: open an RFC (see ROADMAP §10) with justification covering
license, size impact (binary growth >10% needs a note), and maintenance status.

## 3. CI enforcement

- `cargo deny check` — licenses (allow MIT / Apache-2.0 / BSD-2/3-Clause / ISC / Unicode-3.0 / Zlib / CC0-1.0), bans (network crates), sources.
- `cargo audit` — RustSec advisories.
- Source scan: `https?://`, `TcpStream::`, `UdpSocket::` must not appear in `musicforge-core/src`.
- `cargo tree -p musicforge-core` — no network crate may appear.

## 4. Shells (cli/gui/server)

Shells may add UI/platform dependencies (Tauri, notify, axum) but must still contain **zero
network clients** except `musicforge-server`'s local HTTP listener. Plugin processes are the
only components allowed to open outbound connections.
