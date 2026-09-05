# Architecture

MusicForge is a three-layer, offline-first music asset platform. This document is the
authoritative boundary description; the executable plan lives in [ROADMAP.md](../ROADMAP.md).

## 1. Layers

```text
┌────────────────────────────────────────────┐
│  musicforge-gui (Tauri 2 + React 18 SPA)   │  desktop shell; same SPA runs in
│  musicforge-server (Axum, NAS, P8)         │  browser mode via api.ts transport
└─────────────────────┬──────────────────────┘
                      │ commands / HTTP (musicforge-ui-protocol events)
┌─────────────────────▼──────────────────────┐
│              musicforge-core               │
│  formats/ · metadata/ · scan/ · clean/     │
│  dedupe/ · organize/ · playlist/ · plan/   │
│  task/ · report/ · safety/ · touch/ · db/  │
│  plugin-host · ZERO network code paths     │
└─────────────────────┬──────────────────────┘
                      │ stdio NDJSON (strongly-typed, versioned)
┌─────────────────────▼──────────────────────┐
│        plugin processes (opt-in)           │
│  musicforge-plugins        → AI / online   │
│  musicforge-format-plugins → format migr.  │  ← the ONLY place with network
└────────────────────────────────────────────┘
```

**Layer rule (D12):** capabilities sink down, never up. Anything requiring network/AI belongs
in a plugin; anything requiring accounts belongs in the optional cloud repo. A PR that moves
network code into core is rejected on sight.

## 2. Crates

| Crate | Network | Responsibility |
|:--|:-:|:--|
| `musicforge-core` | ✗ | format framework, scan, clean, dedupe, organize, playlist, plan, task, report, safety, state (db) |
| `musicforge-cli` | ✗ | zero-dependency CLI for desktop & NAS/SSH |
| `musicforge-gui` | ✗ | Tauri shell + React workbench |
| `musicforge-plugin-api` | ✗ | plugin protocol, schemas, permission model |
| `musicforge-plugin-host` | ✗ | process spawn/admission/timeout/isolation |
| `musicforge-ui-protocol` | ✗ | single source of truth for UI event schemas |
| `musicforge-server` (P8) | local HTTP | NAS web host, serves the same SPA |

## 3. Format adapter boundary

- `FormatAdapter` trait (probe / decode / preferred extension) with a `FormatRegistry`.
- `.ncm` is the **first, built-in adapter** — it stays in-process (test-protected, zero legal risk).
- External format plugins register through `PluginFormatAdapter` (host bridge); the registry
  probes built-ins first, then enabled plugins.
- Format detection priority: magic bytes → container structure → metadata → extension → user-pinned plugin.

## 4. Task pipeline (all batch operations)

```text
Scan → Plan → Dry-run → Apply → Verify → Report → Trash / Undo
```

- Destructive commands (clean/dedupe/organize) default to dry-run; `--apply` executes.
- Every executed item is recorded in `manifest.jsonl` (portable archive) and the state db
  (queryable history). The db is a **renewable cache**: the filesystem + manifests are the
  source of truth.
- All writes go through temp-file + atomic rename; interrupted batches resume from the manifest.

## 5. Safety boundaries

- Plugin admission is enforced at load time (permission manifest; delete/move/upload ⇒ refuse).
- Suggestions-only rule: plugins cannot mutate anything; the Core plan layer does.
- Path safety is centralized in `safety/path.rs` (long-path `\\?\`, escapes, symlink policy).
- Threat model: [threat-model.md](threat-model.md).
