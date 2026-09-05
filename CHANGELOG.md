# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

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

[Unreleased]: https://github.com/simenty/MusicForge/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/simenty/MusicForge/releases/tag/v0.1.0
