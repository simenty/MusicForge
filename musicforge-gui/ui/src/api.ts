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
  Album,
  Artist,
  BatchArgs,
  BatchSummary,
  CleanApplyResult,
  CleanPlan,
  DedupeReport,
  DragPayload,
  FileEntry,
  FileResult,
  FormatMigrateResponse,
  HistoryEntry,
  IndexOutcome,
  LibraryRefreshReport,
  LibraryStats,
  OrganizeApplyResult,
  OrganizeArgs,
  OrganizePlan,
  PlannedItem,
  PlayerSnapshot,
  PluginsStatus,
  QueueItem,
  CueInspect,
  CueSplitReport,
  ScanItem,
  ScanReport,
  ServerVersion,
  Source,
  StatsOverview,
  Track,
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

/** P5 文件关联：取走冷启动待打开的 .ncm（take 语义，只消费一次） */
export async function takeStartupFiles(): Promise<string[]> {
  if (!IS_DESKTOP) return [];
  return invoke<string[]>("take_startup_files");
}

/** P5 文件关联：二实例转发的待打开文件（已运行实例收到 → 入转换列表） */
export function onOpenFiles(handler: (files: string[]) => void): Promise<UnlistenFn> {
  if (!IS_DESKTOP) return Promise.resolve(() => {});
  return listen<string[]>("open-files", (ev) => handler(ev.payload));
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
// 媒体库域（P1 曲库体验；服务端形态经 invoke 显式降级 MF-DESKTOP-ONLY）
// ---------------------------------------------------------------------------

/** 曲库总览统计（曲目/艺术家/专辑/总大小/总时长） */
export async function libraryStats(): Promise<LibraryStats> {
  return invoke<LibraryStats>("library_stats");
}

/** 分页读取曲目（core 侧 limit 硬上限 500——分页是契约，不传全量） */
export async function listTracks(limit = 200, offset = 0): Promise<Track[]> {
  return invoke<Track[]>("list_tracks", { limit, offset });
}

/** 艺术家聚合列表（按曲目数降序） */
export async function listArtists(): Promise<Artist[]> {
  return invoke<Artist[]>("list_artists");
}

/** 专辑聚合列表（按曲目数降序） */
export async function listAlbums(): Promise<Album[]> {
  return invoke<Album[]>("list_albums");
}

/** 搜索曲目（通配符按字面转义；结果上限 500） */
export async function searchTracks(query: string, limit = 200): Promise<Track[]> {
  return invoke<Track[]>("search_tracks", { query, limit });
}

/** 媒体源列表 */
export async function sourcesList(): Promise<Source[]> {
  return invoke<Source[]>("sources_list");
}

/** 登记媒体源（幂等：同路径重复登记返回同一 id） */
export async function sourcesAdd(path: string, label?: string): Promise<{ id: number }> {
  return invoke<{ id: number }>("sources_add", { path, label: label ?? null });
}

/** 移除媒体源（连带清理其曲目索引；音乐文件不受影响） */
export async function sourcesRemove(id: number): Promise<{ removedTracks: number }> {
  return invoke<{ removedTracks: number }>("sources_remove", { id });
}

/** 对已登记媒体源构建/刷新索引（扫描 → 读标签 → 入库 → 清理陈旧行） */
export async function indexSource(sourceId: number): Promise<IndexOutcome> {
  return invoke<IndexOutcome>("index_source", { sourceId });
}

/** 添加媒体源并立即索引（首次向导一键完成） */
export async function sourcesAddAndIndex(
  path: string,
  label?: string
): Promise<{ id: number; outcome: IndexOutcome }> {
  return invoke<{ id: number; outcome: IndexOutcome }>("sources_add_and_index", {
    path,
    label: label ?? null,
  });
}

// ---------------------------------------------------------------------------
// 播放域（P2；服务端形态经 invoke 显式降级 MF-DESKTOP-ONLY）
// ---------------------------------------------------------------------------

/** 设置队列并从 `index` 开始播放（替换旧队列） */
export async function playerPlayQueue(items: QueueItem[], index: number): Promise<void> {
  return invoke<void>("player_play_queue", { items, index });
}

/** 播放/暂停切换 */
export async function playerToggle(): Promise<void> {
  return invoke<void>("player_toggle");
}

/** 暂停 */
export async function playerPause(): Promise<void> {
  return invoke<void>("player_pause");
}

/** 停止（清空当前曲目，保留队列） */
export async function playerStop(): Promise<void> {
  return invoke<void>("player_stop");
}

/** 下一首 */
export async function playerNext(): Promise<void> {
  return invoke<void>("player_next");
}

/** 上一首 */
export async function playerPrev(): Promise<void> {
  return invoke<void>("player_prev");
}

/** 跳到队列中的指定位置（队列抽屉点选） */
export async function playerJump(index: number): Promise<void> {
  return invoke<void>("player_jump", { index });
}

/** 跳转到指定毫秒 */
export async function playerSeek(ms: number): Promise<void> {
  return invoke<void>("player_seek", { ms });
}

/** 设置音量（0.0–1.0） */
export async function playerSetVolume(volume: number): Promise<void> {
  return invoke<void>("player_set_volume", { volume });
}

/** 播放状态快照（含动态位置/欠载计数；前端轮询） */
export async function playerStatus(): Promise<PlayerSnapshot> {
  return invoke<PlayerSnapshot>("player_status");
}

// ---------------------------------------------------------------------------
// 行为域（P2：喜欢 / 播放历史；服务端形态经 invoke 显式降级 MF-DESKTOP-ONLY）
// ---------------------------------------------------------------------------

/** 切换「喜欢」并返回切换后的状态 */
export async function trackToggleLike(trackId: number): Promise<{ liked: boolean }> {
  return invoke<{ liked: boolean }>("track_toggle_like", { trackId });
}

/** 全部已喜欢的曲目 id（前端 Set 判定行状态；刻意不分页） */
export async function likedIds(): Promise<number[]> {
  return invoke<number[]>("liked_ids");
}

/** 播放历史（倒序；Track 字段 + playedAt/msPlayed） */
export async function playHistory(limit = 200): Promise<HistoryEntry[]> {
  return invoke<HistoryEntry[]>("play_history", { limit });
}

/** 清空播放历史（返回清空条数） */
export async function historyClear(): Promise<{ cleared: number }> {
  return invoke<{ cleared: number }>("history_clear");
}

// ---------------------------------------------------------------------------
// 收藏与统计（P3）
// ---------------------------------------------------------------------------

/** 喜欢的曲目（分页；按收藏时间倒序） */
export async function likedTracks(limit = 200, offset = 0): Promise<Track[]> {
  return invoke<Track[]>("liked_tracks", { limit, offset });
}

/** 统计总览（曲库规模 + 行为计数 + 近 7 天 + 最常播放 Top 10） */
export async function statsOverview(): Promise<StatsOverview> {
  return invoke<StatsOverview>("stats_overview");
}

/** 最近播放（按曲目去重）——首页「继续聆听」 */
export async function recentPlays(limit = 8): Promise<Track[]> {
  return invoke<Track[]>("recent_plays", { limit });
}

// ---------------------------------------------------------------------------
// 工具箱：CUE 分轨（P4）
// ---------------------------------------------------------------------------

/** 原生选择 .cue 文件（取消 → null） */
export async function cuePick(): Promise<string | null> {
  return invoke<string | null>("cue_pick");
}

/** 检视 CUE：曲目清单 + 音频存在性 + 是否需要 ffmpeg（不触碰音频） */
export async function cueInspect(path: string): Promise<CueInspect> {
  return invoke<CueInspect>("cue_inspect", { path });
}

/** 整轨切分（长任务；失败轨不落盘，报告与 CLI 同形） */
export async function cueSplit(cuePath: string, outDir: string): Promise<CueSplitReport> {
  return invoke<CueSplitReport>("cue_split", { cuePath, outDir });
}

// ---------------------------------------------------------------------------
// P5 更新（唯一网络行为——dependency-policy.md 显式例外）
// ---------------------------------------------------------------------------

/** 更新检查结果（check_update） */
export interface UpdateInfo {
  available: boolean;
  version?: string;
  currentVersion?: string;
  notes?: string | null;
}

/** 检查更新（拉取 latest.json；签名校验发生在安装时） */
export async function checkUpdate(): Promise<UpdateInfo> {
  if (!IS_DESKTOP) return { available: false };
  return invoke<UpdateInfo>("check_update");
}

/** 下载并安装更新（签名校验失败即报错；完成后需 restartApp） */
export async function installUpdate(): Promise<void> {
  if (!IS_DESKTOP) return;
  return invoke<void>("install_update");
}

/** 重启应用（更新安装完成后生效） */
export async function restartApp(): Promise<void> {
  if (!IS_DESKTOP) return;
  return invoke<void>("restart_app");
}

// ---------------------------------------------------------------------------
// P6 在线封面（网络边界：仅用户显式触发——见 docs/dependency-policy.md §4）
// ---------------------------------------------------------------------------

/** 为专辑抓取封面（MusicBrainz → Cover Art Archive → 本地缓存）。
 *  返回缓存文件路径；未找到封面 → null；网络失败 → 抛错（可重试） */
export async function coverFetch(albumId: number): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("cover_fetch", { albumId });
}

