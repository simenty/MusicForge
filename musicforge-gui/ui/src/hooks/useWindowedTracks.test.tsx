// useWindowedTracks 测试（P1）：分页取数的窗口化行为。
// 重点：reset 后拉首页；rowAt 命中缓存；滚动到深处按可视区间拉对应页。
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({ listTracks: vi.fn() }));

import { listTracks } from "../api";
import type { Track } from "../lib/types";
import { TRACK_ROW_H, useWindowedTracks } from "./useWindowedTracks";

const mockList = vi.mocked(listTracks);

function trackRow(i: number): Track {
  return {
    id: i,
    sourceId: 1,
    path: `/m/${i}.flac`,
    size: 1024,
    title: `t${i}`,
    artist: "a",
    album: "al",
    trackNo: i,
    durationMs: 1000,
    format: "flac",
    sampleRate: 44100,
    bitDepth: 16,
    channels: 2,
    isLossless: true,
  };
}

describe("useWindowedTracks", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockList.mockImplementation(async (limit = 200, offset = 0) =>
      Array.from({ length: Math.min(limit, Math.max(0, 500 - offset)) }, (_, i) =>
        trackRow(offset + i)
      )
    );
  });

  it("reset 后拉取首页；rowAt 命中缓存", async () => {
    const { result } = renderHook(() => useWindowedTracks(200));

    act(() => result.current.reset(500));
    await waitFor(() => expect(mockList).toHaveBeenCalledWith(200, 0, undefined, undefined));
    await waitFor(() => expect(result.current.rowAt(0)?.title).toBe("t0"));
    expect(result.current.total).toBe(500);
    // 未加载页的行返回 undefined（渲染占位）
    expect(result.current.rowAt(400)).toBeUndefined();
  });

  it("滚动到深处：按可视区间拉取对应页（不拉全量）", async () => {
    const { result } = renderHook(() => useWindowedTracks(200));

    act(() => result.current.reset(500));
    await waitFor(() => expect(result.current.rowAt(0)?.title).toBe("t0"));

    const el = document.createElement("div");
    el.scrollTop = TRACK_ROW_H * 400; // 滚到第 400 行附近
    Object.defineProperty(el, "clientHeight", { value: 560, configurable: true });
    act(() => result.current.onScroll(el));

    // 可视区间 ≈ 第 392..420 行 → 页 1（offset 200）与页 2（offset 400）
    await waitFor(() => expect(mockList).toHaveBeenCalledWith(200, 200, undefined, undefined));
    await waitFor(() => expect(mockList).toHaveBeenCalledWith(200, 400, undefined, undefined));
    // 页 0 早已拉取；未请求过的页（如 offset 600）不应出现
    expect(mockList).not.toHaveBeenCalledWith(200, 600, undefined, undefined);
  });

  // P6.25：过滤词下推——同一 hook 需把 query 透传给服务端取页
  it("query 变化时透传给 listTracks", async () => {
    const { result, rerender } = renderHook(
      ({ q }: { q?: string }) => useWindowedTracks(200, undefined, q),
      { initialProps: {} }
    );

    act(() => result.current.reset(2));
    await waitFor(() => expect(mockList).toHaveBeenCalledWith(200, 0, undefined, undefined));

    act(() => rerender({ q: "晴天" }));
    await waitFor(() => expect(mockList).toHaveBeenCalledWith(200, 0, undefined, "晴天"));
  });
});
