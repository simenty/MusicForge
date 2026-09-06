import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  cancelBatch,
  collectFiles,
  previewTemplate,
  saveFailures,
  selectDirectory,
  selectNcmFiles,
  onBatchDone,
  onBatchFile,
  onDragDropEvent,
  planBatch,
  startBatch,
  type BatchSummary,
  type FileResult,
  type PlannedItem,
  type UnlistenFn,
} from "./api";
import { loadSettings, saveSettings, type Settings } from "./settings";

/**
 * 行状态。
 * 后端批处理只在每个文件**到达终态**时推送一次事件（`batch-file`），
 * 没有「开始处理」事件 —— 所以这里不做「处理中」这个假状态：
 * 与其靠并发数去猜哪几个文件在跑，不如老老实实显示「等待」+ 底部进度。
 */
type RowStatus = "pending" | FileResult["status"];

interface Row {
  source: string;
  /** G3：目录输入的根（散文件为 null）——start_batch 随行回传，保留源目录树 */
  root: string | null;
  status: RowStatus;
  output: string | null;
  reason: string | null;
}

type FilterKey = "all" | "pending" | "ok" | "skipped" | "failed" | "cancelled";

const STATUS_META: Record<RowStatus, { text: string; cls: string; icon: string }> = {
  pending: { text: "等待", cls: "s-pending", icon: "●" },
  ok: { text: "完成", cls: "s-ok", icon: "●" },
  skipped: { text: "跳过", cls: "s-skipped", icon: "●" },
  failed: { text: "失败", cls: "s-failed", icon: "●" },
  cancelled: { text: "已取消", cls: "s-cancelled", icon: "●" },
};

const FILTERS: { key: FilterKey; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "pending", label: "等待" },
  { key: "ok", label: "完成" },
  { key: "skipped", label: "跳过" },
  { key: "failed", label: "失败" },
  { key: "cancelled", label: "已取消" },
];

/** 虚拟滚动行高（px）。与 CSS 中 .grid-row 的 height 必须一致 */
const ROW_H = 36;
/** 缓冲区行数：上下各多渲染一些，避免快速滚动时白屏 */
const OVERSCAN = 8;
/** 进度事件合并刷新间隔（ms）——5701 个文件若逐条 setState 会拖垮渲染 */
const FLUSH_MS = 100;

