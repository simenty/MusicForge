// LyricsPanel 组件测试（P6.11）：点击行跳转播放位置 / 偏移校准与持久化。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  lyricsFetch: vi.fn(async () => "[00:00.00]第一行\n[00:02.00]第二行\n[00:04.00]第三行"),
}));

import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import LyricsPanel from "./LyricsPanel";
import type { PlayerApi } from "./hooks/usePlayer";

/** 构造 PlayerApi 桩（默认：播放中、轨道 7、位置 1s）。 */
function api(positionMs = 1000): PlayerApi {
  return {
    status: {
      state: "playing",
      error: null,
      trackId: 7,
      title: "测试曲",
      artist: "X",
      durationMs: 100_000,
      sampleRate: 44100,
      channels: 2,
      queueLen: 1,
      queueIndex: 0,
      volume: 1,
      positionMs,
      underruns: 0,
    },
    playing: true,
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
    seek: vi.fn(async () => {}),
    setVolume: vi.fn(),
    stop: vi.fn(async () => {}),
  };
}

function renderPanel(p: PlayerApi) {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <LyricsPanel player={p} onClose={() => {}} />
    </I18nProvider>
  );
}

describe("LyricsPanel（歌词面板）", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("点击歌词行跳转到该时间点（偏移 0 即行时间）", async () => {
    const user = userEvent.setup();
    const p = api();
    renderPanel(p);
    await user.click(await screen.findByText("第二行"));
    expect(p.seek).toHaveBeenCalledWith(2000);
  });

  it("偏移 +0.5s：持久化按曲目存储，跳转目标随偏移", async () => {
    const user = userEvent.setup();
    const p = api();
    renderPanel(p);
    await screen.findByText("第三行");
    await user.click(screen.getByText("+0.5s"));
    expect(localStorage.getItem("mf.lrcOff.7")).toBe("500");
    expect(screen.getByText(zh.player.lrcOffset("0.5s"))).toBeInTheDocument();
    await user.click(screen.getByText("第三行"));
    expect(p.seek).toHaveBeenCalledWith(4500);
  });

  it("重置：点偏移值归零并清除持久化", async () => {
    localStorage.setItem("mf.lrcOff.7", "1000");
    const user = userEvent.setup();
    const p = api();
    renderPanel(p);
    await screen.findByText("第一行");
    await user.click(screen.getByText(zh.player.lrcOffset("1.0s")));
    expect(localStorage.getItem("mf.lrcOff.7")).toBeNull();
    expect(screen.getByText(zh.player.lrcOffset("0.0s"))).toBeInTheDocument();
  });
});