/** 选择本地图片（原生对话框；取消 → null） */
export async function coverPickImage(): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("cover_pick_image");
}

/** 设置本地封面（**离线能力**：不需要联网；复制进缓存并写回库）。
 *  返回缓存文件路径 */
export async function coverSetLocal(albumId: number, srcPath: string): Promise<string> {
  return invoke<string>("cover_set_local", { albumId, srcPath });
}

/** 曲目所在专辑的封面路径（底栏封面；无 → null） */
export async function trackCover(trackId: number): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("track_cover", { trackId });
}

/** 艺术家代表图（本地查询，不发网络——列表 mount 可安全批量调用） */
export async function artistCoverLocal(artistId: number): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("artist_cover_local", { artistId });
}

/** 艺术家代表图批量本地查询（一次 IPC 拉全部已有封面；`{ id: path }`） */
export async function artistCoversLocal(ids: number[]): Promise<Record<string, string>> {
  if (!IS_DESKTOP) return {};
  return invoke<Record<string, string>>("artist_covers_local", { ids });
}

/** 艺术家代表图（未抓过会当场抓一次——在线；补全按钮用） */
export async function artistCover(artistId: number): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("artist_cover", { artistId });
}

// ---------------------------------------------------------------------------
// P6 歌词（网络边界：打开歌词面板且缓存未命中时——LRCLIB，合规公开 API）
// ---------------------------------------------------------------------------

/** 取歌词（LRC 文本）。缓存优先；无收录 → null；网络失败 → 抛错 */
export async function lyricsFetch(trackId: number): Promise<string | null> {
  if (!IS_DESKTOP) return null;
  return invoke<string | null>("lyrics_fetch", { trackId });
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
  Album,
  Artist,
  BatchArgs,
  BatchSummary,
  CleanAction,
  CleanApplyResult,
  CleanPlan,
  CueInspect,
  CueSplitReport,
  DedupeReport,
  DragPayload,
  DupFile,
  DupGroup,
  FileEntry,
  FileResult,
  FileStatus,
  FormatMigrateResponse,
  HistoryEntry,
  IndexOutcome,
  InstalledPlugin,
  LibraryRefreshReport,
  LibraryStats,
  OrganizeApplyResult,
  OrganizeArgs,
  OrganizeCounts,
  OrganizeItem,
  OrganizePlan,
  PlannedItem,
  PlayerSnapshot,
  PluginsStatus,
  QueueItem,
  SameNameGroup,
  ScanItem,
  ScanReport,
  ScanRuleHit,
  ServerVersion,
  Source,
  StatsOverview,
  Track,
  WizardStatus,
} from "./lib/types";
