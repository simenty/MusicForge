# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/simenty/MusicForge/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/simenty/MusicForge/compare/v0.2.0...v0.4.0
[0.2.0]: https://github.com/simenty/MusicForge/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/simenty/MusicForge/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/simenty/MusicForge/releases/tag/v0.1.0