export default function App() {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [rows, setRows] = useState<Row[]>([]);
  const [summary, setSummary] = useState<BatchSummary | null>(null);
  /** 计划预览（dry-run）：planBatch 的结果，展示在列表上方的预览面板 */
  const [plannedRows, setPlannedRows] = useState<PlannedItem[] | null>(null);
  const [running, setRunning] = useState(false);
  const [filter, setFilter] = useState<FilterKey>("all");
  const [dragOver, setDragOver] = useState(false);
  const [preview, setPreview] = useState<string[]>([]);
  const [toast, setToast] = useState<string | null>(null);
  const [elapsedMs, setElapsedMs] = useState(0);
  /** 致命错误（事件订阅失败等）——置顶横幅，绝不能静默 */
  const [fatal, setFatal] = useState<string | null>(null);

  // ---- 虚拟滚动 ----
  const [scrollTop, setScrollTop] = useState(0);
  const [viewH, setViewH] = useState(320);
  const viewRef = useRef<HTMLDivElement | null>(null);

  // ---- 批处理事件缓冲 ----
  // G3 后 Row 增加了 root（事件载荷不含 root）——缓冲只存状态补丁，落行时合并
  type RowPatch = Pick<Row, "source" | "status" | "output" | "reason">;
  const pendingRef = useRef<RowPatch[]>([]);
  const indexRef = useRef(new Map<string, number>());

  // 事件订阅只在挂载时做一次；下面两个 ref 让回调始终读到最新的值，
  // 避免闭包捕获到挂载时的旧状态（拖拽导入时 running 会失效）
  const recursiveRef = useRef(settings.recursive);
  const importRef = useRef<(paths: string[], recursive: boolean) => Promise<void>>(
    async () => {}
  );

  // 设置持久化（防抖 300ms，避免连续输入时频繁写入）
  useEffect(() => {
    const t = setTimeout(() => saveSettings(settings), 300);
    return () => clearTimeout(t);
  }, [settings]);

  const patch = useCallback((p: Partial<Settings>) => {
    setSettings((s) => ({ ...s, ...p }));
  }, []);

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    window.setTimeout(() => setToast((cur) => (cur === msg ? null : cur)), 3600);
  }, []);

  // ---- 模板实时预览（debounce 300ms）----
  useEffect(() => {
    const t = setTimeout(() => {
      if (settings.template.trim()) {
        previewTemplate(settings.template).then(setPreview).catch(() => setPreview([]));
      } else {
        setPreview([]);
      }
    }, 300);
    return () => clearTimeout(t);
  }, [settings.template]);

  // ---- 导入文件（追加 + 去重）----
  // 工具条提示「可继续添加」、拖放区提示「拖动更多文件」，因此语义必须是**追加**
  // 而非替换：用户从多个文件夹收集是常态，替换会静默丢掉前一批。
  const importPaths = useCallback(
    async (paths: string[], recursive: boolean) => {
      if (running || paths.length === 0) return;
      const ncm = await collectFiles(paths, recursive);
      if (ncm.length === 0) {
        showToast(`这些路径里没有找到 .ncm 文件（共检查 ${paths.length} 个路径）`);
        return;
      }
      // 在事件处理里基于当前 rows 计算 next（不在 setState 更新器里改 ref）
      const known = new Set(rows.map((r) => r.source));
      const fresh = ncm.filter((f) => !known.has(f.path));
      if (fresh.length === 0) {
        showToast(`所选 ${ncm.length} 个文件都已在列表中`);
        return;
      }
      const next = [
        ...rows,
        ...fresh.map((f) => ({
          source: f.path,
          root: f.root,
          status: "pending" as const,
          output: null,
          reason: null,
        })),
      ];
      indexRef.current = new Map(next.map((r, i) => [r.source, i]));
      setRows(next);
      setSummary(null);
      setElapsedMs(0);
      setScrollTop(0);
      if (viewRef.current) viewRef.current.scrollTop = 0;
      const skippedDup = ncm.length - fresh.length;
      const nonNcm = paths.length - ncm.length;
      const parts: string[] = [`已添加 ${fresh.length} 个 .ncm 文件`];
      if (skippedDup > 0) parts.push(`去重 ${skippedDup} 个`);
      if (nonNcm > 0) parts.push(`忽略 ${nonNcm} 个非 ncm 路径`);
      showToast(parts.join(" · ") + `（列表共 ${next.length} 个）`);
    },
    [running, rows, showToast]
  );

  // ref 同步：让只订阅一次的事件回调读到最新值
  recursiveRef.current = settings.recursive;
  importRef.current = importPaths;

  // ---- 事件订阅 ----
  // ⚠ 注册失败绝不能静默吞掉：曾经 listen() 被 ACL 拒绝（event.listen not allowed）
  //   而 void p.then() 把错误吞了，表现为后台正常转换、UI 永远停在 0/N 且无任何报错。
  useEffect(() => {
    let cancelled = false;
    const cleanups: UnlistenFn[] = [];

    const register = (p: Promise<UnlistenFn>, name: string) => {
      p.then((un) => {
        if (cancelled) un();
        else cleanups.push(un);
      }).catch((e) => {
        if (!cancelled) {
          setFatal(
            `事件订阅失败（${name}）：${String(e)}\n` +
              "进度将无法显示。请截图此提示并反馈。"
          );
        }
      });
    };

    register(
      onDragDropEvent((ev) => {
        if (ev.type === "enter" || ev.type === "over") {
          setDragOver(true);
        } else if (ev.type === "leave") {
          setDragOver(false);
        } else if (ev.type === "drop") {
          setDragOver(false);
          // QA 第二轮：拖放导入的失败此前是 unhandled rejection，
          // 表现为「拖进来什么都没发生」。
          void importRef.current(ev.paths, recursiveRef.current).catch((e) =>
            setFatal(`导入失败：${String(e)}`)
          );
        }
      }),
      "拖拽导入"
    );
    register(
      onBatchFile((r) => {
        // 只入缓冲，不 setState —— 由下面的定时器批量刷新
        pendingRef.current.push({
          source: r.source,
          status: r.status,
          output: r.output,
          reason: r.reason,
        });
      }),
      "进度"
    );
    register(
      onBatchDone((s) => {
        setSummary(s);
        setRunning(false);
      }),
      "汇总"
    );

    return () => {
      cancelled = true;
      cleanups.forEach((un) => un());
    };
    // 只在挂载时订阅一次；importPaths / recursive 均通过 ref 取最新值
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 合并刷新：O(batch) 增量更新，而不是每个文件一次全量重渲染。
  // 计数不在这里维护 —— 由 rows 派生（见下方 useMemo），追加导入时不会算错。
  useEffect(() => {
    const id = window.setInterval(() => {
      const batch = pendingRef.current;
      if (batch.length === 0) return;
      pendingRef.current = [];
      setRows((prev) => {
        const next = prev.slice();
        for (const u of batch) {
          const i = indexRef.current.get(u.source);
          if (i === undefined) continue;
          // 只接受「等待 → 终态」的首次跃迁，忽略重复事件；合并而非整行替换（root 保留）
          if (next[i].status !== "pending") continue;
          next[i] = { ...next[i], status: u.status, output: u.output, reason: u.reason };
        }
        return next;
      });
    }, FLUSH_MS);
    return () => clearInterval(id);
  }, []);

  // ---- 计时 ----
  useEffect(() => {
    if (!running) return;
    const t0 = Date.now();
    const id = window.setInterval(() => setElapsedMs(Date.now() - t0), 250);
    return () => clearInterval(id);
  }, [running]);

  // ---- 视口高度测量（虚拟滚动需要）----
  useEffect(() => {
    const el = viewRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewH(el.clientHeight));
    ro.observe(el);
    setViewH(el.clientHeight);
    return () => ro.disconnect();
  }, []);

  // ---- 筛选 + 虚拟窗口 ----
  const filtered = useMemo(
    () => (filter === "all" ? rows : rows.filter((r) => r.status === filter)),
    [rows, filter]
  );

  const filterCounts = useMemo(() => {
    const c: Record<RowStatus | "all", number> = {
      all: rows.length,
      pending: 0,
      ok: 0,
      skipped: 0,
      failed: 0,
      cancelled: 0,
    };
    for (const r of rows) c[r.status]++;
    return c;
  }, [rows]);

  // 计数从 rows 派生（而非增量累加）：追加导入/重跑/清除时天然正确，
  // 不存在「计数与列表脱节」这类状态同步 bug。O(n) 每次 flush 可忽略。
  const counts = filterCounts;

  // 输入行变化后，旧的计划预览即失效
  useEffect(() => {
    setPlannedRows(null);
  }, [rows.length]);

  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(filtered.length, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN);
  const visible = filtered.slice(start, end);

  // ---- 动作 ----
  // QA 第二轮：所有 async 动作都必须有拒绝分支。此前 `onClick={addFiles}` 之类的
  // 写法把 invoke 失败变成 unhandled rejection —— 按钮点了没反应、控制台一行报错，
  // 用户完全不知道发生了什么。统一经 `guard` 兜住并提示。
  const guard = useCallback(
    (label: string, p: Promise<unknown>) => {
      p.catch((e) => showToast(`${label}失败：${String(e)}`));
    },
    [showToast]
  );

  const addFiles = useCallback(() => {
    guard(
      "添加文件",
      selectNcmFiles(settings.outDir || undefined).then(async (picked) => {
        if (picked.length) await importPaths(picked, false);
      })
    );
  }, [guard, importPaths, settings.outDir]);

  const addFolder = useCallback(() => {
    guard(
      "添加目录",
      selectDirectory(settings.outDir || null, "选择包含 .ncm 的文件夹").then(async (dir) => {
        if (dir) await importPaths([dir], settings.recursive);
      })
    );
  }, [guard, importPaths, settings.outDir, settings.recursive]);

  const browseOutDir = useCallback(() => {
    guard(
      "选择输出目录",
      selectDirectory(settings.outDir || null, "选择输出目录").then((dir) => {
        if (dir) patch({ outDir: dir });
      })
    );
  }, [guard, patch, settings.outDir]);

  const clearList = () => {
    if (running) return;
    pendingRef.current = [];
    indexRef.current = new Map();
    setRows([]);
    setSummary(null);
    setElapsedMs(0);
  };

  const removeRow = (source: string) => {
    if (running) return;
    // 直接在事件处理里算出 next，不在 setState 更新器里改 ref
    // （StrictMode 下更新器会被调用两次，副作用放进去是坏味道）
    const next = rows.filter((r) => r.source !== source);
    indexRef.current = new Map(next.map((r, i) => [r.source, i]));
    setRows(next);
  };

  const startRun = async () => {
    if (running) return;
    if (rows.length === 0) {
      showToast("文件列表为空，请先添加文件，或把 .ncm 文件/文件夹拖进来。");
      return;
    }
    if (settings.saveTo === "custom" && !settings.outDir.trim()) {
      showToast("已选择「自定义目录」，请先指定输出目录。");
      return;
    }
    // dry-run：不进入执行流，改走计划预览（plan_batch 只规划不落盘）
    if (settings.dryRun) {
      try {
        const items = await planBatch({
          inputs: rows.map((r) => ({ path: r.source, root: r.root })),
          outDir: settings.saveTo === "custom" ? settings.outDir.trim() : null,
          template: settings.template,
          skipExisting: settings.skipExisting,
          recursive: settings.recursive,
          jobs: settings.jobs,
          dryRun: true,
        });
        const failed = items.filter((i) => i.error !== null).length;
        setPlannedRows(items);
        showToast(
          `已生成计划：${items.length - failed} 项可执行` +
            (failed > 0 ? `，${failed} 项失败（见预览面板）` : "（未改动任何文件）")
        );
      } catch (e) {
        showToast(String(e));
      }
      return;
    }

    // 允许重跑：先把所有行重置为等待。
    // （竞品在这里直接拒绝、逼用户重新导入，属于没必要的限制）
    pendingRef.current = [];
    setPlannedRows(null);
    setRows((prev) =>
      prev.map((r) => ({ ...r, status: "pending" as const, output: null, reason: null }))
    );
    setSummary(null);
    setElapsedMs(0);
    setRunning(true);
    try {
      await startBatch({
        inputs: rows.map((r) => ({ path: r.source, root: r.root })),
        outDir: settings.saveTo === "custom" ? settings.outDir.trim() : null,
        template: settings.template,
        skipExisting: settings.skipExisting,
        recursive: settings.recursive,
        jobs: settings.jobs,
        dryRun: settings.dryRun,
      });
    } catch (e) {
      setRunning(false);
      showToast(String(e));
    }
  };

  // QA 第二轮：`cancelBatch()` 若被拒绝，此前既没有提示也没有任何痕迹，
  // 用户点了「取消」却完全不知道请求有没有送到后端。
  const doCancel = useCallback(() => {
    guard(
      "取消",
      cancelBatch().then((ok) =>
        showToast(
          ok
            ? "已请求取消：未开始的文件会标记为「已取消」，正在处理的文件会跑完。"
            : "当前没有正在运行的转换任务，取消请求未生效。"
        )
      )
    );
  }, [guard, showToast]);

  const exportFailures = async () => {
    const failedRows = rows.filter((r) => r.status === "failed");
    if (failedRows.length === 0) return;
    try {
      const path = await saveFailures(
        failedRows.map((r) => ({ source: r.source, status: "failed", reason: r.reason }))
      );
      if (path) showToast(`失败清单已导出：${path}`);
    } catch (e) {
      showToast(`导出失败：${String(e)}`);
    }
  };

  const doneCount = counts.ok + counts.skipped + counts.failed + counts.cancelled;
  const total = rows.length;
  const pct = total > 0 ? Math.min(100, Math.round((doneCount / total) * 100)) : 0;
  const finishedMs = summary ? summary.durationMs : elapsedMs;

  return (
    <div className="window">
      {fatal && (
        <div className="fatal" onClick={() => setFatal(null)} title="点击关闭">
          <b>⚠ 应用初始化异常</b>
          <pre>{fatal}</pre>
        </div>
      )}
      {plannedRows && plannedRows.length > 0 && (
        <div className="plan-panel">
          <div className="plan-head">
            <b>
              计划预览（{plannedRows.length} 项 ·{" "}
              {plannedRows.filter((i) => i.error === null).length} 可执行 · 未改动任何文件）
            </b>
            <button className="btn sm" onClick={() => setPlannedRows(null)}>
              关闭
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
      {/* ---------- 标题栏 ---------- */}
      <div className="titlebar">
        <div className="brand">
          <span className="logo">▤</span>
          <strong>MusicForge</strong>
          <span className="sub">本地音乐格式转换</span>
        </div>
        <div className="chips">
          <span className="chip green">零网络 · 离线运行</span>
          <span className="chip">MIT</span>
          <span className="chip">v0.1.0</span>
        </div>
      </div>

      {/* ---------- 工具条 ---------- */}
      <div className="toolbar">
        <div className="tb-left">
          <button className="btn" onClick={addFiles} disabled={running}>
            <span className="ico">＋</span>添加文件
          </button>
          <button className="btn" onClick={addFolder} disabled={running}>
            <span className="ico">▣</span>添加目录
          </button>
          <button className="btn" onClick={clearList} disabled={running || rows.length === 0}>
            <span className="ico">⌫</span>清除列表
          </button>
        </div>
        <div className="tb-right">
          {running ? (
            <button className="btn danger" onClick={doCancel}>
              <span className="ico">■</span>取消
            </button>
          ) : (
            <button className="btn primary" onClick={startRun} disabled={rows.length === 0}>
              <span className="ico">{settings.dryRun ? "🗎" : "▶"}</span>
              {settings.dryRun ? "生成计划（不写文件）" : "开始转换"}
            </button>
          )}
        </div>
      </div>

      {/* ---------- 设置区 ---------- */}
      <div className="settings">
        {/* 保存位置：渐进披露（借鉴竞品，选「自定义目录」才展开路径输入） */}
        <div className="row">
          <label className="lbl">保存到</label>
          <div className="radios">
            <label className="radio">
              <input
                type="radio"
                name="saveto"
                checked={settings.saveTo === "source"}
                onChange={() => patch({ saveTo: "source" })}
                disabled={running}
              />
              <span>源文件所在目录</span>
            </label>
            <label className="radio">
              <input
                type="radio"
                name="saveto"
                checked={settings.saveTo === "custom"}
                onChange={() => patch({ saveTo: "custom" })}
                disabled={running}
              />
              <span>自定义目录</span>
            </label>
            {settings.saveTo === "custom" && (
              <div className="outdir">
                <input
                  className="val"
                  value={settings.outDir}
                  onChange={(e) => patch({ outDir: e.target.value })}
                  placeholder="点击右侧「浏览」选择输出目录"
                  disabled={running}
                />
                <button className="btn sm" onClick={browseOutDir} disabled={running}>
                  浏览
                </button>
              </div>
            )}
          </div>
        </div>

        {/* 命名模板 + 实时预览 */}
        <div className="row">
          <label className="lbl">命名模板</label>
          <div className="tpl">
            <input
              className="val mono"
              value={settings.template}
              onChange={(e) => patch({ template: e.target.value })}
              disabled={running}
              spellCheck={false}
            />
            <div className="tpl-help">
              {"{title} {artist} {album} {track:02d} {format}"} · <code>/</code> 产生子目录
            </div>
            {preview.length > 0 && (
              <div className="preview">
                {preview.map((p, i) => (
                  <div key={i} className="preview-line">
                    <span className="preview-label">{i === 0 ? "示例" : "无元数据"}</span>
                    {p}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* 其它选项 */}
        <div className="row">
          <label className="lbl">选项</label>
          <div className="opts">
            <label className="check">
              <input
                type="checkbox"
                checked={settings.skipExisting}
                onChange={(e) => patch({ skipExisting: e.target.checked })}
                disabled={running}
              />
              <span>跳过已存在（带完整性校验）</span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={settings.recursive}
                onChange={(e) => patch({ recursive: e.target.checked })}
                disabled={running}
              />
              <span>递归子目录（保留目录结构）</span>
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={settings.dryRun}
                onChange={(e) => patch({ dryRun: e.target.checked })}
                disabled={running}
              />
              <span>仅规划（dry-run，不写文件）</span>
            </label>
            <label className="check">
              <span className="nowrap">并发</span>
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

      {/* ---------- 拖放区 ---------- */}
      <div
        className={
          "dropzone" + (dragOver ? " drop-active" : "") + (rows.length > 0 ? " compact" : "")
        }
      >
        {rows.length === 0 ? (
          <>
            <div className="dz-icon" aria-hidden="true">
              ▤
            </div>
            <div className="dz-title">把 .ncm 文件或整个文件夹拖到这里</div>
            <div className="dz-sub">也可以用上方「添加文件 / 添加目录」按钮</div>
          </>
        ) : (
          <div className="dz-inline">
            <span>已导入 {total} 个文件</span>
            <span className="dot">·</span>
            <span>拖动更多文件到窗口可继续添加</span>
          </div>
        )}
      </div>

      {/* ---------- 筛选 ---------- */}
      <div className="listhead">
        <div className="filters">
          {FILTERS.map((f) => (
            <button
              key={f.key}
              className={"fchip" + (filter === f.key ? " on" : "")}
              onClick={() => setFilter(f.key)}
            >
              {f.label}
              <span className="fnum">{filterCounts[f.key]}</span>
            </button>
          ))}
        </div>
        {filtered.length !== rows.length && (
          <span className="filter-note">
            已筛选 {filtered.length} / {rows.length}
          </span>
        )}
      </div>

      {/* ---------- 表头 ---------- */}
      <div className="grid-head">
        <span>状态</span>
        <span>文件</span>
        <span>输出 / 失败原因</span>
        <span className="ta-c">操作</span>
      </div>

      {/* ---------- 虚拟滚动列表 ---------- */}
      <div
        className="vlist"
        ref={viewRef}
        onScroll={(e) => setScrollTop((e.target as HTMLDivElement).scrollTop)}
      >
        {filtered.length === 0 ? (
          <div className="empty">
            {rows.length === 0
              ? "还没有文件"
              : `没有「${FILTERS.find((f) => f.key === filter)?.label}」状态的记录`}
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
                      {meta.text}
                    </span>
                    <span className="fp" title={r.source}>
                      {fileName(r.source)}
                    </span>
                    <span
                      className="op"
                      title={r.reason ?? r.output ?? ""}
                    >
                      {r.status === "failed" ? (
                        <span className="reason">{r.reason ?? "未知错误"}</span>
                      ) : (
                        <span className="mono">{r.output ? relOutput(r.output) : "—"}</span>
                      )}
                    </span>
                    <span className="ta-c">
                      <button
                        className="btn-mini"
                        onClick={() => removeRow(r.source)}
                        disabled={running}
                        title="从列表移除（不会删除磁盘上的文件）"
                      >
                        移除
                      </button>
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
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
            <span className="muted">🗎 已规划 {summary.planned}（未落盘）</span>
          )}
          {summary?.isCancelled && <span className="badge-cancel">已取消</span>}
        </div>
        <button
          className="btn sm"
          onClick={exportFailures}
          disabled={counts.failed === 0}
          title={counts.failed === 0 ? "没有失败项" : "导出失败清单 CSV"}
        >
          导出失败清单
        </button>
      </div>

      <div className="legal">
        MusicForge 仅用于处理你已合法获得的文件的个人本地格式转换 · 不联网 · 不上传 · 不收集任何数据 ·
        MIT License
      </div>

      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}

/** 取路径最后一段（跨平台分隔符） */
function fileName(p: string): string {
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  return i >= 0 ? p.slice(i + 1) : p;
}

/** 输出路径只显示末两段，避免长路径撑破表格 */
function relOutput(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 2 ? norm : "…/" + parts.slice(-2).join("/");
}

function formatDuration(ms: number): string {
  if (ms <= 0) return "—";
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}.${Math.floor((ms % 1000) / 100)}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}m`;
}
