// useSort 测试（P1-14）：持久化排序键必须按「当前列表实际支持的键」校验——
// 不支持的键回退默认，并**显式**把失效键交给 UI 提示；绝不能静默回退
// （否则用户以为自己选的排序生效了，实际后端按 t.path 返回）。
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { SORT_SUPPORTED, useSort } from "./useSort";

describe("useSort (P1-14)", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("无持久化值时用 fallback", () => {
    const { result } = renderHook(() => useSort("t1", "default", SORT_SUPPORTED.library));
    expect(result.current[0]).toBe("default");
    expect(result.current[2]).toBeNull();
  });

  it("支持的持久化值被沿用，且无失效提示", () => {
    localStorage.setItem("mf.sort.t2", "title");
    const { result } = renderHook(() => useSort("t2", "default", SORT_SUPPORTED.library));
    expect(result.current[0]).toBe("title");
    expect(result.current[2]).toBeNull();
  });

  it("不支持的键（played_at → 库列表会静默回退 t.path）回退默认并显式报出", () => {
    localStorage.setItem("mf.sort.t3", "played_at");
    const { result } = renderHook(() => useSort("t3", "default", SORT_SUPPORTED.library));
    expect(result.current[0]).toBe("default");
    expect(result.current[2]).toBe("played_at"); // ← 显式提示的数据源
  });

  it("脏值（非排序键）同样回退并报出", () => {
    localStorage.setItem("mf.sort.t4", "garbage");
    const { result } = renderHook(() => useSort("t4", "default", SORT_SUPPORTED.library));
    expect(result.current[0]).toBe("default");
    expect(result.current[2]).toBe("garbage");
  });

  it("回退后把默认值写回 localStorage（下次进入不再提示）", () => {
    localStorage.setItem("mf.sort.t5", "liked_at");
    renderHook(() => useSort("t5", "default", SORT_SUPPORTED.library));
    expect(localStorage.getItem("mf.sort.t5")).toBe("default");
  });

  it("内存排序（详情页）不支持 play_count", () => {
    localStorage.setItem("mf.sort.t6", "play_count");
    const { result } = renderHook(() => useSort("t6", "default", SORT_SUPPORTED.memory));
    expect(result.current[0]).toBe("default");
    expect(result.current[2]).toBe("play_count");
  });

  it("不传 allowed 时保持原行为（不校验，兼容既有调用）", () => {
    localStorage.setItem("mf.sort.t7", "played_at");
    const { result } = renderHook(() => useSort("t7", "default"));
    expect(result.current[0]).toBe("played_at");
    expect(result.current[2]).toBeNull();
  });

  it("setSort 更新并持久化", () => {
    const { result } = renderHook(() => useSort("t8", "default", SORT_SUPPORTED.library));
    act(() => {
      result.current[1]("artist");
    });
    expect(result.current[0]).toBe("artist");
    expect(localStorage.getItem("mf.sort.t8")).toBe("artist");
  });
});
