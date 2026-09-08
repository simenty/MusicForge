# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added（P7：v0.8.0 多平台交付 + GUI i18n，发布另行授权）
- **多平台交付（ROADMAP P7 / D26）**：CI build 矩阵新增 `x86_64-unknown-linux-musl`
  （musl 静态链接，rusqlite bundled 随 musl-tools 编译）；unix 产物统一打包
  `musicforge-v{ver}-{target}.tar.gz` + 独立 `.sha256`；
  **musl 裸容器冒烟**（alpine 零 glibc 环境 `--version`/`--help` 跑通——P7 验收）。
- **统一发布管线**：release-installer 重构为「构建 ×3（windows NSIS+zip /
  linux-musl / macos-aarch64）→ assemble job 统一生成全量 `SHA256SUMS.txt`
  一次上传」（GH Release 唯一事实源，消除分 job 各自建 Release 的资产分裂）。
- **平台差异壳层闸**：CI 新增 grep 断言——core/cli/plugin-api/plugin-host
  业务源码禁平台 `cfg` attribute（target_os/windows/unix；测试模块不受限），
  "cfg 只在壳层" 验收落地为永久防退化闸。
- **GUI i18n 中英（ROADMAP P7）**：零依赖 i18n 框架（`i18n.tsx` Context +
  双语字典 `i18n/zh.ts`/`en.ts`；`en: typeof zh` 类型强制两语言形状一致——
  缺键/多键编译期报错）；App/Dedupe/Scan/Plugin 四面板全文案接入
  （插值文案用函数值）；标题栏语言切换（跟随系统默认 + localStorage 持久化
  `mf.lang`）；顺手修正标题栏版本徽标 v0.1.0 → v0.7.0 漂移。

### Fixed（稳定性审计第二轮：P6b 新增代码面）
- **B5（严重）**：convert 桥接批量——不同目录下的同名源（`track01.kwm` × N）
  共享同一暂存目录，插件迁移撞名 `MF-OUTPUT-EXISTS` → 批次中断。修复：
  暂存子目录按输入序号隔离（`staging/<task>/<idx>/`）。
- **B9（严重·独立仓）**：format 插件服务环清单名取自未设置的环境变量 →
  默认 `"format-plugin"` ≠ ACK 记录的真名 → ACK 闸永远失败。修复：
  `serve(name, handler)` 显式接名（与 plugin.json 逐字节一致），进程级
  测试钉住。已在 `musicforge-format-plugins` 同步。
- **B7（一般·独立仓）**：HTTP 错误摘要按字节切片，多字节字符（CJK 错误体）
  中间截断 → 插件进程 panic。修复：按字符边界截断（`utf8_truncate`）。
- **B6（一般）**：CLI `plugins enable` 与 GUI 校验语义不一致（不 trim、
  静默保留重复名）。修复：对齐 GUI——trim、拒空名、重复名显式拒绝且
  不改写既有配置。
- **B8（一般·独立仓）**：隔离区 task 目录毫秒+pid 命名在同进程连续失败时
  撞名 → 改名失败掩盖原始错误。修复：纳秒精度。

### Added（P6b.1：格式迁移插件 Host 桥接框架）
- **plugin-api**：`FORMAT_MIGRATE` 方法常量 + `FormatMigrateParams` /
  `FormatMigrateResult` / `MigrateVerification` / `MigrateAudit` 强类型契约；
  `PluginManifest.ack_required` 字段（X35 规则：缺键默认 false，向后兼容）。
- **config**：`plugins.acked`（高风险插件 ACK 确认记录；缺段默认空，向后兼容）。
- **cli**：`plugins list / enable / disable-all / acknowledge` 子命令（config 持久化）
  + `format migrate` 命令——ACK 闸先行（未确认 → `MF-PLUGIN-ACK-REQUIRED`），
  启用且白名单目录存在插件方可调用；默认构建响亮报 `MF-PLUGIN-NOT-FOUND`
  （绝不静默装作执行过）。
