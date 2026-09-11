// 业务 API 层（P4-3 拆分后）：按域组织的双形态命令函数，并作为**对外唯一入口**
// （门面 re-export 全部传输层符号与 DTO 类型——组件/测试的 `import ... from "./api"`
// 无需任何改动，拆分对调用方不可见）。
//
// 结构：
//   lib/transport.ts  传输基础（IS_DESKTOP/token/HttpApiError/httpPost/httpGet/invoke）
//   lib/types.ts      DTO 类型（与 Rust/HTTP 数据形状一一对应）
//   本文件            业务函数（批处理 / 插件 / 扫描 / 去重 / 治理 / 服务端元信息）
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  HttpApiError,
  IS_DESKTOP,
  httpGet,
  httpPost,
  invoke,
  type UnlistenFn,
} from "./lib/transport";
import type {
  BatchArgs,
  BatchSummary,
  CleanApplyResult,
  CleanPlan,
  DedupeReport,
  DragPayload,
  FileEntry,
  FileResult,
  FormatMigrateResponse,
  LibraryRefreshReport,
  OrganizeApplyResult,
  OrganizeArgs,
  OrganizePlan,
  PlannedItem,
  PluginsStatus,
  ScanItem,
  ScanReport,
  ServerVersion,
  WizardStatus,
} from "./lib/types";

// ---------------------------------------------------------------------------
// 批处理域（convert）
// ---------------------------------------------------------------------------

export async function previewTemplate(template: string): Promise<string[]> {
  return invoke<string[]>("preview_template", { template });
}

export async function collectFiles(
  inputs: string[],
  recursive: boolean
): Promise<FileEntry[]> {
  return invoke<FileEntry[]>("collect_files", { inputs, recursive });
}

/**
 * 原生多选文件对话框（Rust 侧白名单命令，前端无法自行唤起对话框）。
 * 返回已选路径（未过滤扩展名，交由 collectFiles 统一处理）。
 */
export async function selectNcmFiles(startDir?: string): Promise<string[]> {
  if (!IS_DESKTOP) {
    // P8.2.6：fnOS 形态 = 路径输入框（NAS 路径如 /vol1/music/song.ncm；一次一个，可多次添加）
    const v = window.prompt(
      "File path (full NAS path) / 文件路径（NAS 完整路径）",
      startDir ?? ""
    );
    const p = v?.trim();
    return p ? [p] : [];
  }
  return invoke<string[]>("select_ncm_files", { startDir: startDir ?? null });
}

/** 原生目录选择对话框（输入/输出共用，靠 title 区分）；用户取消返回 null */
export async function selectDirectory(
  startDir: string | null,
  title: string
): Promise<string | null> {
  if (!IS_DESKTOP) {
    // P8.2.6：fnOS 形态 = 路径输入框（返回类型与桌面同型：null=取消）
    const v = window.prompt(
      `${title} — full NAS path / 目录完整路径`,
      startDir ?? ""
    );
    const p = v?.trim();
    return p ? p : null;
  }
  return invoke<string | null>("select_directory", { startDir, title });
}

/**
 * 导出失败清单（原生保存对话框 + 写 CSV）。
 * 格式与 CLI `--export-failures` 完全一致（复用同一导出函数）。
 * 返回写入路径；无失败项或用户取消返回 null。
 */
export async function saveFailures(
  rows: Pick<FileResult, "source" | "status" | "reason">[]
): Promise<string | null> {
  return invoke<string | null>("save_failures", { rows });
}

/** 只规划不执行（dry-run 的前端形态） */
export async function planBatch(args: BatchArgs): Promise<PlannedItem[]> {
  return invoke<PlannedItem[]>("plan_batch", { args });
}

export async function startBatch(args: BatchArgs): Promise<void> {
  await invoke("start_batch", { args });
}

/** P8.2.7：fnOS 形态内置 NCM 批处理——与桌面 startBatch 同源引擎（HTTP 同步
 * 形态：无进度事件/无取消，一次调用返回终态 summary；jobs 服务端硬约束 ≤10）。
 * 仅处理内置转换域（.ncm）——插件格式由调用方走 formatMigrate 分派。 */
