// Tauri 桥接层：命令调用 + 事件监听（类型化）
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type { UnlistenFn };

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
  return invoke<ScanReport>("scan_library", { dir, recursive });
}

export async function startBatch(args: BatchArgs): Promise<void> {
  await invoke("start_batch", { args });
}

export async function cancelBatch(): Promise<boolean> {
  return invoke<boolean>("cancel_batch");
}

export function onBatchFile(handler: (r: FileResult) => void): Promise<UnlistenFn> {
  return listen<FileResult>("batch-file", (ev) => handler(ev.payload));
}

export function onBatchDone(handler: (s: BatchSummary) => void): Promise<UnlistenFn> {
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
  return getCurrentWebview().onDragDropEvent((ev) => handler(ev.payload));
}