- **回归测试 ×8**：ack_gate / acknowledge 幂等 / status 装配 / set_enabled
  整表覆盖 / 默认构建响亮降级 / config acked roundtrip / manifest ack_required
  缺键兼容 / format.migrate 类型 roundtrip。
- 独立仓同步：`musicforge-plugins` ×3 清单 + `musicforge-format-plugins`
  `kwm-migration` 清单/服务环声明 `ack_required`（防漂移测试覆盖）。

### Added（P6b.2/P6b.3：extensions 能力声明 + convert 管线自动分派）
- **plugin-api**：`PluginManifest.extensions` 能力声明（X35 缺键兼容）——
  加密容器无明文魔数，Host 按扩展名探测（逐格式兼容性申报）。
- **cli（plugin-host feature）**：`PluginFormatAdapter` 桥接组件——
  `probe` 按扩展名（置信度 0.6，低于内置 magic）/ `decode` 委托插件迁移 →
  产物读回构造 `DecodedAudio`；`common_work_root`（源+输出最小公共祖先，
  跨盘显式报错）；`registry_with_plugins()` 装配（门槛三连：启用+ACK+exe，
  缺一静默缺席=降级铁律）。
- **convert 自动分派（staging 设计）**：`plan_one_with` 注册表感知规划——
  非内置适配器在规划期迁移到 `.musicforge/staging/<task>/` 暂存区，
  `execute_one` 桥接分支改名入位 + sidecar（覆盖重转走 B1 备份/回滚语义）；
  dry-run 暂存树计划后整体清理；源文件全程不动。
- **回归测试**：feature 门控 3 项（桥接规划/执行/覆盖重转）+ 公开 API
  `common_work_root` 收敛断言；注册表装配门槛三连测试。
- 独立仓同步：`musicforge-plugins`（extensions 空声明）+
  `musicforge-format-plugins`（kwm 申报 `.kwm`）。

## [0.7.0] - 2026-09-08

### Added（P6a：插件系统 + AI，★曲库管家完整形态）
- **D8 feature 接线**：`musicforge-cli` 新增 `plugin-host` optional feature
  （default off，默认构建零插件符号——CI `cargo tree` 断言扩展至 CLI）；
  `musicforge-gui` 新增 `plugin-host` feature（发行版构建默认携带，
  release 命令含 `--features musicforge-gui/custom-protocol,musicforge-gui/plugin-host`）。
- **冻结方法集落地**：`musicforge-plugin-api` 新增 `methods` 常量模块
  （`plugin.manifest/health/shutdown` + `ai.identify_track/generate_filename_regex/
  review_duplicate_group` + `lyrics.verify` + `cover.search/generate`）与全部
  强类型 Params/Result（最小请求模型——类型层面无 `absolute_path`/`audio_bytes`/
  `cover_bytes`）；`musicforge-plugin-host` 新增 `call_typed<T>()` 与
  `call_limited()`（超时夹取）；mock 插件实现全部方法；e2e 6→17 用例。
- **限制三件套**：host `limits` 模块（单请求超时窗口 10–30s 夹取；插件进程
  并发上限 2，RAII 槽位闸）；降级完整性 = D8 默认构建无 host 符号 +
  `musicforge-cli/tests/p6a_degradation.rs` 五域（扫描/清洗/去重/归档/转换）
  零插件环境全链路集成测试。
- **D24 质量画像**：`dedupe` 新增 `QualityProfile` + 三默认画像
  （fidelity=原固定权重=默认 / archival / metadata）+ `*_with` 评分/reason
  API + `dedupe_scan_with_profile`；默认路径行为与 v0.6.0 完全一致
  （reason 可复算铁律不变）。