export async function runBatchHttp(args: BatchArgs): Promise<BatchSummary> {
  return httpPost<BatchSummary>("/api/batch", {
    inputs: args.inputs,
    out_dir: args.outDir,
    template: args.template,
    skip_existing: args.skipExisting,
    recursive: args.recursive,
    jobs: args.jobs,
    dry_run: args.dryRun,
  });
}

export async function cancelBatch(): Promise<boolean> {
  return invoke<boolean>("cancel_batch");
}

export function onBatchFile(handler: (r: FileResult) => void): Promise<UnlistenFn> {
  if (!IS_DESKTOP) {
    // P8.2.6：优雅降级为 noop（HTTP 形态转换 = 逐文件同步调用，无进度事件）。
    // 此前 reject 会在 SPA 启动期冒泡，被 fnOS 桌面包装成「应用初始化异常」弹窗。
    return Promise.resolve(() => {});
  }
  return listen<FileResult>("batch-file", (ev) => handler(ev.payload));
}

export function onBatchDone(handler: (s: BatchSummary) => void): Promise<UnlistenFn> {
  if (!IS_DESKTOP) {
    // 同上：noop 降级（HTTP 形态的汇总由前端逐文件循环后直接 set）
    return Promise.resolve(() => {});
  }
  return listen<BatchSummary>("batch-done", (ev) => handler(ev.payload));
}

/** 拖拽事件（Tauri v2 webview 级）：enter/over/leave 用于视觉反馈，drop 用于导入；
 * HTTP 形态 noop 降级（浏览器原生拖拽由 App 自行处理） */
export function onDragDropEvent(
  handler: (ev: DragPayload) => void
): Promise<UnlistenFn> {
  if (!IS_DESKTOP) {
    return Promise.resolve(() => {});
  }
  return getCurrentWebview().onDragDropEvent((ev) => handler(ev.payload));
}

// ---------------------------------------------------------------------------
// 插件域（plugin，X37/X36）
// ---------------------------------------------------------------------------

export async function pluginsStatus(): Promise<PluginsStatus> {
  return invoke<PluginsStatus>("plugins_status");
}

export async function pluginsSetEnabled(enabled: string[]): Promise<{ enabled: string[] }> {
  return invoke<{ enabled: string[] }>("plugins_set_enabled", { enabled });
}

/** P6b.2：高风险插件 ACK 确认（幂等；写入 config.json plugins.acked） */
export async function pluginsAcknowledge(name: string): Promise<void> {
  return invoke<void>("plugins_acknowledge", { name });
}

/** P6b.4：按插件能力声明选择待迁移源文件（未选择 → 空数组） */
export async function selectMigrationFiles(
  extensions: string[],
  startDir?: string
): Promise<string[]> {
  if (!IS_DESKTOP) {
    // P8.2.6：fnOS 形态 = 路径输入框（单文件完整路径；目录迁移待 server 域扩展）
    const v = window.prompt(
      `File to migrate (${extensions.join("/")}) / 待迁移文件完整路径（扩展名: ${extensions.join("/")})`,
      startDir ?? ""
    );
    const p = v?.trim();
    return p ? [p] : [];
  }
  return invoke<string[]>("select_migration_files", {
    extensions,
    startDir: startDir ?? null,
  });
}

/** P6b.4：经插件执行本地格式迁移（outputDir 缺省 = 源父目录）；
 * X49：ekey = 用户自备密钥（QMC STag 尾标变体；仅本地传递给插件进程，零网络）；
 * P8.2.5：HTTP 形态 → /api/convert（同形状 outputPath） */
export async function formatMigrate(
  plugin: string,
  source: string,
  outputDir?: string,
  ekey?: string
): Promise<FormatMigrateResponse> {
  if (!IS_DESKTOP) {
    return httpPost<FormatMigrateResponse>("/api/convert", {
      plugin,
      source,
      output_dir: outputDir ?? null,
      ekey: ekey?.trim() ? ekey.trim() : null,
    });
  }
  return invoke<FormatMigrateResponse>("format_migrate", {
    plugin,
    source,
    outputDir: outputDir ?? null,
    ekey: ekey?.trim() ? ekey.trim() : null,
  });
}

