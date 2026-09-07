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

## 2. Crates & modules

| Crate | Network | Responsibility |
|:--|:-:|:--|
| `musicforge-core` | ✗ | format framework, scan, clean, dedupe, organize, playlist, plan, task, report, safety, state (db) |
| `musicforge-cli` | ✗ | zero-dependency CLI for desktop & NAS/SSH |
| `musicforge-gui` | ✗ | Tauri shell + React workbench |
| `musicforge-plugin-api` | ✗ | plugin protocol, schemas, permission model |
| `musicforge-plugin-host` | ✗ | process spawn/admission/timeout/isolation |
| `musicforge-ui-protocol` | ✗ | single source of truth for UI event schemas |
| `musicforge-server` (P8) | local HTTP | NAS web host, serves the same SPA |

`musicforge-core` module map (as implemented, P3/P4):

```text
formats/   FormatAdapter + FormatRegistry (ncm built-in, magic-first detection)
metadata/  model + tagger (lofty; FillMissingOnly semantics) + template engine
scan.rs    read-only recursive walker (prunes .musicforge/), 9 rule cards,
           trash-based clean executor + rollback.jsonl + restore
db.rs      state layer (SQLite library.db: files index / hash cache / tasks / ack)
dedupe.rs  exact grouping + same-name candidates + explainable keep-score
           + similar-cover aHash clustering (report-only)
organize.rs template placement + conflict strategies (never overwrites) + idempotent
playlist.rs M3U8 export by category + import path repair
stylecode.rs filename style-code parser ([Y23-S01-...]) + genre write plan/apply
```

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
- Library-lifecycle commands (clean / dedupe / organize) never delete: sacrifices go to
  `<root>/.musicforge/trash/<task>/` with a `rollback.jsonl` (`from`↔`to`); `clean --restore`
  replays it in reverse. The scanner prunes `.musicforge/` so tool state never re-enters
  scans, plans, or reorganizations.
- Read-only commands (`scan`, `dedupe` without `--apply`, `playlist`, `genre` without
  `--apply`) mutate nothing; `genre --apply` writes tags in place but never overwrites an
  existing genre value unless `--replace-all` (high-risk grading requires `--yes`).

## 5. Safety boundaries

- Plugin admission is enforced at load time (permission manifest; delete/move/upload ⇒ refuse).
- Suggestions-only rule: plugins cannot mutate anything; the Core plan layer does.
- Path safety is centralized in `safety/path.rs` (long-path `\\?\`, escapes, symlink policy).
- Threat model: [threat-model.md](threat-model.md).
