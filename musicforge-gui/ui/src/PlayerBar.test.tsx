// PlayerBar 组件测试（P2）：空态禁用 / 播放中暂停切换 / 错误态展示。
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

// PlayerBar 经 IS_DESKTOP 区分「无曲目」与「服务端形态」文案——测试按桌面形态
// P6：组件在曲目切换时会查封面（trackCover）并提供歌词入口（LyricsPanel）
vi.mock("./api", () => ({
  IS_DESKTOP: true,
  trackCover: vi.fn(async () => null),
  lyricsFetch: vi.fn(async () => null),
}));

import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import PlayerBar from "./PlayerBar";
import type { PlayerApi } from "./hooks/usePlayer";
import type { PlayerSnapshot } from "./lib/types";

/** 构造 PlayerApi 桩（status 可部分覆盖）。 */
function api(over: Partial<PlayerApi> = {}, status: Partial<PlayerSnapshot> | null = null): PlayerApi {
  const snapshot: PlayerSnapshot | null = status
    ? {
        state: "idle",
        error: null,
        trackId: null,
        title: null,
        artist: null,
        durationMs: null,
        sampleRate: null,
        channels: null,
        queueLen: 0,
        queueIndex: null,
        volume: 1,
        positionMs: 0,
        underruns: 0,
        ...status,
      }
    : null;
  const base: PlayerApi = {
    status: snapshot,
    playing: false,
    queue: [],
    restored: null,
    resume: vi.fn(async () => {}),
    playTracks: vi.fn(async () => {}),
    toggle: vi.fn(async () => {}),
    pause: vi.fn(async () => {}),
    next: vi.fn(async () => {}),
    prev: vi.fn(async () => {}),
    jump: vi.fn(async () => {}),
    queueMove: vi.fn(async () => {}),
    queueRemove: vi.fn(async () => {}),
    queueAppend: vi.fn(async () => {}),
    clearQueue: vi.fn(async () => {}),
    playNext: vi.fn(async () => {}),
    seek: vi.fn(async () => {}),
    setVolume: vi.fn(),
    stop: vi.fn(async () => {}),
  };
  return { ...base, ...over };
}

function renderBar(p: PlayerApi) {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <PlayerBar player={p} />
    </I18nProvider>
  );
}

