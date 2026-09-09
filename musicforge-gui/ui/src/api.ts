// Tauri 桥接层：命令调用 + 事件监听（类型化）
// P8.2.5 双形态传输层：Tauri 桌面 = invoke IPC；fnOS 服务端 = fetch 同源 API。
// 未接 HTTP 的命令在服务端形态下经 invoke 包装**显式降级**（MF-DESKTOP-ONLY，
// 降级铁律：绝不静默装作可用）。
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type { UnlistenFn };

/** Tauri 桌面环境探测（fnOS server 形态下为 false） */
export const IS_DESKTOP =
  typeof window !== "undefined" &&
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    undefined;

const TOKEN_KEY = "mf.server.token";

/** fnOS 服务端形态的访问 token（用户从服务端首启日志获取，存 localStorage） */
export function serverToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? "";
}

export function setServerToken(t: string): void {
  localStorage.setItem(TOKEN_KEY, t);
}

/** HTTP 形态错误：code = MF-* 稳定码（与 CLI/GUI 同码表），message 原文 */
export class HttpApiError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

async function httpPost<T>(path: string, body?: unknown): Promise<T> {
  // B19: 30s 超时——网络挂起时显式失败而非永久 pending
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 30_000);
  let res: Response;
  try {
    res = await fetch(path, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-token": serverToken(),
      },
      body: JSON.stringify(body ?? {}),
      signal: ctl.signal,
    });
  } catch (e) {
    clearTimeout(timer);
    throw new HttpApiError(
      "MF-HTTP-FAILED",
      `服务端不可达或超时（${e instanceof Error && e.name === "AbortError" ? "30s 超时" : String(e)}）`
    );
  }
  clearTimeout(timer);
  let v: { ok: boolean; data?: T; code?: string; message?: string };
  try {
    v = (await res.json()) as typeof v;
  } catch {
    throw new HttpApiError("MF-HTTP-FAILED", `服务端响应非 JSON（HTTP ${res.status}）`);
  }
  if (!v.ok) {
    throw new HttpApiError(v.code ?? "MF-HTTP-FAILED", v.message ?? `HTTP ${res.status}`);
  }
  return v.data as T;
}

/**
 * 命令调用的双形态分发：桌面直通 Tauri IPC；服务端形态对未接 HTTP 的命令
 * 显式降级（已接线命令在各自函数内走 httpPost，不经过此包装的降级路径）。
 */
async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  if (IS_DESKTOP) return tauriInvoke<T>(cmd, args);
  throw new HttpApiError(
    "MF-DESKTOP-ONLY",
    `功能 ${cmd} 需要桌面版：fnOS 服务端形态当前提供 扫描/格式迁移/整理/清洗 域`
  );
}

export type FileStatus = "ok" | "skipped" | "cancelled" | "failed";

export interface FileResult {
  source: string;
  status: FileStatus;
  output: string | null;
  reason: string | null;
}

export interface BatchSummary {
  /** v0.2.0：仅规划未执行的条目数（dry-run 模式） */
  planned: number;
  ok: number;
  skipped: number;
  cancelled: number;
  failed: number;
  durationMs: number;
  isCancelled: boolean;
  results: FileResult[];
}

export interface BatchArgs {
  /** 展开后的输入（G3：root 随行穿透，目录导入保留源结构） */
  inputs: { path: string; root: string | null }[];
  outDir: string | null;
  template: string;
  skipExisting: boolean;
  recursive: boolean;
  jobs: number;
  /** v0.2.0：仅规划不落盘 */
  dryRun: boolean;
}

/** collect_files 返回的展开项：root = 目录输入的根（散文件为 null） */
export interface FileEntry {
  path: string;
  root: string | null;
}

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
  return invoke<string[]>("select_ncm_files", { startDir: startDir ?? null });
}

