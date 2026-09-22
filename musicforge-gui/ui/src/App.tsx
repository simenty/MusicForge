import { Suspense, lazy, useCallback, useEffect, useRef, useState } from "react";
import {
  onOpenFiles,
  selectDirectory,
  selectNcmFiles,
  serverToken,
  setServerToken,
  takeStartupFiles,
  IS_DESKTOP,
} from "./api";
import { useLang, type Lang } from "./i18n";
import { useServerAuth } from "./useServerAuth";
import { useToast } from "./hooks/useToast";
import { useSettings } from "./hooks/useSettings";
import { useBatch, ROW_H, type FilterKey, type RowStatus } from "./hooks/useBatch";
import { fileName, relOutput, formatDuration } from "./lib/format";
// 外壳常驻（首屏必装）：错误边界 / 服务端信息卡 / 首启引导 / 播放底栏
import ErrorBoundary from "./ErrorBoundary";
import ServerInfoCard from "./ServerInfoCard";
import WelcomeGuide from "./WelcomeGuide";
import PlayerBar from "./PlayerBar";

// P6.13 体积治理：页面级懒加载——首屏只装外壳，页面在首次进入时才取
// （Vite 依据动态 import 自动切分 chunk）。共享件（TrackRow / api / i18n / 图标）
// 仍留在主包：被多个页面用到，拆出去只会重复下载。
const MediaHome = lazy(() => import("./MediaHome"));
const LibraryPage = lazy(() => import("./LibraryPage"));
const ArtistsPage = lazy(() => import("./ArtistsPage"));
const AlbumsPage = lazy(() => import("./AlbumsPage"));
const PlaylistsPage = lazy(() => import("./PlaylistsPage"));
const SourcesPage = lazy(() => import("./SourcesPage"));
const HistoryPage = lazy(() => import("./HistoryPage"));
const FavoritesPage = lazy(() => import("./FavoritesPage"));
const StatsPage = lazy(() => import("./StatsPage"));
const CuePanel = lazy(() => import("./CuePanel"));
const ScanPanel = lazy(() => import("./ScanPanel"));
const DedupePanel = lazy(() => import("./DedupePanel"));
const OrganizePanel = lazy(() => import("./OrganizePanel"));
const CleanPanel = lazy(() => import("./CleanPanel"));
const TrashPanel = lazy(() => import("./TrashPanel"));
const PluginPanel = lazy(() => import("./PluginPanel"));
const UpdateSection = lazy(() => import("./UpdateSection"));
const SearchPalette = lazy(() => import("./SearchPalette"));
// 类型只用于接线，不进运行时（避免把面板拉进首屏包）
import type { SearchTarget } from "./SearchPalette";
import { usePlayer } from "./hooks/usePlayer";
import {
  IconClock,
  IconConvert,
  IconCopy,
  IconDisc,
  IconDownload,
  IconFolder,
  IconHeart,
  IconHome,
  IconLibrary,
  IconList,
  IconMenu,
  IconSearch,
  IconPlan,
  IconPlay,
  IconPlugin,
  IconPlus,
  IconOrganize,
  IconRestore,
  IconScan,
  IconClean,
  IconSettings,
  IconStop,
  IconTrash,
  IconUser,
} from "./icons";

/** 主分区（P4 信息架构：媒体库 / 工具箱 / 设置） */
type ViewKey = "media" | "toolbox" | "settings";

/** 状态视觉元数据（文案走 i18n：RowStatus 键与字典 status 命名空间同名） */
const STATUS_META: Record<RowStatus, { cls: string; icon: string }> = {
  pending: { cls: "s-pending", icon: "●" },
  ok: { cls: "s-ok", icon: "●" },
  skipped: { cls: "s-skipped", icon: "●" },
  failed: { cls: "s-failed", icon: "●" },
  cancelled: { cls: "s-cancelled", icon: "●" },
};

/** 筛选键（label = all → t.filter.all；其余键与 t.status 同名） */
const FILTERS: FilterKey[] = ["all", "pending", "ok", "skipped", "failed", "cancelled"];