describe("PlayerBar（播放底栏）", () => {
  it("无曲目：显示未在播放，主按钮禁用", () => {
    const p = api();
    renderBar(p);
    expect(screen.getByText(zh.player.noTrack)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.player.play })).toBeDisabled();
  });

  it("播放中：显示曲目信息，主按钮换为暂停并调用 toggle", async () => {
    const user = userEvent.setup();
    const p = api(
      { playing: true },
      {
        state: "playing",
        trackId: 7,
        title: "海阔天空",
        artist: "Beyond",
        durationMs: 324_000,
        positionMs: 1_000,
        queueLen: 3,
      }
    );
    renderBar(p);
    expect(screen.getByText("海阔天空")).toBeInTheDocument();
    expect(screen.getByText(/Beyond/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: zh.player.pause }));
    expect(p.toggle).toHaveBeenCalledOnce();
  });

  it("断流态：显示错误文案，主按钮可点击（播放即触发流重建恢复）", async () => {
    const user = userEvent.setup();
    const p = api(
      {},
      { state: "paused", error: "音频输出中断（设备被移除？点播放重试）", trackId: 7 }
    );
    renderBar(p);
    expect(
      screen.getByText(zh.player.errorPrefix("音频输出中断（设备被移除？点播放重试）"))
    ).toBeInTheDocument();
    const btn = screen.getByRole("button", { name: zh.player.play });
    expect(btn).not.toBeDisabled();
    await user.click(btn);
    expect(p.toggle).toHaveBeenCalledOnce();
  });

  it("会话恢复：引擎空闲但有上次队列 → 显示曲目、点播放调 resume 而非 toggle", async () => {
    const user = userEvent.setup();
    const p = api(
      {
        restored: {
          items: [
            { trackId: 9, path: "C:\\m\\x.flac", title: "夜航", artist: "Nova", durationMs: 200_000 },
          ],
          index: 0,
          positionMs: 61_000,
          ts: 1,
        },
      },
      { state: "idle" }
    );
    renderBar(p);
    expect(screen.getByText("夜航")).toBeInTheDocument();
    expect(screen.getByText(new RegExp(zh.player.sessionResume))).toBeInTheDocument();
    const btn = screen.getByRole("button", { name: zh.player.play });
    expect(btn).not.toBeDisabled();
    await user.click(btn);
    expect(p.resume).toHaveBeenCalledOnce();
    expect(p.toggle).not.toHaveBeenCalled();
  });

  it("睡眠定时：15 分钟到期自动暂停（pause 一次性，不调 toggle）", () => {
    vi.useFakeTimers();
    try {
      const p = api(
        { playing: true },
        { state: "playing", trackId: 7, title: "T", positionMs: 1000 }
      );
      renderBar(p);
      // 假时钟下用 fireEvent（同步）——userEvent 的等待链依赖真实计时器
      fireEvent.click(screen.getByRole("button", { name: zh.player.sleep }));
      fireEvent.click(screen.getByRole("menuitem", { name: zh.player.sleepMin(15) }));
      expect(screen.queryByRole("menu")).not.toBeInTheDocument();

      // 未到期：不暂停
      act(() => {
        vi.advanceTimersByTime(14 * 60_000);
      });
      expect(p.pause).not.toHaveBeenCalled();

      // 到期：暂停一次（守卫语义由 hook 保证，此处断言调用与不误触 toggle）
      act(() => {
        vi.advanceTimersByTime(61_000);
      });
      expect(p.pause).toHaveBeenCalledOnce();
      expect(p.toggle).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  }, 20_000);

  it("队列可编辑：上移 / 移除 / 点选跳转分别调用对应方法（P6.15）", async () => {
    const user = userEvent.setup();
    const queue = [
      { trackId: 1, path: "a", title: "A", artist: "x", durationMs: 1 },
      { trackId: 2, path: "b", title: "B", artist: "y", durationMs: 1 },
      { trackId: 3, path: "c", title: "C", artist: "z", durationMs: 1 },
    ];
    const p = api(
      { queue },
      { state: "playing", queueLen: 3, queueIndex: 0, trackId: 1, title: "A" }
    );
    renderBar(p);
    await user.click(screen.getByRole("button", { name: zh.player.queue })); // 打开队列抽屉

    // 3 项 → 3 个上移按钮（首项禁用）；点第 2 项上移 → queueMove(1, 0)
    const ups = screen.getAllByRole("button", { name: zh.player.queueUp });
    expect(ups).toHaveLength(3);
    await user.click(ups[1]);
    expect(p.queueMove).toHaveBeenCalledWith(1, 0);

    // 末项移除 → queueRemove(2)
    const removes = screen.getAllByRole("button", { name: zh.player.queueRemove });
    await user.click(removes[2]);
    expect(p.queueRemove).toHaveBeenCalledWith(2);

    // 点选曲目 → jump(0)（并关闭抽屉）
    const jumps = screen.getAllByRole("button", { name: zh.player.jumpTo });
    await user.click(jumps[0]);
    expect(p.jump).toHaveBeenCalledWith(0);
  });

  it("队列清空：点击清空按钮调用 clearQueue（P6.16）", async () => {
    const user = userEvent.setup();
    const queue = [
      { trackId: 1, path: "a", title: "A", artist: "x", durationMs: 1 },
      { trackId: 2, path: "b", title: "B", artist: "y", durationMs: 1 },
    ];
    const p = api(
      { queue },
      { state: "playing", queueLen: 2, queueIndex: 0, trackId: 1, title: "A" }
    );
    renderBar(p);
    await user.click(screen.getByRole("button", { name: zh.player.queue })); // 打开队列抽屉
    await user.click(screen.getByRole("button", { name: zh.player.queueClear }));
    expect(p.clearQueue).toHaveBeenCalledOnce();
  });
});