- **D22 资产策略 + X36 provenance**：core 新模块 `assets`——`Provenance`
  （embedded/filename/ai/manual/provider，第一天含 `ai`）、封面/歌词来源链、
  `asset_mode` 三模式（默认 embedded-only）、质量门限 500px、
  「来源优先级 + 质量门限」双重判定（已有达标内嵌绝不发起外部请求——
  验收断言钉死）、存量 `.lrc`/附属图吸入规则。
- **X36 config**：core 新模块 `config`——`config.json`（schema_version=1，
  含 `plugins.enabled` 空段；未知键忽略、高版本显式拒绝、原子写、损坏报
  `MF-CONFIG-INVALID`——已登记 `docs/result-codes.md`）；`NcmError::Config`。
- **X13 置信度并入 Plan**：CLI `Suggestion` 载体（provider/method/confidence/
  fields/reason）+ `PlannedItem.suggestion`（预览）+ `ManifestItem.suggestion`
  （NDJSON 行级可选键：无建议行与 v0.2.0–v0.6.0 字节兼容；schema_version 不
  bump——X35 兼容规则测试钉死）。
- **X37 GUI 功能态**：「AI 与插件」面板从占位态解锁——运行时可用性、白名单
  目录已装插件清单、逐插件启用开关（写 X36 config）；页面零请求，
  PLUGIN_POLICY 链接保持纯文本；IPC 契约测试覆盖。

### Fixed（P6a 收官回归）
- **organize 子目录模板归位判定**：`{album}/{title}` 等含子目录的模板下，
  位于 target_root 根级、渲染文件名恰好相同的文件曾被误判 `AlreadyInPlace`
  （归位判定只比文件名、不比目录），永远不归入子文件夹；且子目录内 suffix
  落位文件（`a (2).wav`）二次规划会被续编号到 `(3)`（无限膨胀）。
  修复：归位判定目录口径从「target_root 根」收紧为「渲染目标所在目录」——
  平铺模板行为不变（既有幂等回归测试保持绿），新增两条子目录模板回归测试。
- **atomic_rename 覆盖重转双丢失窗口（B1，审计）**：先删旧产物再改名，
  rename 失败（os error 5 / Defender 锁）时新旧产物同时丢失。修复：旧产物
  先同目录改名备份，新产物就位成功才删备份，失败回滚归位——目标路径在任何
  失败路径下都保持有效内容；目录占位仍显式失败（QA 拍板语义不变）。
- **clean --restore 还原碰撞中断（B2，审计）**：还原目标被同名文件占用时
  整体还原中途失败。修复：邻位 `(restored[-n])` 落盘（绝不覆盖占用者）；
  回收站内已缺失的条目跳过不中断。
- **D20 兼容区间隐式放行（B4，审计）**：`api_compatible` 对无法解析的
  host_range（如完整 semver / 乱码）曾退化为放行一切。修复：未解析出合法
  约束或出现未知形态 → 显式不兼容（拒绝优于放行）。

### Security（审计复核结论）
- 零网络四层闸 / 路径遍历三防线（GUI 改选 canonical 校验、遍历不跟随符号链接、
  插件路径守卫）/ 无注入面（rusqlite 参数化）复核通过；本轮无新增安全修复项
  （B4 为兼容边界收紧，非漏洞）。详见 `docs/audit-2026-09-08.md`。

## [0.6.0] - 2026-09-07

### Added
- **Lossy export presets (`transcode --format mp3|aac|opus`)**: MP3 320
  (libmp3lame) / AAC 256 (native aac → .m4a) / Opus 160 (libopus) via FFmpeg
  sidecar (D1: never bundled — five-level detection `--ffmpeg-path` → exe dir
  → PATH → common dirs → `MF-FFMPEG-MISSING` with install guidance; every
  candidate verified via `ffmpeg -version`); built-in read-back verification
  per output (container magic per preset + duration delta <1s parsed from
  `ffmpeg -i` stderr — no ffprobe dependency; failing output deleted).
