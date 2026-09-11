// 前端 DTO 类型层（P4-3 从 api.ts 拆出）：与 Rust 侧/HTTP API 的数据形状一一对应。
//
// 形状漂移的防线：契约护栏（scripts/check-api-contract.sh）+ Rust 侧契约测试；
// 本文件只放**类型**，不放逻辑（逻辑见 api.ts / lib/transport.ts）。

// ---- 批处理（convert 域） ----

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

/** 计划预览条目（dry-run 的数据形态；target=null 表示规划失败） */
export interface PlannedItem {
  source: string;
  target: string | null;
  format: string | null;
  error: string | null;
}

/** 拖拽事件载荷（Tauri v2 webview 级） */
export type DragPayload =
  | { type: "enter"; paths: string[] }
  | { type: "over" }
  | { type: "drop"; paths: string[] }
  | { type: "leave" };

// ---- 插件（plugin 域） ----

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

/** format.migrate 结果（outputPath = 已双验产物） */
export interface FormatMigrateResponse {
  outputPath: string;
}

// ---- 扫描 / 曲库刷新（library 域） ----

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

/** P8 LibraryRefresher：库级增量重扫（扫描 + D17 增量哈希缓存刷新/入库）。 */
export interface LibraryRefreshReport {
  dir: string;
  scannedFiles: number;
  scannedDirs: number;
  audio: number;
  /** size+mtime 命中缓存（零文件读取）——增量生效的可见证据 */
  cacheHits: number;
  /** 未命中 → 重算 sha256 并回写缓存 */
  hashed: number;
  skipped: number;
}

// ---- 去重（dedupe 域） ----

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

// ---- 曲库治理（governance 域：整理 / 清洗 / 回收站） ----

export interface OrganizeItem {
  source: string;
  target: string;
  status: "planned" | "in_place" | "skipped_conflict" | "conflict_never";
  note: string | null;
}

export interface OrganizeCounts {
  planned: number;
  in_place: number;
  skipped_conflict: number;
  conflict_never: number;
}

export interface OrganizePlan {
  counts: OrganizeCounts;
  plan: { items: OrganizeItem[]; template: string; strategy: string };
}

export interface OrganizeApplyResult {
  moved: number;
  skipped: number;
  failed: number;
  rollback_manifest: string | null;
}

export interface OrganizeArgs {
  dir: string;
  template?: string;
  targetRoot?: string | null;
  strategy?: string;
}

export interface CleanAction {
  path: string;
  rule_id: string;
}

export interface CleanPlan {
  actions: CleanAction[];
  empty_dirs: string[];
  trash_root: string;
}

export interface CleanApplyResult {
  moved: number;
  dirs_removed: number;
  rollback_manifest: string | null;
}

// ---- 服务端元信息（server 域） ----

/** `GET /api/version`：服务端版本与 API 面标识（服务端形态） */
export interface ServerVersion {
  name: string;
  version: string;
  api_surface: string;
}

/** `GET /api/wizard/status`：首启自检（token 就绪 / 数据目录可写 / 关键路径） */
export interface WizardStatus {
  token_ready: boolean;
  data_dir_writable: boolean;
  data_dir: string;
  library_dir: string | null;
}
