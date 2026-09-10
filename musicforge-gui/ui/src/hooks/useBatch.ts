/**
 * 批处理编排（P1-3 拆分：从 App.tsx 原样迁出，**行为逐行对齐**）。
 *
 * 职责：文件行状态 + 事件订阅 + 合并刷新 + 导入/执行/取消/导出 + 虚拟滚动窗口。
 * 不负责：导航、筛选（filter 由调用方持有并传入）、渲染。
 *
 * 迁移纪律：本文件是**搬迁**而非重写——所有注释中标注的修复（AUD-1/AUD-3、
 * QA 第二轮、G3 等）一并带来，不得在迁移中顺手改语义。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  cancelBatch,
  collectFiles,
  formatMigrate,
  onBatchDone,
  onBatchFile,
  onDragDropEvent,
  planBatch,
  runBatchHttp,
  saveFailures,
  startBatch,
  IS_DESKTOP,
  type BatchSummary,
  type FileResult,
  type PlannedItem,
  type UnlistenFn,
} from "../api";
import { useLang } from "../i18n";
import { extOf, percent } from "../lib/format";
import type { Settings } from "../settings";

type Dict = ReturnType<typeof useLang>["t"];

/**
 * 行状态。
 * 后端批处理只在每个文件**到达终态**时推送一次事件（`batch-file`），
 * 没有「开始处理」事件 —— 所以这里不做「处理中」这个假状态：
 * 与其靠并发数去猜哪几个文件在跑，不如老老实实显示「等待」+ 底部进度。
 */
export type RowStatus = "pending" | FileResult["status"];

export interface Row {
  source: string;
  /** G3：目录输入的根（散文件为 null）——start_batch 随行回传，保留源目录树 */
  root: string | null;
  status: RowStatus;
  output: string | null;
  reason: string | null;
}

export type FilterKey = "all" | "pending" | "ok" | "skipped" | "failed" | "cancelled";

/** 虚拟滚动行高（px）。与 CSS 中 .grid-row 的 height 必须一致 */
export const ROW_H = 36;
/** 缓冲区行数：上下各多渲染一些，避免快速滚动时白屏 */
const OVERSCAN = 8;
/** 进度事件合并刷新间隔（ms）——5701 个文件若逐条 setState 会拖垮渲染 */
const FLUSH_MS = 100;

/** P8.2.6：fnOS 形态按扩展名分派迁移插件（.ncm 走内置批处理） */
const PLUGIN_BY_EXT: Record<string, string> = {
  kwm: "kwm-migration",
  qmc0: "qmc-migration",
  qmc3: "qmc-migration",
  qmcflac: "qmc-migration",
  qmc2: "qmc-migration",
  qmcogg: "qmc-migration",
  qmcmp3: "qmc-migration",
  qmflac: "qmc-migration",
  bkcflac: "qmc-migration",
  bkcmp3: "qmc-migration",
  mflac: "qmc-migration",
  mflac0: "qmc-migration",
  mflac1: "qmc-migration",
  mgg: "qmc-migration",
  mgg0: "qmc-migration",
  mgg1: "qmc-migration",
  mggl: "qmc-migration",
};

interface UseBatchArgs {
  t: Dict;
  settings: Settings;
  showToast: (msg: string) => void;
  /** 当前筛选（由调用方持有——筛选是视图状态） */
  filter: FilterKey;
}