- **Upgrade interception**: lossy source entering the lossless pipeline raises
  `MF-LOSSY-TO-LOSSLESS` before any decode; `--i-know-lossy-to-lossless`
  bypasses explicitly (ffmpeg flac/pcm re-encode).
- **APE / WavPack / TAK whole-track splitting (D21)**: `split` accepts
  sidecar-format images via ffmpeg decode to temporary 24-bit WAV (value-space
  lossless; temp deleted after slicing); output format defaults to FLAC for
  these sources (`--format flac|wav` override); WavPack path covered by
  end-to-end test (ffmpeg has a wv encoder; APE/TAK share the same decode
  path — ffmpeg ships decoders only for them).
- **Template compatibility aliases (D25)**: beets `$artist` and Music Tag Web
  `%artist%` styles normalize to the native `{artist}` syntax (single alias
  table, not a dual-syntax engine; `$artists`-style longer names never
  clobbered); organize/convert templates accept all three spellings.
- **Facade migration guide (D9)**: `docs/migration-facades.md` — full
  old→new path mapping for the P1b facades, dual error-code namespaces, and
  the honest preconditions for `#[deprecated]` markers (QA-protected tests
  must migrate first).

### Changed
- deps: +encoding_rs, +chardetng (MPL-2.0, allowlisted); CLI ~2.7MB.

## [0.5.0] - 2026-09-07

### Added
- **Pure-Rust lossless transcode (`transcode`)**: WAV↔FLAC sample-exact conversion
  (hound read/write + claxon decode + flacenc encode — all pure Rust, Apache/MIT);
  magic-first probing (RIFF..WAVE / fLaC, extension ignored); interleaved-i32 PCM
  pipeline; **built-in read-back verification** (the output must decode sample-exact
  or it is deleted and `MF-LOSSLESS-FAILED` raised — unverified output never
  survives); float WAV and >25-bit FLAC encode rejected explicitly with source
  untouched; same-format inputs skipped; `(n)` collision suffix; human + JSON.
- **CUE sheet parsing & whole-track splitting (`split`, D21)**: encoding detection
  (BOM → strict UTF-8 → chardetng GBK/BIG5, tolerant fallback); tolerant dialect
  parser (unknown commands preserved-and-ignored, R21; multi-FILE rejected
  explicitly); INDEX 01 boundaries (MM:SS:FF @75fps) with per-track duration
  verification **before write** (vs INDEX delta <1s; failing tracks not written);
  tag flow via lofty (track TITLE/ARTIST + album fallback, ALBUM, TRACKNUMBER,
  REM DATE) and whole-track embedded cover written to every split track (WAV via
  Id3v2 chunk — RiffInfo cannot hold pictures); naming `NN Title` through the
  shared sanitizer.
- **Synthetic waveform generator (D19)**: sine/silence at any spec written as
  WAV/FLAC — zero-copyright test fixtures powering all transcode/CUE tests.
- **GUI**: 「AI 与插件」placeholder panel (X37) — zero-request page stating that
  all local features work without plugins; v0.7.0 reads as unlock, not a new
  stranger entry.

### Changed
- deps: +hound, +claxon, +flacenc (pure Rust), +encoding_rs, +chardetng (MPL-2.0,
  already allowlisted); CLI ~2.6MB (growth within budget).

## [0.4.0] - 2026-09-07

### Added
- **Library dedupe (`dedupe`)**: exact-content duplicate grouping (size pre-filter →
  streaming sha256 buckets, reusing the D17 hash cache); explainable keep-score
  interpreter (lossless +40 / sample-rate +8 / bit-depth +8 / tags +10 / cover +5 /
  verified sidecar +20; ties keep the lexicographically smallest path — recomputable
  reasons printed per sacrifice); same-name candidates reported (opt-in execution via
  `--include-same-name`); sacrifices always go to the trash with a rollback manifest.
- **Similar-cover grouping (`dedupe --covers`)**: hand-written 8×8 grayscale aHash over
  embedded cover bytes only; union-find clustering at Hamming ≤ 8/64; report-only
  (cover replacement ships with v0.7.0 AI review).
