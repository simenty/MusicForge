// useBatch 测试（P2-21）：导入追加去重 / 空列表与 dry-run 短路 / 进度事件合并刷新
// （只接受 pending→终态 首次跃迁）/ 订阅失败绝不静默 / 筛选与计数派生。
//
// 说明：批处理依赖 Tauri IPC（api）与 i18n 字典——api 用 vi.mock 提供，t 直接用真实 zh
// 字典（避免为字典单独造桩）。事件订阅（onBatchFile/onBatchDone）通过 mock 捕获 handler，
// 便于在测试中直接投递事件；合并刷新为 100ms 定时器，相关用例用 fake timers 推进。
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({
  IS_DESKTOP: true,
  collectFiles: vi.fn(),
  startBatch: vi.fn(),
  planBatch: vi.fn(),
  runBatchHttp: vi.fn(),
  cancelBatch: vi.fn(),
  saveFailures: vi.fn(),
  formatMigrate: vi.fn(),
  onBatchFile: vi.fn(() => Promise.resolve(() => {})),
  onBatchDone: vi.fn(() => Promise.resolve(() => {})),
  onDragDropEvent: vi.fn(() => Promise.resolve(() => {})),
}));

import {
  collectFiles,
  onBatchDone,
  onBatchFile,
  planBatch,
  saveFailures,
  startBatch,
  type BatchSummary,
} from "../api";
import { DEFAULT_SETTINGS, type Settings } from "../settings";
import { zh } from "../i18n/zh";
import { useBatch, type FilterKey } from "./useBatch";

const mockCollect = vi.mocked(collectFiles);
const mockStart = vi.mocked(startBatch);
const mockPlan = vi.mocked(planBatch);
const mockSaveFailures = vi.mocked(saveFailures);
const mockOnFile = vi.mocked(onBatchFile);
const mockOnDone = vi.mocked(onBatchDone);

function entry(path: string, root: string | null = null) {
  return { path, root };
}

function renderBatch(opts?: { settings?: Partial<Settings>; filter?: FilterKey }) {
  const showToast = vi.fn();
  const settings: Settings = { ...DEFAULT_SETTINGS, ...opts?.settings };
  const hook = renderHook(() =>
    useBatch({ t: zh, settings, showToast, filter: opts?.filter ?? "all" })
  );
  return { ...hook, showToast, settings };
}

/** 最近一次 toast 文案（索引取末位——tsconfig 目标库低于 es2022，无 Array.prototype.at） */
function lastToast(showToast: ReturnType<typeof vi.fn>) {
  const calls = showToast.mock.calls;
  return String(calls[calls.length - 1]?.[0] ?? "");
}

/** 挂载时注册的 handler（每个 handler 只订阅一次） */
const fileHandler = () => mockOnFile.mock.calls[0][0];
const doneHandler = () => mockOnDone.mock.calls[0][0];

function summary(over: Partial<BatchSummary> = {}): BatchSummary {
  return {
    planned: 0,
    ok: 0,
    skipped: 0,
    cancelled: 0,
    failed: 0,
    durationMs: 0,
    isCancelled: false,
    results: [],
    ...over,
  };
}

