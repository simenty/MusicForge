// useLiked 测试（P6.23）：批量取消喜欢 optimistic 更新 + 空列表短路。
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({
  IS_DESKTOP: true,
  likedIds: vi.fn(),
  trackToggleLike: vi.fn(),
  unlikeTracks: vi.fn(),
}));

import { likedIds, unlikeTracks } from "../api";
import { useLiked } from "./useLiked";

const mockLikedIds = vi.mocked(likedIds);
const mockUnlike = vi.mocked(unlikeTracks);

describe("useLiked (P6.23)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockLikedIds.mockResolvedValue([1, 2, 3]);
    mockUnlike.mockResolvedValue(2);
  });

  it("unlikeMany 调用后端并乐观更新本地集合", async () => {
    const { result } = renderHook(() => useLiked());
    await waitFor(() => expect(result.current.loaded).toBe(true));
    expect(result.current.isLiked(2)).toBe(true);

    await act(async () => {
      await result.current.unlikeMany([2, 3]);
    });

    expect(mockUnlike).toHaveBeenCalledWith([2, 3]);
    expect(result.current.isLiked(2)).toBe(false);
    expect(result.current.isLiked(3)).toBe(false);
    expect(result.current.isLiked(1)).toBe(true);
  });

  it("空列表时不发 IPC", async () => {
    const { result } = renderHook(() => useLiked());
    await waitFor(() => expect(result.current.loaded).toBe(true));
    await act(async () => {
      await result.current.unlikeMany([]);
    });
    expect(mockUnlike).not.toHaveBeenCalled();
  });
});
