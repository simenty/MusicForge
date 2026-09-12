import { useCallback, useState } from "react";
import {
  selectDirectory,
  selectNcmFiles,
  serverToken,
  setServerToken,
  IS_DESKTOP,
} from "./api";
import { useLang, type Lang } from "./i18n";
import { useServerAuth } from "./useServerAuth";
import { useToast } from "./hooks/useToast";
import { useSettings } from "./hooks/useSettings";
import { useBatch, ROW_H, type FilterKey, type RowStatus } from "./hooks/useBatch";
import { fileName, relOutput, formatDuration } from "./lib/format";
import DedupePanel from "./DedupePanel";
import ScanPanel from "./ScanPanel";
import PluginPanel from "./PluginPanel";
import OrganizePanel from "./OrganizePanel";
import CleanPanel from "./CleanPanel";
import TrashPanel from "./TrashPanel";
import ErrorBoundary from "./ErrorBoundary";
import ServerInfoCard from "./ServerInfoCard";
import {
  IconConvert,
  IconCopy,
  IconDownload,
  IconFolder,
  IconLibrary,
  IconMenu,
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
} from "./icons";

/** 主分区（信息架构：转换 / 曲库 / 插件 / 设置） */
type ViewKey = "convert" | "library" | "plugins" | "settings";

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
  /** 当前主分区（默认「转换」——核心流程零跳转可达） */
  const [view, setView] = useState<ViewKey>("convert");
  /** 鉴权总开关（false = MUSICFORGE_AUTH=off：隐藏 token 框、显示状态徽标） */
  const { authEnabled } = useServerAuth();
  /** 窄屏抽屉（≤860px 时侧栏转为抽屉，由顶栏汉堡开关） */
  const [navOpen, setNavOpen] = useState(false);
  /** 曲库分区的二级菜单（左侧栏分组）：扫描 / 去重 / 整理 / 清洗 / 回收站 */
  const [libraryTab, setLibraryTab] = useState<
    "scan" | "dedupe" | "organize" | "clean" | "trash"
  >("scan");
  /** 顶栏标题：当前分区（曲库时附二级项名——布局吸收后导航在左侧栏，顶栏只显示位置） */
  const viewLabel =
    view === "convert"
      ? t.app.navConvert
      : view === "library"
        ? `${t.app.navLibrary} · ${
            libraryTab === "scan"
              ? t.library.tabScan
              : libraryTab === "dedupe"
                ? t.library.tabDedupe
                : libraryTab === "organize"
                  ? t.library.tabOrganize
                  : libraryTab === "clean"
                    ? t.library.tabClean
                    : t.library.tabTrash
          }`
        : view === "plugins"
          ? t.app.navPlugins
          : t.app.navSettings;
  // P8.2.5：fnOS 服务端形态的访问 token（HTTP 形态标题栏可见可改）
  const [serverTokenInput, setServerTokenInput] = useState<string>(serverToken());
  const [filter, setFilter] = useState<FilterKey>("all");

  const { toast, setToast, showToast } = useToast();
  const { settings, patch, preview } = useSettings();
  const b = useBatch({ t, settings, showToast, filter });
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
          <div className="nav-group">
            <button
              className={"nav-item" + (view === "convert" ? " on" : "")}
              onClick={() => {
                setView("convert");
                setNavOpen(false);
              }}
            >
              <IconConvert />
              <span>{t.app.navConvert}</span>
              {rows.length > 0 && <span className="badge">{rows.length}</span>}
            </button>
          </div>

          <div className="nav-sep" role="separator"></div>

          {/* 曲库 = 分组：二级项从分区内 subnav 提升到侧栏（少一层左侧嵌套） */}
          <div className="nav-group">
            <button
              className={"nav-item" + (view === "library" ? " on" : "")}
              onClick={() => {
                setView("library");
                setNavOpen(false);
              }}
            >
              <IconLibrary />
              <span>{t.app.navLibrary}</span>
            </button>
            <div className="nav-sub">
              <button
                className={"nav-item sub" + (view === "library" && libraryTab === "scan" ? " on" : "")}
                onClick={() => {
                  setView("library");
                  setLibraryTab("scan");
                  setNavOpen(false);
                }}
              >
                <IconScan />
                <span>{t.library.tabScan}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "library" && libraryTab === "dedupe" ? " on" : "")}
                onClick={() => {
                  setView("library");
                  setLibraryTab("dedupe");
                  setNavOpen(false);
                }}
              >
                <IconCopy />
                <span>{t.library.tabDedupe}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "library" && libraryTab === "organize" ? " on" : "")}
                onClick={() => {
                  setView("library");
                  setLibraryTab("organize");
                  setNavOpen(false);
                }}
              >
                <IconOrganize />
                <span>{t.library.tabOrganize}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "library" && libraryTab === "clean" ? " on" : "")}
                onClick={() => {
                  setView("library");
                  setLibraryTab("clean");
                  setNavOpen(false);
                }}
              >
                <IconClean />
                <span>{t.library.tabClean}</span>
              </button>
              <button
                className={"nav-item sub" + (view === "library" && libraryTab === "trash" ? " on" : "")}
                onClick={() => {
                  setView("library");
                  setLibraryTab("trash");
                  setNavOpen(false);
                }}
              >
                <IconRestore />
                <span>{t.library.tabTrash}</span>
              </button>
            </div>
          </div>

          <div className="nav-sep" role="separator"></div>

          <div className="nav-group">
            <button
              className={"nav-item" + (view === "plugins" ? " on" : "")}
              onClick={() => {
                setView("plugins");
                setNavOpen(false);
              }}
            >
              <IconPlugin />
              <span>{t.app.navPlugins}</span>
            </button>
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
            <span className="chip">v0.9.0</span>
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
      {/* P0-2：分区级错误边界——任一分区渲染异常只降级该分区，不带走整个应用 */}
      <ErrorBoundary
        title={t.app.errorTitle}
        hint={t.app.errorHint}
        retry={t.app.errorRetry}
      >
      {view === "convert" && (
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
              onClick={() => setFilter(f)}
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

      {/* ---------- 曲库治理（二级导航已提升至左侧栏「曲库」分组） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "library" && (
        <div className="library-main">
          {libraryTab === "scan" && <ScanPanel hideCollapse />}
          {libraryTab === "dedupe" && <DedupePanel hideCollapse />}
          {/* P1：整理 / 清洗 / 回收站还原——后端能力已就绪，此前无 UI 入口 */}
          {libraryTab === "organize" && <OrganizePanel />}
          {libraryTab === "clean" && <CleanPanel />}
          {libraryTab === "trash" && <TrashPanel />}
        </div>
      )}
      </ErrorBoundary>

      {/* ---------- 插件面板（X37：零请求） ---------- */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {view === "plugins" && <PluginPanel />}
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

        </div>
      )}

      </ErrorBoundary>

      {/* v3 审计 §3.2：服务端信息卡同样纳入错误边界——它会请求后端（/version、/wizard/status），
          属"可能失败的渲染"。此前落在所有边界之外：一旦异常即整页白屏（ErrorBoundary 的存在意义被绕过）。 */}
      <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
        <ServerInfoCard />
      </ErrorBoundary>

      </main>

      <div className="legal">{t.app.legal}</div>
      </div>
      {/* /main-wrap */}

      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