- **Library organize (`organize`)**: template-driven placement sharing the converter's
  rendering semantics (sanitize / reserved names / dual length caps / fallbacks);
  metadata from embedded tags; conflict strategies `skip | suffix | overwrite-never`
  (default `skip`; nothing is ever overwritten); idempotent (suffix-placed files are
  recognized as in-place on re-plans); rollback manifest restorable via `clean --restore`.
- **Playlist module (`playlist export` / `playlist import`)**: UTF-8 M3U8 export grouped
  by artist/album (EXTINF title/duration, playlist-relative paths); import repairs
  broken paths by same-name matching with ±1s duration disambiguation; unresolvable
  entries preserved as `# FAIL` comments (audit never loses rows).
- **Genre writer (`genre`)**: filename style codes `[Y23-S01-E01-C01-C02-V00]` →
  year/style/mood/scenes/version (codebook JSON translation with raw-code fallback);
  `FillMissingOnly` by default — existing genres are never overwritten;
  `--replace-all` requires `--yes` (high-risk grading).
- **GUI**: duplicate-groups view (in-group comparison, suggested-keep highlight,
  manual re-pick via radio, trash execution with confirmation and server-side
  path-escape validation) and library-scan panel.
- D17 incremental hashing wired into scanning: second scan of the same library
  re-hashes nothing (real-library evidence: 1340 files → 161 hashed once, then 1.0s
  all-cache-hits).

### Fixed
- Keep selection on exact-score ties prefers filenames **without** `(N)`
  duplicate-artifact markers (real libraries contain `song.flac` + `song (2).flac`
  pairs from repeated downloads; the clean-named original is now the one kept).
- Scanner now prunes the `.musicforge/` convention directory — trashed copies no
  longer re-enter scans as phantom duplicate groups, and organize can no longer
  relocate pending-restore files (surfaced by real-library validation).
- Organize suffix strategy is idempotent: `name (2).ext` placements are no longer
  re-suffixed on every plan (unbounded growth bug, surfaced by real-library validation).

## [0.3.0] - 2026-09-07

> 随 v0.4.0 同树发布（未单独打 tag）：0.4.0 包含两处真机实测修复，单独切 0.3.0 代码态会带上已修复的缺陷。

### Added
- **Library scan (`scan`)**: read-only recursive classification (audio/lyrics/cover/
  junk/other), empty-directory collection, orphan lyric/cover detection, long-path /
  illegal-character / mojibake-replacement flags; human and `--json` reports.
- **Library clean (`clean`)**: 9 rule cards (`MF-CLEAN-001`–`009`, rules-as-data shared
  with the GUI); destructive grading (dry-run by default, `--apply` to execute);
  everything goes to the trash (relative structure preserved, `rollback.jsonl`) —
  never deleted; `--rules` filter and `--restore`.
- CLI subcommands (`scan` / `clean`) alongside legacy top-level converter arguments
  (subcommand/args mutual exclusion, legacy behavior unchanged).

## [0.2.0] - 2026-09-06

### Added
- **Safety task layer**: per-task NDJSON manifest (`schema_version=1`) with one item
  per file (result, `MF-*` code, target sha256, adapter id); `--dry-run` (plan only,
  zero writes); `--resume <manifest>` (skip completed; reserved target names prevent
  overwrite-on-resume); atomic writes (temp + rename) with startup cleanup of stale
  temp files.
- **State layer** (`--state-db`, D16): SQLite `library.db` (files index / hash cache /
  task history / ack records). Renewable cache only — the filesystem and manifest
  remain the source of truth. Local config dir only; network mounts rejected.
- **Safety grading**: destructive commands default to dry-run (`--apply` to execute,
  `--yes` for high-risk); stable codes `MF-OP-CONFLICT` / `MF-OP-NEEDS-YES`.
