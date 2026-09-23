import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useRequestGuard } from "./useRequestGuard";

describe("useRequestGuard", () => {
it("当前代际的令牌不应被判为过期", () => {
  const { result } = renderHook(() => useRequestGuard());
  const t = result.current.token();
  expect(result.current.isStale(t)).toBe(false);
  expect(result.current.isMounted()).toBe(true);
});

it("bump 后旧令牌必须被判为过期（参数变化 → 旧响应不得覆写新结果）", () => {
  const { result } = renderHook(() => useRequestGuard());
  const old = result.current.token();

  act(() => result.current.bump());

  // 旧代际结果必须丢弃
  expect(result.current.isStale(old)).toBe(true);
  // 新代际结果必须放行
  expect(result.current.isStale(result.current.token())).toBe(false);
});

it("卸载后所有令牌都过期（不得对已卸载组件 setState）", () => {
  const { result, unmount } = renderHook(() => useRequestGuard());
  const t = result.current.token();

  unmount();

  expect(result.current.isMounted()).toBe(false);
  expect(result.current.isStale(t)).toBe(true);
});

it("多次 bump 只有最后一代有效（模拟连续改筛选词）", () => {
  const { result } = renderHook(() => useRequestGuard());
  const t0 = result.current.token();
  act(() => result.current.bump());
  const t1 = result.current.token();
  act(() => result.current.bump());
  const t2 = result.current.token();

  expect(result.current.isStale(t0)).toBe(true);
  expect(result.current.isStale(t1)).toBe(true);
  expect(result.current.isStale(t2)).toBe(false);
});
});
