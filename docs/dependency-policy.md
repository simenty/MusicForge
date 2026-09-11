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

**Approved 2026-09-07 (P5.2):** `encoding_rs`（Apache-2.0 OR MIT）+ `chardetng`（MPL-2.0）—
CUE 文件编码检测（UTF-8/GBK/BIG5），Mozilla Firefox 同源组件；MPL-2.0 已加入 licenses 白名单
（文件级 copyleft，链接无传染）。**Approved 2026-09-07 (P4):** `image` with `default-features = false`, features =
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

## 5. GitHub Actions pinning (supply chain)

可变 tag 可被上游转移（或被账号劫持）→ 在 CI 内执行任意代码，而 CI 持有 Release/GHCR
写权限与签名相关 secret。分级策略：

- **第三方 action（会执行代码 / 接触 secret）：pin commit SHA**
  - `taiki-e/install-action`（下载并执行工具二进制）
  - `dtolnay/rust-toolchain`（安装工具链；**pin SHA 后必须显式 `with: toolchain: stable`**——
    该 action 原本以 `@stable` 这个 ref 推断版本，pin 后 ref 变成 SHA 便无法推断）
  - `goto-bus-stop/setup-zig`（下载 zig）
  - `docker/*-action`（接触 registry 凭据）
  - `softprops/action-gh-release`（持有 Release 写权限）
  - 写法：`uses: <owner>/<repo>@<40-hex-sha> # <version>`（行尾注释保留可读版本，便于人工更新）
- **官方 `actions/*`：保持 major tag**（`@v4`）——官方维护、不会转移 tag，保持 tag 可自动获得安全修复。

新增第三方 action 时必须按上述规则 pin；更新时用
`gh api repos/<owner>/<repo>/commits/<ref> --jq .sha` 取新 SHA 替换并同步注释。

