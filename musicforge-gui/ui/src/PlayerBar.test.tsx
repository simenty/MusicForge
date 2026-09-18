// PlayerBar 组件测试（P2）：空态禁用 / 播放中暂停切换 / 错误态展示。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

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
    playTracks: vi.fn(async () => {}),
    toggle: vi.fn(async () => {}),
    next: vi.fn(async () => {}),
    prev: vi.fn(async () => {}),
    jump: vi.fn(async () => {}),
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

  it("错误态：显示错误文案，主按钮禁用", () => {
    const p = api({}, { state: "error", error: "设备被移除", trackId: 7 });
    renderBar(p);
    expect(screen.getByText(zh.player.errorPrefix("设备被移除"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.player.play })).toBeDisabled();
  });
});