describe("useBatch (P2-21)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("导入为追加语义：第二批不覆盖第一批，重复路径去重", async () => {
    const { result, showToast } = renderBatch();
    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm", "/a")]);
    await act(async () => {
      await result.current.importPaths(["/a"], true);
    });
    expect(result.current.rows.map((r) => r.source)).toEqual(["/a/1.ncm"]);

    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm", "/a"), entry("/b/2.ncm", "/b")]);
    await act(async () => {
      await result.current.importPaths(["/b"], true);
    });
    // 追加而非替换：前一批仍在，重复路径不重复入列
    expect(result.current.rows.map((r) => r.source)).toEqual(["/a/1.ncm", "/b/2.ncm"]);
    const toast = lastToast(showToast);
    expect(toast).toContain(zh.app.added(1));
    expect(toast).toContain(zh.app.deduped(1));
  });

  it("所选路径无 .ncm：提示且不加行", async () => {
    const { result, showToast } = renderBatch();
    mockCollect.mockResolvedValueOnce([]);
    await act(async () => {
      await result.current.importPaths(["/x"], true);
    });
    expect(result.current.rows).toHaveLength(0);
    expect(lastToast(showToast)).toBe(zh.app.noNcmFound(1));
  });

  it("空列表点开始：提示且不调用 startBatch", async () => {
    const { result, showToast } = renderBatch();
    await act(async () => {
      await result.current.startRun();
    });
    expect(mockStart).not.toHaveBeenCalled();
    expect(lastToast(showToast)).toBe(zh.app.emptyList);
  });

  it("dryRun：只生成计划预览（planBatch），绝不执行 startBatch", async () => {
    const { result } = renderBatch({ settings: { dryRun: true } });
    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm", "/a")]);
    await act(async () => {
      await result.current.importPaths(["/a"], true);
    });
    mockPlan.mockResolvedValueOnce([
      { source: "/a/1.ncm", target: "/out/1.flac", format: "flac", error: null },
    ]);
    await act(async () => {
      await result.current.startRun();
    });
    expect(mockPlan).toHaveBeenCalledTimes(1);
    expect(mockPlan.mock.calls[0][0]).toMatchObject({ dryRun: true });
    expect(mockStart).not.toHaveBeenCalled();
    expect(result.current.plannedRows).toHaveLength(1);
  });

  it("执行：提交 startBatch；收到 done 事件后收尾（running=false + summary）", async () => {
    const { result } = renderBatch();
    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm", "/a")]);
    await act(async () => {
      await result.current.importPaths(["/a"], true);
    });
    mockStart.mockResolvedValueOnce(undefined);
    await act(async () => {
      await result.current.startRun();
    });
    expect(mockStart).toHaveBeenCalledTimes(1);
    expect(result.current.running).toBe(true);

    const s = summary({ planned: 1, ok: 1 });
    await act(async () => {
      doneHandler()(s);
    });
    expect(result.current.running).toBe(false);
    expect(result.current.summary).toEqual(s);
  });

  it("进度事件：100ms 合并刷新，且只接受 pending→终态 首次跃迁", async () => {
    vi.useFakeTimers();
    try {
      const { result } = renderBatch();
      mockCollect.mockResolvedValueOnce([entry("/a/1.ncm", "/a")]);
      await act(async () => {
        await result.current.importPaths(["/a"], true);
      });

      act(() => {
        fileHandler()({ source: "/a/1.ncm", status: "ok", output: "/out/1.flac", reason: null });
      });
      await act(async () => {
        vi.advanceTimersByTime(150);
      });
      expect(result.current.rows[0].status).toBe("ok");

      // 已非 pending：第二次跃迁必须被忽略（终态不可被覆写）
      act(() => {
        fileHandler()({ source: "/a/1.ncm", status: "failed", output: null, reason: "boom" });
      });
      await act(async () => {
        vi.advanceTimersByTime(150);
      });
      expect(result.current.rows[0].status).toBe("ok");
    } finally {
      vi.useRealTimers();
    }
  });

  it("事件订阅失败：置 fatal（绝不静默吞掉）", async () => {
    mockOnFile.mockRejectedValueOnce(new Error("denied"));
    const { result } = renderBatch();
    await waitFor(() => expect(result.current.fatal).not.toBeNull());
    expect(result.current.fatal).toContain("denied");
  });

  it("筛选与计数从 rows 派生（非增量累加）", async () => {
    const { result } = renderBatch({ filter: "failed" });
    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm"), entry("/a/2.ncm")]);
    await act(async () => {
      await result.current.importPaths(["/a"], true);
    });
    expect(result.current.total).toBe(2);
    expect(result.current.counts.all).toBe(2);
    expect(result.current.counts.pending).toBe(2);
    expect(result.current.filtered).toHaveLength(0); // 暂无失败行
  });

  it("removeRow 移除指定行；clearList 清空", async () => {
    const { result } = renderBatch();
    mockCollect.mockResolvedValueOnce([entry("/a/1.ncm"), entry("/a/2.ncm")]);
    await act(async () => {
      await result.current.importPaths(["/a"], true);
    });
    act(() => {
      result.current.removeRow("/a/1.ncm");
    });
    expect(result.current.rows.map((r) => r.source)).toEqual(["/a/2.ncm"]);
    act(() => {
      result.current.clearList();
    });
    expect(result.current.rows).toHaveLength(0);
  });

  it("exportFailures 仅导出失败行", async () => {
    vi.useFakeTimers();
    try {
      const { result } = renderBatch();
      mockCollect.mockResolvedValueOnce([entry("/a/1.ncm"), entry("/a/2.ncm")]);
      await act(async () => {
        await result.current.importPaths(["/a"], true);
      });
      act(() => {
        fileHandler()({ source: "/a/2.ncm", status: "failed", output: null, reason: "bad" });
      });
      await act(async () => {
        vi.advanceTimersByTime(150);
      });
      mockSaveFailures.mockResolvedValueOnce("/tmp/f.csv");
      await act(async () => {
        await result.current.exportFailures();
      });
      expect(mockSaveFailures).toHaveBeenCalledWith([
        { source: "/a/2.ncm", status: "failed", reason: "bad" },
      ]);
    } finally {
      vi.useRealTimers();
    }
  });
});