/**
 * App：只负责**分区导航 + 视图编排 + 渲染**。
 *
 * P1-3 拆分后，批处理编排下沉到 `hooks/useBatch`、设置下沉到 `hooks/useSettings`、
 * 轻提示下沉到 `hooks/useToast`、纯字符串变换下沉到 `lib/format`——本文件的
 * 每个 useEffect/useCallback 都只服务于界面本身。
 */
export default function App() {
  const { t, lang, setLang } = useLang();
  /** 当前主分区（默认「媒体库」——音乐应用打开即见曲库） */
  const [view, setView] = useState<ViewKey>("media");
  /** 鉴权总开关（false = MUSICFORGE_AUTH=off：隐藏 token 框、显示状态徽标） */
  const { authEnabled } = useServerAuth();
  /** 窄屏抽屉（≤860px 时侧栏转为抽屉，由顶栏汉堡开关） */
  const [navOpen, setNavOpen] = useState(false);
  /** P6.14 全局搜索面板 */
  const [searchOpen, setSearchOpen] = useState(false);
  /** P6.14 搜索跳转目标：命中专辑/艺术家/歌单时展开其详情 */
  const [focusId, setFocusId] = useState<{
    kind: "album" | "artist" | "playlist";
    id: number;
  } | null>(null);

  /** 工具箱分区的二级菜单（P4 能力归位）：转换 / 扫描 / 去重 / 整理 / 清理 / 回收站 / CUE / 插件 */
  const [toolboxTab, setToolboxTab] = useState<
    "convert" | "scan" | "dedupe" | "organize" | "clean" | "trash" | "cue" | "plugins"
  >("convert");
  /** 媒体库分区的二级菜单：概览 / 音乐库 / 艺术家 / 专辑 / 歌单 / 喜欢 / 历史 / 统计 / 媒体源 */
  const [mediaTab, setMediaTab] = useState<
    | "home"
    | "library"
    | "artists"
    | "albums"
    | "playlists"
    | "favorites"
    | "history"
    | "stats"
    | "sources"
  >("home");
  /** 顶栏标题：当前分区（曲库时附二级项名——布局吸收后导航在左侧栏，顶栏只显示位置） */
  const mediaTabLabel =
    mediaTab === "home"
      ? t.media.tabHome
      : mediaTab === "library"
        ? t.media.tabLibrary
        : mediaTab === "artists"
          ? t.media.tabArtists
          : mediaTab === "albums"
            ? t.media.tabAlbums
            : mediaTab === "playlists"
              ? t.media.tabPlaylists
              : mediaTab === "favorites"
              ? t.media.tabFavorites
              : mediaTab === "history"
                ? t.media.tabHistory
                : mediaTab === "stats"
                  ? t.media.tabStats
                  : t.media.tabSources;
  const toolboxTabLabel =
    toolboxTab === "convert"
      ? t.app.navConvert
      : toolboxTab === "scan"
        ? t.library.tabScan
        : toolboxTab === "dedupe"
          ? t.library.tabDedupe
          : toolboxTab === "organize"
            ? t.library.tabOrganize
            : toolboxTab === "clean"
              ? t.library.tabClean
              : toolboxTab === "trash"
                ? t.library.tabTrash
                : toolboxTab === "cue"
                  ? t.cue.tab
                  : t.app.navPlugins;
  const viewLabel =
    view === "media"
      ? `${t.media.nav} · ${mediaTabLabel}`
      : view === "toolbox"
        ? `${t.app.navToolbox} · ${toolboxTabLabel}`
        : t.app.navSettings;
  // P8.2.5：fnOS 服务端形态的访问 token（HTTP 形态标题栏可见可改）
  const [serverTokenInput, setServerTokenInput] = useState<string>(serverToken());
  const [filter, setFilter] = useState<FilterKey>("all");

  const { toast, setToast, showToast } = useToast();
  const { settings, patch, preview } = useSettings();
  /** P2 播放（底栏 + 双击播放；服务端形态下 status 恒为 null） */
  const player = usePlayer();
  const b = useBatch({ t, settings, showToast, filter });

  // P5 文件关联：冷启动参数 + 二实例转发 → 切到工具箱·NCM 转换并入列。
  // ref 模式：importPaths 随 rows/running 变化，但事件只需注册一次。
  const importPathsRef = useRef(b.importPaths);
  importPathsRef.current = b.importPaths;
  useEffect(() => {
    if (!IS_DESKTOP) return;
    const open = (files: string[]) => {
      if (files.length === 0) return;
      setView("toolbox");
      setToolboxTab("convert");
      void importPathsRef.current(files, false);
    };
    void takeStartupFiles().then(open);
    let stop: (() => void) | undefined;
    void onOpenFiles(open).then((un) => {
      stop = un;
    });
    return () => stop?.();
  }, []);

  // P6.14 全局搜索：Ctrl/Cmd+K 唤起（输入框内也生效——浏览器默认是聚焦搜索栏）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  /** P6.14 搜索跳转：切到对应分区，并让该页展开命中的专辑/艺术家/歌单 */
  const openSearchTarget = (target: SearchTarget) => {
    if (target.kind === "track") return;
    setView("media");
    setMediaTab(
      target.kind === "album" ? "albums" : target.kind === "artist" ? "artists" : "playlists"
    );
    setFocusId({ kind: target.kind, id: target.id });
  };
  const {
    rows,
    summary,
    plannedRows,
    setPlannedRows,
    running,
    fatal,
    setFatal,
    dragOver,
    viewRef,
    setScrollTop,
    filtered,
    visible,
    start,
    filterCounts,
    counts,
    doneCount,
    total,
    pct,
    finishedMs,
    startRun,
    doCancel,
    clearList,
    removeRow,
    exportFailures,
    importPaths,
    guard,
  } = b;

  const addFiles = useCallback(() => {
    guard(
      t.app.labelAddFiles,
      selectNcmFiles(settings.outDir || undefined).then(async (picked) => {
        if (picked.length) await importPaths(picked, false);
      })
    );
  }, [guard, importPaths, settings.outDir, t]);

  const addFolder = useCallback(() => {
    guard(
      t.app.labelAddFolder,
      selectDirectory(settings.outDir || null, t.app.pickFolderTitle).then(async (dir) => {
        if (dir) await importPaths([dir], settings.recursive);
      })
    );
  }, [guard, importPaths, settings.outDir, settings.recursive, t]);

  const browseOutDir = useCallback(() => {
    guard(
      t.app.labelOutDir,
      selectDirectory(settings.outDir || null, t.app.pickOutDirTitle).then((dir) => {
        if (dir) patch({ outDir: dir });
      })
    );
  }, [guard, patch, settings.outDir, t]);

  const filterLabel = (key: FilterKey): string =>
    key === "all" ? t.filter.all : t.status[key];

  return (
    <div className="window">
      {fatal && (
        <div className="fatal" onClick={() => setFatal(null)} title={t.app.clickToClose}>
          <b>{t.app.fatalTitle}</b>
          <pre>{fatal}</pre>
        </div>
      )}
      {plannedRows && plannedRows.length > 0 && (
        <div className="plan-panel">
          <div className="plan-head">
            <b>
              {t.app.planPreviewTitle(
                plannedRows.length,
                plannedRows.filter((i) => i.error === null).length
              )}
            </b>
            <button className="btn sm" onClick={() => setPlannedRows(null)}>
              {t.app.close}
            </button>
          </div>
          <div className="plan-body">
            {plannedRows.map((i) => (
              <div key={i.source} className={"plan-row" + (i.error ? " plan-err" : "")}>
                <span className="plan-src" title={i.source}>
                  {i.source}
                </span>
                <span className="plan-arrow">→</span>
                <span className="plan-dst" title={i.target ?? i.error ?? ""}>
                  {i.error ? `✕ ${i.error}` : (i.target ?? "")}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
      {/* ---------- 左侧栏（布局吸收：品牌 + 主导航 + 曲库治理分组常驻） ---------- */}
      <aside className={"sidebar" + (navOpen ? " open" : "")} aria-label="主导航">
        <div className="brand">
          <span className="logo">
            <IconConvert size={18} />
          </span>
          <div className="brand-text">
            <strong>MusicForge</strong>
            <span className="sub">{t.app.subtitle}</span>
          </div>
        </div>

        <nav className="nav">
          {/* 媒体库（P1）：仅桌面形态——曲库维度索引在桌面端建立；
              NAS Web UI（服务端形态）不显示该分区（页面内也有 desktopOnly 兜底文案） */}
          {IS_DESKTOP && (
            <div className="nav-group">
              <button
                className={"nav-item" + (view === "media" ? " on" : "")}
                onClick={() => {
                  setView("media");
                  setNavOpen(false);
                }}
              >
                <IconHome />
                <span>{t.media.nav}</span>
              </button>
              <div className="nav-sub">
                <button
                  className={"nav-item sub" + (view === "media" && mediaTab === "home" ? " on" : "")}
                  onClick={() => {
                    setView("media");
                    setMediaTab("home");
                    setNavOpen(false);
                  }}
                >
                  <IconHome />
                  <span>{t.media.tabHome}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "library" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("library");
                    setNavOpen(false);
                  }}
                >
                  <IconLibrary />
                  <span>{t.media.tabLibrary}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "artists" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("artists");
                    setNavOpen(false);
                  }}
                >
                  <IconUser />
                  <span>{t.media.tabArtists}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "albums" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("albums");
                    setNavOpen(false);
                  }}
                >
                  <IconDisc />
                  <span>{t.media.tabAlbums}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "playlists" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("playlists");
                    setNavOpen(false);
                  }}
                >
                  <IconList />
                  <span>{t.media.tabPlaylists}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "favorites" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("favorites");
                    setNavOpen(false);
                  }}
                >
                  <IconHeart />
                  <span>{t.media.tabFavorites}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "history" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("history");
                    setNavOpen(false);
                  }}
                >
                  <IconClock />
                  <span>{t.media.tabHistory}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "stats" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("stats");
                    setNavOpen(false);
                  }}
                >
                  <IconDisc />
                  <span>{t.media.tabStats}</span>
                </button>
                <button
                  className={
                    "nav-item sub" + (view === "media" && mediaTab === "sources" ? " on" : "")
                  }
                  onClick={() => {
                    setView("media");
                    setMediaTab("sources");
                    setNavOpen(false);
                  }}
                >
                  <IconFolder />
                  <span>{t.media.tabSources}</span>
                </button>
              </div>
            </div>
          )}

          <div className="nav-sep" role="separator"></div>

          {/* 工具箱（P4 能力归位：NCM 转换 / 治理 5 项 / CUE 分轨 / 插件——不占主导航心智） */}
          <div className="nav-group">
            <button
              className={"nav-item" + (view === "toolbox" ? " on" : "")}
              onClick={() => {
                setView("toolbox");
                setNavOpen(false);
              }}
            >
              <IconConvert />
              <span>{t.app.navToolbox}</span>
            </button>
            <div className="nav-sub">
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "convert" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("convert");
                  setNavOpen(false);
                }}
              >
                <IconConvert />
                <span>{t.app.navConvert}</span>
                {rows.length > 0 && <span className="badge">{rows.length}</span>}
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "scan" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("scan");
                  setNavOpen(false);
                }}
              >
                <IconScan />
                <span>{t.library.tabScan}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "dedupe" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("dedupe");
                  setNavOpen(false);
                }}
              >
                <IconCopy />
                <span>{t.library.tabDedupe}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "organize" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("organize");
                  setNavOpen(false);
                }}
              >
                <IconOrganize />
                <span>{t.library.tabOrganize}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "clean" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("clean");
                  setNavOpen(false);
                }}
              >
                <IconClean />
                <span>{t.library.tabClean}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "trash" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("trash");
                  setNavOpen(false);
                }}
              >
                <IconRestore />
                <span>{t.library.tabTrash}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "cue" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("cue");
                  setNavOpen(false);
                }}
              >
                <IconDisc />
                <span>{t.cue.tab}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "toolbox" && toolboxTab === "plugins" ? " on" : "")}
                onClick={() => {
                  setView("toolbox");
                  setToolboxTab("plugins");
                  setNavOpen(false);
                }}
              >
                <IconPlugin />
                <span>{t.app.navPlugins}</span>
              </button>
            </div>
          </div>

          <div className="nav-sep" role="separator"></div>

          <div className="nav-group">
            <button
              className={"nav-item" + (view === "settings" ? " on" : "")}
              onClick={() => {
                setView("settings");
                setNavOpen(false);
              }}
            >
              <IconSettings />
              <span>{t.app.navSettings}</span>
            </button>
          </div>
        </nav>
      </aside>

      {/* 窄屏抽屉遮罩 */}
      {navOpen && <div className="nav-mask" onClick={() => setNavOpen(false)} aria-hidden="true"></div>}

      {/* ---------- 右区：顶栏 + 内容 ---------- */}
      <div className="main-wrap">
        <header className="titlebar">
          <button className="icon-btn menu-toggle" onClick={() => setNavOpen(true)} aria-label="打开导航">
            <IconMenu />
          </button>
          <strong className="tb-title">{viewLabel}</strong>
          <div className="chips">
            <span className="chip green">{t.app.offlineChip}</span>
            <span className="chip">MIT</span>
            <span className="chip">v0.10.0</span>
            {!IS_DESKTOP &&
              (authEnabled === false ? (
                // 鉴权已关闭（MUSICFORGE_AUTH=off）：隐藏无用的 token 框，改显状态徽标
                <span className="chip" title={t.serverInfo.authOffNote}>
                  {t.serverInfo.authOff}
                </span>
              ) : (
                <input
                  className={
                    "lang-select server-token" + (serverTokenInput.trim() ? "" : " needs-token")
                  }
                  type="password"
                  placeholder={t.app.tokenPlaceholder}
                  value={serverTokenInput}
                  onChange={(e) => {
                    setServerTokenInput(e.target.value);
                    setServerToken(e.target.value);
                  }}
                  spellCheck={false}
                  aria-label={t.app.tokenPlaceholder}
                  title={t.auth.where}
                />
              ))}
            {/* P6.14 全局搜索入口（Ctrl/Cmd+K 亦可唤起） */}
            <button
              className="btn sm search-btn"
              onClick={() => setSearchOpen(true)}
              title={t.search.shortcut}
              aria-label={t.search.title}
            >
              <IconSearch size={14} />
              <span>{t.search.title}</span>
            </button>
            <select
              className="lang-select"
              value={lang}
              onChange={(e) => setLang(e.target.value as Lang)}
              aria-label={t.lang.aria}
            >
              <option value="zh">中文</option>
              <option value="en">English</option>
            </select>
          </div>
        </header>

      <main className="main">
      {/* P6.13：懒加载页面首次进入时的占位（本地文件加载，通常一闪而过） */}
      <Suspense fallback={<div className="media-empty"><p>{t.media.loading}</p></div>}>
      {/* P0-2：分区级错误边界——任一分区渲染异常只降级该分区，不带走整个应用 */}
      <ErrorBoundary
        title={t.app.errorTitle}
        hint={t.app.errorHint}
        retry={t.app.errorRetry}
      >
      {view === "toolbox" && toolboxTab === "convert" && (
        <>
      {/* ---------- 导入操作区 ---------- */}
      <div className="panel">
        <div className="panel-head">
          <h2>{t.app.navConvert}</h2>
        </div>
      <div className="toolbar">
        <div className="tb-left">
          <button className="btn" onClick={addFiles} disabled={running}>
            <IconPlus />
            {t.app.addFiles}
          </button>
          <button className="btn" onClick={addFolder} disabled={running}>
            <IconFolder />
            {t.app.addFolder}
          </button>
          <button className="btn" onClick={clearList} disabled={running || rows.length === 0}>
            <IconTrash />
            {t.app.clearList}
          </button>
        </div>
        <div className="tb-right">
          {running ? (
            <button className="btn danger" onClick={doCancel}>
              <IconStop />
              {t.app.cancel}
            </button>
          ) : (
            <button className="btn primary" onClick={startRun} disabled={rows.length === 0}>
              {settings.dryRun ? <IconPlan /> : <IconPlay />}
              {settings.dryRun ? t.app.planRun : t.app.start}
            </button>
          )}
        </div>
      </div>
      </div>

      {/* ---------- 拖放区 ---------- */}
      <div
        className={
          "dropzone" + (dragOver ? " drop-active" : "") + (rows.length > 0 ? " compact" : "")
        }
      >
        {rows.length === 0 ? (
          <>
            <div className="dz-icon" aria-hidden="true">
              <IconConvert size={28} />
            </div>
            <div className="dz-title">{t.app.dropTitle}</div>
            <div className="dz-sub">{t.app.dropSub}</div>
          </>
        ) : (
          <div className="dz-inline">
            <span>{t.app.imported(total)}</span>
            <span className="dot">·</span>
            <span>{t.app.dropMore}</span>
          </div>
        )}
      </div>

      {/* ---------- 文件列表 ---------- */}
      <div className="panel">
      <div className="listhead">
        <div className="filters">
          {FILTERS.map((f) => (
            <button
              key={f}
              className={"fchip" + (filter === f ? " on" : "")}
              onClick={() => {
                setFilter(f);
                // 筛选后结果集骤减，若残留旧 scrollTop，虚拟化会算出 start > end
                // → 列表全空；且内层高度可能小于视口（**无滚动条**）→ 永远等不到
                // onScroll 自愈。与 useBatch 的 importPaths 同款复位。
                setScrollTop(0);
                if (viewRef.current) viewRef.current.scrollTop = 0;
              }}
            >
              {filterLabel(f)}
              <span className="fnum">{filterCounts[f]}</span>
            </button>
          ))}
        </div>
        {filtered.length !== rows.length && (
          <span className="filter-note">{t.app.filtered(filtered.length, rows.length)}</span>
        )}
      </div>

      {/* ---------- 表头 ---------- */}
      <div className="grid-head">
        <span>{t.app.colStatus}</span>
        <span>{t.app.colFile}</span>
        <span>{t.app.colOutput}</span>
        <span className="ta-c">{t.app.colAction}</span>
      </div>

      {/* ---------- 虚拟滚动列表 ---------- */}
      <div
        className="vlist"
        ref={viewRef}
        onScroll={(e) => setScrollTop((e.target as HTMLDivElement).scrollTop)}
      >
        {filtered.length === 0 ? (
          <div className="empty">
            {rows.length === 0 ? t.app.noFiles : t.app.emptyFiltered(filterLabel(filter))}
          </div>
        ) : (
          <div style={{ height: filtered.length * ROW_H, position: "relative" }}>
            <div style={{ transform: `translateY(${start * ROW_H}px)` }}>
              {visible.map((r) => {
                const meta = STATUS_META[r.status] ?? STATUS_META.pending;
                return (
                  <div className="grid-row" key={r.source} style={{ height: ROW_H }}>
                    <span className={"st " + meta.cls}>
                      <span className="st-ico">{meta.icon}</span>
                      {t.status[r.status]}
                    </span>
                    <span className="fp" title={r.source}>
                      {fileName(r.source)}
                    </span>
                    <span
                      className="op"
                      title={r.reason ?? r.output ?? ""}
                    >
                      {r.status === "failed" ? (
                        <span className="reason">{r.reason ?? t.app.unknownError}</span>
                      ) : (
                        <span className="mono">{r.output ? relOutput(r.output) : "—"}</span>
                      )}
                    </span>
                    <span className="ta-c">
                      <button
                        className="btn-mini"
                        onClick={() => removeRow(r.source)}
                        disabled={running}
                        title={t.app.removeTip}
                      >
                        {t.app.remove}
                      </button>
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>
      </div>

      {/* ---------- 底部进度与汇总 ---------- */}
      {/* running 仅用于驱动进度条流动动画（纯展示，不参与业务判断） */}
      <div className={"footer" + (running ? " running" : "")}>
        <div className="progress">
          <div
            className={"progress-fill" + (counts.failed > 0 ? " has-fail" : "")}
            style={{ width: `${pct}%` }}
          />
        </div>
        <div className="stats">
          <span className="s-ok">✓ {counts.ok}</span>
          <span className="s-skipped">⏭ {counts.skipped}</span>
          <span className="s-failed">✕ {counts.failed}</span>
          {counts.cancelled > 0 && <span className="s-cancelled">⏸ {counts.cancelled}</span>}
          <span className="sep">|</span>
          <span className="muted">
            {doneCount} / {total} · {pct}%
          </span>
          <span className="sep">|</span>
          <span className="muted">{formatDuration(finishedMs)}</span>
          {summary && summary.planned > 0 && (
            <span className="muted">{t.app.plannedBadge(summary.planned)}</span>
          )}
          {summary?.isCancelled && <span className="badge-cancel">{t.status.cancelled}</span>}
        </div>
        <button
          className="btn sm"
          onClick={exportFailures}
          disabled={counts.failed === 0}
          title={counts.failed === 0 ? t.app.noFailures : t.app.exportTip}
        >
          <IconDownload />
          {t.app.exportFailures}
        </button>
      </div>

        </>
      )}

      </ErrorBoundary>

      {/* ---------- 媒体库（P1：概览/音乐库/艺术家/专辑/媒体源） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "media" && (
        <div className="library-main">
          {mediaTab === "home" && (
            <MediaHome
              goSources={() => setMediaTab("sources")}
              onNavigate={(tab) => setMediaTab(tab)}
              onPlay={player.playTracks}
              onQueue={player.queueAppend} onPlayNext={player.playNext}
            />
          )}
          {mediaTab === "library" && (
            <LibraryPage onPlay={player.playTracks} onQueue={player.queueAppend} onPlayNext={player.playNext} />
          )}
          {mediaTab === "artists" && (
            <ArtistsPage
              onPlay={player.playTracks}
              focusId={focusId?.kind === "artist" ? focusId.id : null}
              onQueue={player.queueAppend} onPlayNext={player.playNext}
            />
          )}
          {mediaTab === "albums" && (
            <AlbumsPage
              onPlay={player.playTracks}
              focusId={focusId?.kind === "album" ? focusId.id : null}
              onQueue={player.queueAppend} onPlayNext={player.playNext}
            />
          )}
          {mediaTab === "playlists" && (
            <PlaylistsPage
              onPlay={player.playTracks}
              focusId={focusId?.kind === "playlist" ? focusId.id : null}
              onQueue={player.queueAppend} onPlayNext={player.playNext}
            />
          )}
          {mediaTab === "favorites" && (
            <FavoritesPage onPlay={player.playTracks} onQueue={player.queueAppend} onPlayNext={player.playNext} />
          )}
          {mediaTab === "history" && (
            <HistoryPage onPlay={player.playTracks} onQueue={player.queueAppend} onPlayNext={player.playNext} />
          )}
          {mediaTab === "stats" && (
            <StatsPage onPlay={player.playTracks} onQueue={player.queueAppend} onPlayNext={player.playNext} />
          )}
          {mediaTab === "sources" && <SourcesPage />}
        </div>
      )}
      </ErrorBoundary>

      {/* ---------- 工具箱治理面板（P4：二级导航在左侧栏「工具箱」分组） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "toolbox" && (
        <div className="library-main">
          {toolboxTab === "scan" && <ScanPanel hideCollapse />}
          {toolboxTab === "dedupe" && <DedupePanel hideCollapse />}
          {/* P1：整理 / 清洗 / 回收站还原——后端能力已就绪，此前无 UI 入口 */}
          {toolboxTab === "organize" && <OrganizePanel />}
          {toolboxTab === "clean" && <CleanPanel />}
          {toolboxTab === "trash" && <TrashPanel />}
          {/* P4：CUE 分轨（核心能力已有，此处只是入口） */}
          {toolboxTab === "cue" && <CuePanel />}
        </div>
      )}
      </ErrorBoundary>

      {/* ---------- 插件面板（X37：零请求） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "toolbox" && toolboxTab === "plugins" && <PluginPanel />}
      </ErrorBoundary>

      {/* ---------- 设置（转换参数集中区） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "settings" && (
        <div className="panel">
          <div className="panel-head">
            <h2>{t.app.navSettings}</h2>
          </div>
      {/* ---------- 设置区 ---------- */}
      <div className="settings">
        {/* 保存位置：渐进披露（借鉴竞品，选「自定义目录」才展开路径输入） */}
        <div className="row">
          <label className="lbl">{t.app.saveTo}</label>
          <div className="radios">
            <label className="radio">
              <input
                type="radio"
                name="saveto"
                checked={settings.saveTo === "source"}
                onChange={() => patch({ saveTo: "source" })}
                disabled={running}
              />
              <span>{t.app.saveSource}</span>
            </label>
            <label className="radio">
              <input
                type="radio"
                name="saveto"
                checked={settings.saveTo === "custom"}
                onChange={() => patch({ saveTo: "custom" })}
                disabled={running}
              />
              <span>{t.app.saveCustom}</span>
            </label>
            {settings.saveTo === "custom" && (
              <div className="outdir">
                <input
                  className="val"
                  value={settings.outDir}
                  onChange={(e) => patch({ outDir: e.target.value })}
                  placeholder={t.app.outDirPlaceholder}
                  disabled={running}
                />
                <button className="btn sm" onClick={browseOutDir} disabled={running}>
                  {t.app.browse}
                </button>
              </div>
            )}
          </div>
        </div>

        {/* 命名模板 + 实时预览 */}
        <div className="row">
          <label className="lbl">{t.app.namingTemplate}</label>
          <div className="tpl">
            <input
              className="val mono"
              value={settings.template}
              onChange={(e) => patch({ template: e.target.value })}
              disabled={running}
              spellCheck={false}
            />
            <div className="tpl-help">
              {"{title} {artist} {album} {track:02d} {format}"} · <code>/</code>{" "}
              {t.app.tplSubdir}
            </div>
            {preview.length > 0 && (
              <div className="preview">
                {preview.map((p, i) => (
                  <div key={i} className="preview-line">
                    <span className="preview-label">{i === 0 ? t.app.sample : t.app.noMeta}</span>
                    {p}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* 其它选项 */}
        <div className="row">
          <label className="lbl">{t.app.options}</label>
          <div className="opts">
            <label className="check">
              <input
                type="checkbox"
                checked={settings.skipExisting}
                onChange={(e) => patch({ skipExisting: e.target.checked })}
                disabled={running}
              />
              <span>{t.app.skipExisting}</span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={settings.recursive}
                onChange={(e) => patch({ recursive: e.target.checked })}
                disabled={running}
              />
              <span>{t.app.recursive}</span>
            </label>
            {/* P6：在线元数据开关（默认关；开启后专辑页「补全封面」可用） */}
            <label className="check" title={t.app.onlineMetaHint}>
              <input
                type="checkbox"
                checked={settings.onlineMeta}
                onChange={(e) => patch({ onlineMeta: e.target.checked })}
              />
              <span>{t.app.onlineMeta}</span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={settings.dryRun}
                onChange={(e) => patch({ dryRun: e.target.checked })}
                disabled={running}
              />
              <span>{t.app.dryRun}</span>
            </label>
            <label className="check">
              <span className="nowrap">{t.app.concurrency}</span>
              <input
                className="num"
                type="number"
                min={1}
                max={16}
                value={settings.jobs}
                onChange={(e) =>
                  patch({
                    jobs: Math.min(16, Math.max(1, Math.round(Number(e.target.value) || 4))),
                  })
                }
                disabled={running}
              />
            </label>
          </div>
        </div>
      </div>

      {/* P5：更新区块（唯一网络行为——检查/安装/重启） */}
      <UpdateSection />

        </div>
      )}

      </ErrorBoundary>

      {/* v3 审计 §3.2：服务端信息卡同样纳入错误边界——它会请求后端（/version、/wizard/status），
          属"可能失败的渲染"。此前落在所有边界之外：一旦异常即整页白屏（ErrorBoundary 的存在意义被绕过）。 */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
        <ServerInfoCard />
      </ErrorBoundary>

      </Suspense>
      </main>

      {/* P2：播放底栏（常驻；.main 内部滚动、底栏固定） */}
      <PlayerBar player={player} />

      <div className="legal">{t.app.legal}</div>
      </div>
      {/* /main-wrap */}

      {/* P6.14 全局搜索面板（顶栏按钮 / Ctrl+K 唤起） */}
      {searchOpen && (
        <SearchPalette
          onClose={() => setSearchOpen(false)}
          onPlay={player.playTracks}
          onOpen={openSearchTarget}
        />
      )}

      {/* P5 首启引导：首次运行显示三步说明（localStorage 标记） */}
      <WelcomeGuide
        onGo={() => {
          setView("media");
          setMediaTab("sources");
        }}
      />

      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