/** 原生目录选择对话框（输入/输出共用，靠 title 区分）；用户取消返回 null */
export async function selectDirectory(
  startDir: string | null,
  title: string
): Promise<string | null> {
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

// ---- P6a（X37/X36）：插件面板功能态 ----

/** 已安装插件清单（白名单目录内 plugin.json 的解析产物） */
export interface InstalledPlugin {
  name: string;
  apiVersion: string;
  kind: string;
  network: boolean;
  /** P6b.2：高风险插件（格式迁移类）需 ACK 确认后方可调用 */
  ackRequired: boolean;
  /** 能力声明：可迁移扩展名（格式迁移类；AI/在线类为空） */
  extensions: string[];
  dir: string;
}

/** plugins_status 返回形状（键名漂移 = 面板静默断裂，Rust 侧契约测试钉住） */
export interface PluginsStatus {
  /** 当前构建是否含插件宿主（plugin-host feature；发行版 true） */
  runtimeAvailable: boolean;
  configPath: string;
  pluginDirs: string[];
  /** config.json `plugins.enabled` 当前值 */
  enabled: string[];
  /** P6b.2：ACK 闸确认记录（config.json `plugins.acked`） */
  acked: string[];
  installed: InstalledPlugin[];
}

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
  return invoke<string[]>("select_migration_files", {
    extensions,
    startDir: startDir ?? null,
  });
}

/** format.migrate 结果（outputPath = 已双验产物） */
export interface FormatMigrateResponse {
  outputPath: string;
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

/** 计划预览条目（dry-run 的数据形态；target=null 表示规划失败） */
export interface PlannedItem {
  source: string;
  target: string | null;
  format: string | null;
  error: string | null;
}

/** 只规划不执行（dry-run 的前端形态） */
export async function planBatch(args: BatchArgs): Promise<PlannedItem[]> {
  return invoke<PlannedItem[]>("plan_batch", { args });
}

/** P3 扫描：单条发现（与 CLI `scan --json` 的 items 同形状） */
export interface ScanItem {
  path: string;
  category: "audio" | "lyrics" | "cover" | "junk" | "other";
  rule: string | null;
  size: number;
}

/** P3 扫描：规则命中行（规则卡随行带描述与风险，前端免查表） */
export interface ScanRuleHit {
  id: string;
  count: number;
  description: string;
  risk: string;
}

/** P3 扫描：只读扫描报告 */
export interface ScanReport {
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
  ruleHits: ScanRuleHit[];
  items: ScanItem[];
}

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
      ruleHits: ScanRuleHit[];
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

/** P4.5 去重：组内成员（score 为该组上下文中的得分） */
export interface DupFile {
  path: string;
  score: number;
  size: number;
}

/** P4.5 去重：exact 内容重复组（keep = 建议保留，前端可改选） */
export interface DupGroup {
  sha256: string;
  size: number;
  keep: { path: string; score: number; detail: string };
  sacrifices: { path: string; score: number; reason: string }[];
  all: DupFile[];
}

/** P4.5 去重：同名候选组（默认仅报告） */
export interface SameNameGroup {
  stem: string;
  keep: { path: string; score: number };
  candidates: { path: string; score: number; reason: string }[];
}

/** P4.5 去重扫描报告（只读） */
export interface DedupeReport {
  dir: string;
  filesSeen: number;
  hashedNow: number;
  skipped: number;
  groups: DupGroup[];
  sameName: SameNameGroup[];
}

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

export async function startBatch(args: BatchArgs): Promise<void> {
  await invoke("start_batch", { args });
}

export async function cancelBatch(): Promise<boolean> {
  return invoke<boolean>("cancel_batch");
}

export function onBatchFile(handler: (r: FileResult) => void): Promise<UnlistenFn> {
  if (!IS_DESKTOP) return Promise.reject(new HttpApiError("MF-DESKTOP-ONLY", "批处理进度事件需要桌面版"));
  return listen<FileResult>("batch-file", (ev) => handler(ev.payload));
}

export function onBatchDone(handler: (s: BatchSummary) => void): Promise<UnlistenFn> {
  if (!IS_DESKTOP) return Promise.reject(new HttpApiError("MF-DESKTOP-ONLY", "批处理完成事件需要桌面版"));
  return listen<BatchSummary>("batch-done", (ev) => handler(ev.payload));
}

export type DragPayload =
  | { type: "enter"; paths: string[] }
  | { type: "over" }
  | { type: "drop"; paths: string[] }
  | { type: "leave" };

/** 拖拽事件（Tauri v2 webview 级）：enter/over/leave 用于视觉反馈，drop 用于导入 */
export function onDragDropEvent(
  handler: (ev: DragPayload) => void
): Promise<UnlistenFn> {
  if (!IS_DESKTOP) return Promise.reject(new HttpApiError("MF-DESKTOP-ONLY", "拖拽导入需要桌面版"));
  return getCurrentWebview().onDragDropEvent((ev) => handler(ev.payload));
}
