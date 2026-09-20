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
  /** 2026-09-12：鉴权总开关状态（false = `MUSICFORGE_AUTH=off`，内网直连模式） */
  auth_enabled: boolean;
  data_dir_writable: boolean;
  data_dir: string;
  library_dir: string | null;
}

// ===================================================== 曲库维度（P1）
// 与 src-tauri/commands/library_db.rs 的 JSON 形状一一对应（camelCase）。

/** 曲目行（db v2 tracks + 解析后的显示名） */
export interface Track {
  id: number;
  sourceId: number;
  path: string;
  size: number;
  title: string | null;
  artist: string | null;
  album: string | null;
  trackNo: number | null;
  durationMs: number | null;
  format: string | null;
  sampleRate: number | null;
  bitDepth: number | null;
  channels: number | null;
  isLossless: boolean;
}

/** 艺术家聚合行 */
export interface Artist {
  id: number;
  name: string;
  trackCount: number;
}

/** 专辑聚合行 */
export interface Album {
  id: number;
  title: string;
  artist: string | null;
  year: number | null;
  trackCount: number;
  /** 本地封面缓存路径（在线补全；null = 尚无封面） */
  coverPath?: string | null;
}

/**
 * 歌单（含曲目数）。
 *
 * 定义在 types.ts（而非 api.ts）：全局搜索结果等复合类型需要引用它，
 * 而 api.ts 反过来依赖 types.ts——放这里避免循环。api.ts 仍再导出，
 * 既有 `import type { Playlist } from "./api"` 不受影响。
 */
export interface Playlist {
  id: number;
  name: string;
  trackCount: number;
}

/** 曲库总览统计 */
export interface LibraryStats {
  tracks: number;
  artists: number;
  albums: number;
  totalSize: number;
  totalDurationMs: number;
}

/** 媒体源 */
export interface Source {
  id: number;
  path: string;
  label: string | null;
  enabled: boolean;
  addedAt: number;
  /** 该源已索引的曲目数（sources_list 聚合） */
  tracksCount: number;
}

/** 索引构建结果 */
export interface IndexOutcome {
  scannedFiles: number;
  audio: number;
  indexed: number;
  tagged: number;
  untagged: number;
  failed: number;
  removed: number;
}

// ===================================================== 播放（P2）

/** 播放状态快照（player_status） */
export interface PlayerSnapshot {
  state: "idle" | "playing" | "paused" | "error";
  error: string | null;
  trackId: number | null;
  title: string | null;
  artist: string | null;
  durationMs: number | null;
  sampleRate: number | null;
  channels: number | null;
  queueLen: number;
  queueIndex: number | null;
  volume: number;
  positionMs: number;
  underruns: number;
}

/** 队列项（前端从曲目行构造） */
export interface QueueItem {
  trackId: number;
  path: string;
  title: string | null;
  artist: string | null;
  durationMs: number | null;
}

/** 播放历史行（Track 字段 + 播放时刻） */
export interface HistoryEntry extends Track {
  /** 播放发生时刻（UNIX 秒） */
  playedAt: number;
  /** 实际播放毫秒（0 = 起播即记，未回写精确时长） */
  msPlayed: number;
}

// ===================================================== 收藏与统计（P3）

// ================================================ 全局搜索（P6.14）

/** 全局搜索结果（search_all：四组各 limit 条；形状与各自 list 一致） */
export interface SearchResults {
  tracks: Track[];
  albums: Album[];
  artists: Artist[];
  playlists: Playlist[];
}

// ================================================ 工具箱：CUE 分轨（P4）

/** CUE 检视结果（cue_inspect） */
export interface CueInspect {
  cue: string;
  album: string | null;
  performer: string | null;
  date: string | null;
  genre: string | null;
  audioFile: string | null;
  audioExists: boolean;
  needsFfmpeg: boolean;
  tracks: { number: number; title: string | null; performer: string | null }[];
}

/** CUE 分轨报告（cue_split；与 CLI `split --json` 同形 + outDir） */
export interface CueSplitReport {
  outDir: string;
  source: string;
  album: string | null;
  tracks: { index: number; title: string | null; path: string; durationSecs: string }[];
  failed: { track: number; reason: string }[];
}

/** 统计总览（stats_overview 一次性拉取） */
export interface StatsOverview {
  tracks: number;
  artists: number;
  albums: number;
  totalSize: number;
  totalDurationMs: number;
  /** 已喜欢的曲目数 */
  liked: number;
  /** 历史累计播放次数 */
  plays: number;
  /** 听过的去重曲目数 */
  playedTracks: number;
  /** 近 7 天按天计数（本地时区 `YYYY-MM-DD`，升序） */
  daily: { day: string; count: number }[];
  /** 最常播放 Top 10（Track + 播放次数） */
  top: (Track & { playCount: number })[];
}
