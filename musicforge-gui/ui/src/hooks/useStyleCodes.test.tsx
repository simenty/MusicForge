// useStyleCodes：批量解析 + 缓存（列表内 chip 的数据来源）。
//
// 核心不变量：**批量**而非每行一次 IPC；已解析路径不重复请求（虚拟列表滚动/翻页
// 时这是性能关键——否则每次列表变化都会重发全量）。
import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({
  styleCodes: vi.fn(async () => ({})),
}));

import { styleCodes } from "../api";
import { useStyleCodes } from "./useStyleCodes";
import type { StyleCodeDto } from "../lib/types";

const mockStyleCodes = vi.mocked(styleCodes);

/** 有码块：`S01` 在 codebook 中译作「流行」 */
const A: StyleCodeDto = {
  year: 2023,
  style: "S01",
  mood: null,
  scenes: [],
  version: null,
  other: [],
  labels: { S01: "流行" },
};

describe("useStyleCodes", () => {
  beforeEach(() => {
    mockStyleCodes.mockReset();
    mockStyleCodes.mockResolvedValue({});
  });

  it("一次批量调用解析整批，只返回有码路径的短标签", async () => {
    mockStyleCodes.mockResolvedValue({ "/a.flac": A });
    const paths = ["/a.flac", "/b.flac"];
    const { result } = renderHook(() => useStyleCodes(paths));
    await waitFor(() => expect(result.current["/a.flac"]).toBe("2023 · 流行"));
    // 关键：1 次批量，而非每行一次
    expect(mockStyleCodes).toHaveBeenCalledTimes(1);
    expect(mockStyleCodes).toHaveBeenCalledWith(["/a.flac", "/b.flac"], undefined);
    // 无前导码块的路径不入结果（调用方取不到即"无码"）
    expect(result.current["/b.flac"]).toBeUndefined();
  });

  it("码名查不到时回退原始码（绝不编造）", async () => {
    mockStyleCodes.mockResolvedValue({ "/c.flac": { ...A, labels: {} } });
    const { result } = renderHook(() => useStyleCodes(["/c.flac"]));
    await waitFor(() => expect(result.current["/c.flac"]).toBe("2023 · S01"));
  });

  it("已解析的路径不重复请求（翻页只补新增）", async () => {
    const { rerender } = renderHook(({ paths }: { paths: string[] }) => useStyleCodes(paths), {
      initialProps: { paths: ["/a.flac"] },
    });
    await waitFor(() => expect(mockStyleCodes).toHaveBeenCalledTimes(1));
    // 列表增长：只请求**新增**的那个路径
    rerender({ paths: ["/a.flac", "/b.flac"] });
    await waitFor(() => expect(mockStyleCodes).toHaveBeenCalledTimes(2));
    expect(mockStyleCodes).toHaveBeenLastCalledWith(["/b.flac"], undefined);
  });

  it("请求失败时标记为空并停止重试（不刷屏）", async () => {
    mockStyleCodes.mockRejectedValue(new Error("bad codebook"));
    const { result, rerender } = renderHook(
      ({ paths }: { paths: string[] }) => useStyleCodes(paths),
      { initialProps: { paths: ["/a.flac"] } },
    );
    await waitFor(() => expect(mockStyleCodes).toHaveBeenCalledTimes(1));
    expect(result.current["/a.flac"]).toBeUndefined();
    // 同一路径再次渲染：已在缓存中（失败已标记），不再重发
    rerender({ paths: ["/a.flac"] });
    await new Promise((r) => setTimeout(r, 20));
    expect(mockStyleCodes).toHaveBeenCalledTimes(1);
  });
});