// ---------------------------------------------------------------------------
// 扫描域（library）
// ---------------------------------------------------------------------------

/** 只读扫描曲库目录（不改动任何文件；错误经 Result 显式返回，不静默） */
export async function scanLibrary(dir: string, recursive: boolean): Promise<ScanReport> {
  if (!IS_DESKTOP) {
    const d = await httpPost<{
      dir: string;
      scannedFiles: number;
      scannedDirs: number;
      summary: {
        audio: number;
        lyrics: number;
        covers: number;
        junk: number;
        other: number;
        emptyDirs: number;
      };
      ruleHits: ScanReport["ruleHits"];
      items: { path: string; category: string; rule: string | null; size: number }[];
      unauthorizedDirs: string[];
    }>("/api/scan", { dir, recursive });
    return {
      dir: d.dir,
      scannedFiles: d.scannedFiles,
      scannedDirs: d.scannedDirs,
      summary: d.summary,
      ruleHits: d.ruleHits,
      items: d.items.map((i) => ({
        path: i.path,
        category: i.category as ScanItem["category"],
        rule: i.rule,
        size: i.size,
      })),
    };
  }
  return invoke<ScanReport>("scan_library", { dir, recursive });
}

/** P8：刷新曲库（增量）——桌面 IPC / fnOS HTTP 双形态同语义 */
export async function refreshLibrary(dir: string): Promise<LibraryRefreshReport> {
  if (!IS_DESKTOP) {
    return httpPost<LibraryRefreshReport>("/api/library/refresh", { dir });
  }
  return invoke<LibraryRefreshReport>("refresh_library", {
    dir,
    stateDb: null,
  });
}

// ---------------------------------------------------------------------------
// 去重域（dedupe）
// ---------------------------------------------------------------------------

/** 只读去重扫描（组内对比 + 建议保留；人工改选在前端完成后经 dedupeApply 提交） */
export async function dedupeScan(dir: string): Promise<DedupeReport> {
  return invoke<DedupeReport>("dedupe_scan", { dir });
}

/** 执行用户改选后的去重（牺牲清单进回收站；服务端强校验路径在 dir 内） */
export async function dedupeApply(
  dir: string,
  sacrificePaths: string[]
): Promise<{ requested: number; moved: number; rollback: string | null }> {
  return invoke("dedupe_apply", { dir, sacrificePaths });
}

// ---------------------------------------------------------------------------
// 曲库治理域（governance：整理 / 清洗 / 回收站还原，**服务端形态专属**）
//
// 形态差异：桌面（Tauri）未暴露这三类命令（CLI 与 server 已有），因此
// IS_DESKTOP 时抛 MF-SERVER-ONLY——显式可见，绝不静默（项目一贯原则）。
//
// 破坏类语义（与后端一致）：必须 plan（只读预览）→ 人工确认 → apply（confirm:true）。
// 后端对 confirm !== true 一律 403 MF-OP-NEEDS-YES。
// ---------------------------------------------------------------------------

/** `POST /api/organize/plan`：整理计划预览（只读，绝不移动） */
export async function organizePlan(args: OrganizeArgs): Promise<OrganizePlan> {
  if (IS_DESKTOP) {
    throw new HttpApiError(
      "MF-SERVER-ONLY",
      "整理为服务端形态能力（fnOS / 自建 server）；桌面版请用 CLI：musicforge organize"
    );
  }
  return httpPost<OrganizePlan>("/api/organize/plan", {
    dir: args.dir,
    template: args.template ?? undefined,
    target_root: args.targetRoot ?? undefined,
    strategy: args.strategy ?? undefined,
  });
}

/** `POST /api/organize/apply`：执行整理（破坏类，confirm 强制） */
export async function organizeApply(args: OrganizeArgs): Promise<OrganizeApplyResult> {
  if (IS_DESKTOP) {
    throw new HttpApiError(
      "MF-SERVER-ONLY",
      "整理为服务端形态能力（fnOS / 自建 server）；桌面版请用 CLI：musicforge organize"
    );
  }
  return httpPost<OrganizeApplyResult>("/api/organize/apply", {
    dir: args.dir,
    template: args.template ?? undefined,
    target_root: args.targetRoot ?? undefined,
    strategy: args.strategy ?? undefined,
    confirm: true,
  });
}

