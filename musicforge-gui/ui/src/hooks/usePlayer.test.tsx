// usePlayer 测试（P2-21）：队列编辑的乐观更新与失败回滚 / pause 仅在播放中生效 /
// playNext 插到当前之后 / 会话恢复（绝不出声）/ 音量钳制 + 节流下发。
//
// 说明：轮询（500ms）与音量节流（120ms）都用**真实**定时器——fake timers 会与 waitFor 互锁
// （waitFor 内部也用 setTimeout）。所有用例均在 500ms 内完成，不会触发第二轮轮询。
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({
  IS_DESKTOP: true,
  playerStatus: vi.fn(),
  playerPlayQueue: vi.fn(),
  playerToggle: vi.fn(),
  playerNext: vi.fn(),
  playerPrev: vi.fn(),
  playerJump: vi.fn(),
  playerQueueMove: vi.fn(),
  playerQueueRemove: vi.fn(),
  playerQueueAppend: vi.fn(),
  playerQueueInsertNext: vi.fn(),
  playerQueueClear: vi.fn(),
  playerSeek: vi.fn(),
  // 必须返回 Promise：hook 内部对音量下发做 .catch（节流定时器内，未 await）
  playerSetVolume: vi.fn(() => Promise.resolve()),
  playerSetMode: vi.fn(),
  playerStop: vi.fn(),
}));

import {
  playerNext,
  playerPlayQueue,
  playerPrev,
  playerQueueAppend,
  playerQueueClear,
  playerQueueInsertNext,
  playerQueueMove,
  playerQueueRemove,
  playerSeek,
  playerSetMode,
  playerSetVolume,
  playerStatus,
  playerStop,
  playerToggle,
} from "../api";
import { clearSession, saveSession } from "../lib/session";
import type { PlayerSnapshot, QueueItem, Track } from "../lib/types";
import { usePlayer } from "./usePlayer";

const mockStatus = vi.mocked(playerStatus);
const mockPlayQueue = vi.mocked(playerPlayQueue);
const mockToggle = vi.mocked(playerToggle);
const mockNext = vi.mocked(playerNext);
const mockPrev = vi.mocked(playerPrev);
const mockQueueMove = vi.mocked(playerQueueMove);
const mockQueueRemove = vi.mocked(playerQueueRemove);
const mockQueueAppend = vi.mocked(playerQueueAppend);
const mockQueueInsertNext = vi.mocked(playerQueueInsertNext);
const mockQueueClear = vi.mocked(playerQueueClear);
const mockSeek = vi.mocked(playerSeek);
const mockSetVolume = vi.mocked(playerSetVolume);
const mockSetMode = vi.mocked(playerSetMode);
const mockStop = vi.mocked(playerStop);

function snap(over: Partial<PlayerSnapshot> = {}): PlayerSnapshot {
  return {
    state: "paused",
    error: null,
    trackId: 1,
    title: "T1",
    artist: "A",
    durationMs: 1000,
    sampleRate: 44100,
    channels: 2,
    queueLen: 0,
    queueIndex: 0,
    playMode: "normal",
    volume: 0.8,
    positionMs: 0,
    underruns: 0,
    ...over,
  };
}

function track(id: number): Track {
  return {
    id,
    sourceId: 1,
    path: `/m/${id}.flac`,
    size: 1,
    title: `T${id}`,
    artist: "A",
    album: null,
    trackNo: 1,
    durationMs: 1000,
    format: "flac",
    sampleRate: 44100,
    bitDepth: 16,
    channels: 2,
    isLossless: true,
  };
}

function queueItem(id: number): QueueItem {
  return { trackId: id, path: `/m/${id}.flac`, title: `T${id}`, artist: "A", durationMs: 1000 };
}