- **Strangler refactor (P1)**: `FormatAdapter` / `NcmAdapter` / `FormatRegistry` with
  equivalence tests; CLI routes format detection through the registry; legacy paths
  kept via facades (D9); `MF-*` error-code namespace added (legacy `NCM-*` retained).
- **GUI**: dry-run toggle (persisted), planned badge, plan preview panel; full-
  resolution icon set (1024px source art in docs/assets).

### Changed
- `MF-*` error-code namespace introduced alongside legacy `NCM-*` codes
  (see docs/result-codes.md); contract tests pin the mapping.
- Whole-tree rustfmt normalization; MSRV pinned via rust-toolchain.toml.

### Docs
- ROADMAP (frozen baseline), PRIVACY, PLUGIN_POLICY, TRADEMARK, SECURITY(SLA),
  docs/{architecture, threat-model, dependency-policy, result-codes, refactor-invariants}.

## [0.1.1] - 2026-09-06

### Fixed
- **Cross-platform filename overflow**: naming-template segment truncation now enforces a
  byte budget (≤200 UTF-8 bytes) in addition to the 100-char cap. Linux/macOS limit filename
  components to 255 **bytes**; a 100-char CJK title (300 B) or emoji title (400 B) previously
  failed with `ENAMETOOLONG` at write time (surfaced by new ubuntu CI runner).

### Added
- CI: four-layer supply/offline gates (cargo-deny, cargo-audit, source-regex, feature
  assertion, network-blocked golden test); three-platform test matrix made green.
- Governance docs: ROADMAP, PRIVACY, PLUGIN_POLICY, TRADEMARK, SECURITY(SLA),
  docs/{architecture, threat-model, dependency-policy}; issue/PR templates; branch protection.

### Changed
- Full-resolution icon set generated via `tauri icon` (1024px source art in docs/assets/).

### Planned (see ROADMAP.md)

- **Cross-platform filename overflow**: naming-template segment truncation now enforces a
  byte budget (≤200 UTF-8 bytes) in addition to the 100-char cap. Linux/macOS limit filename
  components to 255 **bytes**; a 100-char CJK title (300 B) or emoji title (400 B) previously
  failed with `ENAMETOOLONG` at write time (surfaced by new ubuntu CI runner).

### Planned (see ROADMAP.md)

- v0.2.0 — Safety task layer: Plan / Manifest / Dry-run / Trash / Rollback / Resume + state db.
- v0.3.0 — Library scan & clean. …(full line at ROADMAP.md §6)

## [0.1.0] - 2026-09-05

Initial public release (renamed from the private prototype "Shelf").

### Added

- `musicforge-core`: offline, streaming, CRC-verified `.ncm` demuxer (RC4 global-offset decrypt, metadata parse, three-tier format detection, stable error codes).
- `musicforge-cli`: batch converter with bounded parallelism, naming templates (`{artist}/{album}/{track:02d} {title}`), `--skip-existing`, failure-list CSV export, cancel token, exit-code semantics.
- `musicforge-gui`: Tauri 2 + React 18 desktop workbench (drag & drop, directory ingestion preserving source tree, live progress events, cancel, failure list export).
- Windows NSIS installer (per-user, offline, no file associations, no autostart, no PATH changes) + portable zip.
- Golden-fixture test suite (7 compliant self-constructed fixtures), QA adversarial rounds, 132 test functions.
- CI: three-platform test matrix, zero-warning clippy, zero-network & zero-panic scans, release builds.

### Security

- Core has zero network code paths (CI-enforced); no telemetry, no crash reporting, no analytics.

[Unreleased]: https://github.com/simenty/MusicForge/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/simenty/MusicForge/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/simenty/MusicForge/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/simenty/MusicForge/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/simenty/MusicForge/compare/v0.2.0...v0.4.0
[0.2.0]: https://github.com/simenty/MusicForge/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/simenty/MusicForge/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/simenty/MusicForge/releases/tag/v0.1.0