/** `POST /api/clean/plan`：垃圾清洗计划预览（只读 dry-run）；rules 缺省 = 全部规则 */
export async function cleanPlan(dir: string, rules?: string): Promise<CleanPlan> {
  if (IS_DESKTOP) {
    throw new HttpApiError(
      "MF-SERVER-ONLY",
      "清洗为服务端形态能力（fnOS / 自建 server）；桌面版请用 CLI：musicforge clean"
    );
  }
  return httpPost<CleanPlan>("/api/clean/plan", { dir, rules: rules || undefined });
}

/** `POST /api/clean/apply`：执行清洗（进回收站可整体还原，confirm 强制） */
export async function cleanApply(dir: string, rules?: string): Promise<CleanApplyResult> {
  if (IS_DESKTOP) {
    throw new HttpApiError(
      "MF-SERVER-ONLY",
      "清洗为服务端形态能力（fnOS / 自建 server）；桌面版请用 CLI：musicforge clean"
    );
  }
  return httpPost<CleanApplyResult>("/api/clean/apply", {
    dir,
    rules: rules || undefined,
    confirm: true,
  });
}

/**
 * `POST /api/trash/restore`：按回滚清单整体还原。
 *
 * 服务端约束（AUD-6）：manifest 必须位于 `.musicforge` 体系内且为 *.jsonl，
 * 否则 403 MF-TRASH-MANIFEST-INVALID（防越权还原任意路径）。
 */
export async function trashRestore(manifest: string): Promise<{ restored: number }> {
  if (IS_DESKTOP) {
    throw new HttpApiError(
      "MF-SERVER-ONLY",
      "回收站还原为服务端形态能力（fnOS / 自建 server）；桌面版请用 CLI：musicforge trash restore"
    );
  }
  return httpPost<{ restored: number }>("/api/trash/restore", { manifest, confirm: true });
}

// ---------------------------------------------------------------------------
// 服务端元信息（P2 收尾：契约缺口闭合——/version、/wizard/status）
//
// 这两个端点此前"后端已实现但前端未接"（契约护栏持续提示）。接入后
// 前端调用面与 server 端点面完全对齐（11/11）。
// ---------------------------------------------------------------------------

export async function serverVersion(): Promise<ServerVersion> {
  if (IS_DESKTOP) {
    throw new HttpApiError("MF-SERVER-ONLY", "服务端信息仅在服务端形态（fnOS / 自建 server）可用");
  }
  return httpGet<ServerVersion>("/api/version");
}

export async function wizardStatus(): Promise<WizardStatus> {
  if (IS_DESKTOP) {
    throw new HttpApiError("MF-SERVER-ONLY", "服务端自检仅在服务端形态（fnOS / 自建 server）可用");
  }
  return httpGet<WizardStatus>("/api/wizard/status");
}

// ---------------------------------------------------------------------------
// 门面（facade）：对外契约 = 原 api.ts 的全部导出。
// 组件与测试的 `import { ... } from "./api"` 不因拆分而改变。
// ---------------------------------------------------------------------------

export {
  HttpApiError,
  IS_DESKTOP,
  IS_SERVER_MODE,
  serverToken,
  setServerToken,
} from "./lib/transport";
export type { UnlistenFn } from "./lib/transport";

export type {
  BatchArgs,
  BatchSummary,
  CleanAction,
  CleanApplyResult,
  CleanPlan,
  DedupeReport,
  DragPayload,
  DupFile,
  DupGroup,
  FileEntry,
  FileResult,
  FileStatus,
  FormatMigrateResponse,
  InstalledPlugin,
  LibraryRefreshReport,
  OrganizeApplyResult,
  OrganizeArgs,
  OrganizeCounts,
  OrganizeItem,
  OrganizePlan,
  PlannedItem,
  PluginsStatus,
  SameNameGroup,
  ScanItem,
  ScanReport,
  ScanRuleHit,
  ServerVersion,
  WizardStatus,
} from "./lib/types";