describe("usePlayer (P2-21)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearSession();
    mockStatus.mockResolvedValue(snap());
  });

  it("playTracks：构造队列并提交引擎，同时作废上次会话", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2)], 1);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 2]);
    expect(mockPlayQueue).toHaveBeenCalledWith(
      [expect.objectContaining({ trackId: 1 }), expect.objectContaining({ trackId: 2 })],
      1
    );
    expect(result.current.restored).toBeNull();
  });

  it("pause：播放中才生效", async () => {
    mockStatus.mockResolvedValue(snap({ state: "playing" }));
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.playing).toBe(true));
    await act(async () => {
      await result.current.pause();
    });
    expect(mockToggle).toHaveBeenCalledTimes(1);
  });

  it("pause：非播放态不调用 toggle（绝不反向唤醒）", async () => {
    mockStatus.mockResolvedValue(snap({ state: "paused" }));
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.pause();
    });
    expect(mockToggle).not.toHaveBeenCalled();
  });

  it("queueMove：成功则前端与引擎同步重排", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2), track(3)], 0);
    });
    await act(async () => {
      await result.current.queueMove(2, 0);
    });
    expect(mockQueueMove).toHaveBeenCalledWith(2, 0);
    expect(result.current.queue.map((q) => q.trackId)).toEqual([3, 1, 2]);
  });

  it("queueMove：引擎失败必须回滚（否则前端队列与引擎永久失同步）", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2), track(3)], 0);
    });
    mockQueueMove.mockRejectedValueOnce(new Error("boom"));
    await act(async () => {
      await result.current.queueMove(2, 0);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 2, 3]);
  });

  it("queueRemove：移除生效；失败回滚", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2), track(3)], 0);
    });
    await act(async () => {
      await result.current.queueRemove(1);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 3]);

    mockQueueRemove.mockRejectedValueOnce(new Error("boom"));
    await act(async () => {
      await result.current.queueRemove(0);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 3]);
  });

  it("queueAppend：追加到队尾（不中断当前播放）", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2)], 0);
    });
    await act(async () => {
      await result.current.queueAppend([track(3)]);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 2, 3]);
    expect(mockQueueAppend).toHaveBeenCalledWith([expect.objectContaining({ trackId: 3 })]);
  });

  it("playNext：插到当前曲目之后（下一首播放）", async () => {
    mockStatus.mockResolvedValue(snap({ queueIndex: 0 }));
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1), track(2), track(3)], 0);
    });
    await act(async () => {
      await result.current.playNext([track(9)]);
    });
    expect(result.current.queue.map((q) => q.trackId)).toEqual([1, 9, 2, 3]);
    expect(mockQueueInsertNext).toHaveBeenCalledWith([expect.objectContaining({ trackId: 9 })]);
  });

  it("启动恢复：读到上次会话则入队并置 restored，但绝不自动出声", async () => {
    saveSession({ items: [queueItem(7)], index: 0, positionMs: 5000, ts: Date.now() });
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.restored).not.toBeNull());
    expect(result.current.queue.map((q) => q.trackId)).toEqual([7]);
    expect(mockPlayQueue).not.toHaveBeenCalled();
  });

  it("resume：重建队列并回到保存位置（>1.5s 才 seek）", async () => {
    saveSession({ items: [queueItem(7)], index: 0, positionMs: 5000, ts: Date.now() });
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.restored).not.toBeNull());
    await act(async () => {
      await result.current.resume();
    });
    expect(mockPlayQueue).toHaveBeenCalledWith([expect.objectContaining({ trackId: 7 })], 0);
    expect(mockSeek).toHaveBeenCalledWith(5000);
    expect(result.current.restored).toBeNull();
  });

  it("setVolume：钳制到 0..1，本地即时生效 + 120ms 节流下发", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    act(() => {
      result.current.setVolume(2); // 钳到 1
    });
    expect(result.current.status?.volume).toBe(1);
    expect(mockSetVolume).not.toHaveBeenCalled(); // 节流窗口内不下发
    await act(async () => {
      await new Promise((r) => setTimeout(r, 160));
    });
    expect(mockSetVolume).toHaveBeenCalledWith(1);
  });

  it("clearQueue：清空前端队列并通知引擎", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await result.current.playTracks([track(1)], 0);
    });
    await act(async () => {
      await result.current.clearQueue();
    });
    expect(result.current.queue).toHaveLength(0);
    expect(mockQueueClear).toHaveBeenCalled();
  });

  it("控制类操作直通引擎（toggle/next/prev/stop/setMode）", async () => {
    const { result } = renderHook(() => usePlayer());
    await waitFor(() => expect(result.current.status).not.toBeNull());
    await act(async () => {
      await Promise.all([
        result.current.toggle(),
        result.current.next(),
        result.current.prev(),
        result.current.stop(),
        result.current.setMode("shuffle"),
      ]);
    });
    expect(mockToggle).toHaveBeenCalled();
    expect(mockNext).toHaveBeenCalled();
    expect(mockPrev).toHaveBeenCalled();
    expect(mockStop).toHaveBeenCalled();
    expect(mockSetMode).toHaveBeenCalledWith("shuffle");
  });
});