export function useBatch({ t, settings, showToast, filter }: UseBatchArgs) {
  const [rows, setRows] = useState<Row[]>([]);
  const [summary, setSummary] = useState<BatchSummary | null>(null);
  /** 计划预览（dry-run）：planBatch 的结果 */
  const [plannedRows, setPlannedRows] = useState<PlannedItem[] | null>(null);
  const [running, setRunning] = useState(false);
  const [dragOver, setDragOver] = useState(false);
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

  // 事件订阅只在挂载时做一次；下面两个 ref 让回调始终读到最新的值
  const recursiveRef = useRef(settings.recursive);
  const importRef = useRef<(paths: string[], recursive: boolean) => Promise<void>>(
    async () => {}
  );

  // ---- 导入文件（追加 + 去重）----
  // 语义必须是**追加**而非替换：用户从多个文件夹收集是常态，替换会静默丢前一批。
  const importPaths = useCallback(
    async (paths: string[], recursive: boolean) => {
      if (running || paths.length === 0) return;
      const ncm = await collectFiles(paths, recursive);
      if (ncm.length === 0) {
        showToast(t.app.noNcmFound(paths.length));
        return;
      }
      const known = new Set(rows.map((r) => r.source));
      const fresh = ncm.filter((f) => !known.has(f.path));
      if (fresh.length === 0) {
        showToast(t.app.alreadyInList(ncm.length));
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
      const parts: string[] = [t.app.added(fresh.length)];
      if (skippedDup > 0) parts.push(t.app.deduped(skippedDup));
      if (nonNcm > 0) parts.push(t.app.ignoredNonNcm(nonNcm));
      showToast(parts.join(" · ") + t.app.listTotal(next.length));
    },
    [running, rows, showToast, t]
  );

  // ref 同步：让只订阅一次的事件回调读到最新值
  recursiveRef.current = settings.recursive;
  importRef.current = importPaths;

  // ---- 事件订阅 ----
  // ⚠ 注册失败绝不能静默吞掉：曾经 listen() 被 ACL 拒绝，而 void p.then() 把
  //   错误吞了，表现为后台正常转换、UI 永远停在 0/N 且无任何报错。
  useEffect(() => {
    let cancelled = false;
    const cleanups: UnlistenFn[] = [];

    const register = (p: Promise<UnlistenFn>, name: string) => {
      p.then((un) => {
        if (cancelled) un();
        else cleanups.push(un);
      }).catch((e) => {
        if (!cancelled) {
          setFatal(t.app.eventSubFailed(name, String(e)));
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
          void importRef.current(ev.paths, recursiveRef.current).catch((e) =>
            setFatal(t.app.importFailed(String(e)))
          );
        }
      }),
      "drag-drop"
    );
    register(
      onBatchFile((r) => {
        pendingRef.current.push({
          source: r.source,
          status: r.status,
          output: r.output,
          reason: r.reason,
        });
      }),
      "progress"
    );
    register(
      onBatchDone((s) => {
        setSummary(s);
        setRunning(false);
      }),
      "summary"
    );

    return () => {
      cancelled = true;
      cleanups.forEach((un) => un());
    };
    // 只在挂载时订阅一次；importPaths / recursive 均通过 ref 取最新值
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 合并刷新：O(batch) 增量更新，而不是每个文件一次全量重渲染
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
          // 只接受「等待 → 终态」的首次跃迁（root 保留）
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

  // 输入行变化后，旧的计划预览即失效
  useEffect(() => {
    setPlannedRows(null);
  }, [rows.length]);

  // ---- 动作 ----
  // QA 第二轮：所有 async 动作必须有拒绝分支（此前 invoke 失败变成
  // unhandled rejection —— 按钮点了没反应，用户完全不知道发生了什么）。
  const guard = useCallback(
    (label: string, p: Promise<unknown>) => {
      p.catch((e) => showToast(t.app.actionFailed(label, String(e))));
    },
    [showToast, t]
  );

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
    // 在事件处理里算出 next，不在 setState 更新器里改 ref（StrictMode 会调用两次）
    const next = rows.filter((r) => r.source !== source);
    indexRef.current = new Map(next.map((r, i) => [r.source, i]));
    setRows(next);
  };

  const doCancel = useCallback(() => {
    guard(
      t.app.labelCancel,
      cancelBatch().then((ok) =>
        showToast(ok ? t.app.cancelRequested : t.app.nothingToCancel)
      )
    );
  }, [guard, showToast, t]);

  const exportFailures = async () => {
    const failedRows = rows.filter((r) => r.status === "failed");
    if (failedRows.length === 0) return;
    try {
      const path = await saveFailures(
        failedRows.map((r) => ({ source: r.source, status: "failed", reason: r.reason }))
      );
      if (path) showToast(t.app.failureExported(path));
    } catch (e) {
      showToast(t.app.exportFailed(String(e)));
    }
  };

  const startRun = async () => {
    if (running) return;
    if (rows.length === 0) {
      showToast(t.app.emptyList);
      return;
    }
    if (settings.saveTo === "custom" && !settings.outDir.trim()) {
      showToast(t.app.needOutDir);
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
          t.app.planOk(items.length - failed) +
            (failed > 0 ? t.app.planFailed(failed) : t.app.planNoChange)
        );
      } catch (e) {
        showToast(String(e));
      }
      return;
    }

    // 允许重跑：先把所有行重置为等待
    pendingRef.current = [];
    setPlannedRows(null);
    setRows((prev) =>
      prev.map((r) => ({ ...r, status: "pending" as const, output: null, reason: null }))
    );
    setSummary(null);
    setElapsedMs(0);
    setRunning(true);

    if (!IS_DESKTOP) {
      // P8.2.6：fnOS 服务端形态——逐文件 HTTP 插件迁移（无批处理进程/事件）
      const t0 = Date.now();
      // AUD-1（严重修复）：此前 HTTP 分支忽略 dryRun，勾选「仅规划」仍会真实写文件
      if (settings.dryRun) {
        setSummary({
          planned: rows.length,
          ok: 0,
          skipped: 0,
          cancelled: 0,
          failed: 0,
          durationMs: Date.now() - t0,
          isCancelled: false,
          results: [],
        });
        setRunning(false);
        return;
      }
      let ok = 0;
      let failed = 0;
      let skipped = 0;
      const results: FileResult[] = [];
      const ncmRows: Row[] = [];
      // 循环 1：插件格式逐文件迁移（kwm/qmc 系 → /api/convert）；.ncm 收集走内置批处理
      for (const r of rows) {
        const ext = extOf(r.source);
        if (ext === "ncm") {
          ncmRows.push(r);
          continue;
        }
        const plugin = PLUGIN_BY_EXT[ext];
        if (!plugin) {
          failed += 1;
          const reason = t.app.fnosNoPlugin(ext || "?");
          results.push({ source: r.source, status: "failed", output: null, reason });
          setRows((prev) =>
            prev.map((x) =>
              x.source === r.source
                ? { ...x, status: "failed" as const, output: null, reason }
                : x
            )
          );
          continue;
        }
        try {
          const out = await formatMigrate(
            plugin,
            r.source,
            settings.saveTo === "custom" ? settings.outDir.trim() : undefined
          );
          ok += 1;
          results.push({ source: r.source, status: "ok", output: out.outputPath, reason: null });
          setRows((prev) =>
            prev.map((x) =>
              x.source === r.source
                ? { ...x, status: "ok" as const, output: out.outputPath, reason: null }
                : x
            )
          );
        } catch (e) {
          failed += 1;
          const reason = String(e);
          results.push({ source: r.source, status: "failed", output: null, reason });
          setRows((prev) =>
            prev.map((x) =>
              x.source === r.source
                ? { ...x, status: "failed" as const, output: null, reason }
                : x
            )
          );
        }
      }
      // 循环 2：.ncm 单次内置批处理（HTTP 同步形态无进度事件，一次取终态 summary）
      if (ncmRows.length > 0) {
        try {
          const s = await runBatchHttp({
            inputs: ncmRows.map((r) => ({ path: r.source, root: r.root })),
            outDir: settings.saveTo === "custom" ? settings.outDir.trim() : null,
            template: settings.template,
            skipExisting: settings.skipExisting,
            recursive: true,
            jobs: settings.jobs,
            dryRun: false,
          });
          for (const r of s.results) {
            results.push({
              source: r.source,
              status: r.status,
              output: r.output,
              reason: r.reason,
            });
            if (r.status === "ok") ok += 1;
            else if (r.status === "skipped") skipped += 1;
            else if (r.status === "failed") failed += 1;
            setRows((prev) =>
              prev.map((x) =>
                x.source === r.source
                  ? { ...x, status: r.status, output: r.output, reason: r.reason }
                  : x
              )
            );
          }
        } catch (e) {
          // batch 调用整体失败：全部 ncm 行显式标失败
          for (const r of ncmRows) {
            failed += 1;
            const reason = String(e);
            results.push({ source: r.source, status: "failed", output: null, reason });
            setRows((prev) =>
              prev.map((x) =>
                x.source === r.source
                  ? { ...x, status: "failed" as const, output: null, reason }
                  : x
              )
            );
          }
        }
      }
      const durationMs = Date.now() - t0;
      setElapsedMs(durationMs); // AUD-3：HTTP 分支结束时同步计时显示
      setSummary({
        planned: 0,
        ok,
        skipped,
        cancelled: 0,
        failed,
        durationMs,
        isCancelled: false,
        results,
      });
      setRunning(false);
      return;
    }

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

  // ---- 派生：筛选 + 虚拟窗口 + 计数 ----
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

  // 计数从 rows 派生（而非增量累加）：追加导入/重跑/清除时天然正确
  const counts = filterCounts;
  const start = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const end = Math.min(filtered.length, Math.ceil((scrollTop + viewH) / ROW_H) + OVERSCAN);
  const visible = filtered.slice(start, end);

  const doneCount = counts.ok + counts.skipped + counts.failed + counts.cancelled;
  const total = rows.length;
  const pct = percent(doneCount, total);
  const finishedMs = summary ? summary.durationMs : elapsedMs;

  return {
    rows,
    summary,
    plannedRows,
    setPlannedRows,
    running,
    elapsedMs,
    fatal,
    setFatal,
    dragOver,
    // 虚拟滚动
    viewRef,
    scrollTop,
    setScrollTop,
    viewH,
    // 列表派生
    filtered,
    visible,
    start,
    filterCounts,
    counts,
    doneCount,
    total,
    pct,
    finishedMs,
    // 动作
    importPaths,
    startRun,
    doCancel,
    clearList,
    removeRow,
    exportFailures,
    guard,
  };
}
