// StatsPage 组件测试（P3）：统计卡 / Top 榜渲染 / 空态。
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({ IS_DESKTOP: true, statsOverview: vi.fn() }));

import { statsOverview } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import StatsPage from "./StatsPage";

const mockStats = vi.mocked(statsOverview);

function track(id: number, title: string) {
  return {
    id,
    sourceId: 1,
    path: `/m/${id}.flac`,
    size: 1024,
    title,
    artist: "A",
    album: "Al",
    trackNo: 1,
    durationMs: 180_000,
    format: "flac",
    sampleRate: 44_100,
    bitDepth: 16,
    channels: 2,
    isLossless: true,
  };
}

function renderPage() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <StatsPage />
    </I18nProvider>
  );
}

describe("StatsPage（统计）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("渲染统计卡与 Top 榜（含播放次数）", async () => {
    mockStats.mockResolvedValue({
      tracks: 1286,
      artists: 48,
      albums: 86,
      totalSize: 86_400_000_000,
      totalDurationMs: 311_040_000,
      liked: 42,
      plays: 99,
      playedTracks: 512,
      daily: [],
      top: [{ ...track(7, "海阔天空"), playCount: 13 }],
    });
    renderPage();

    expect(await screen.findByText(zh.media.statLiked)).toBeInTheDocument();
    expect(screen.getByText("42")).toBeInTheDocument();
    expect(screen.getByText(zh.media.statPlays)).toBeInTheDocument();
    expect(screen.getByText("99")).toBeInTheDocument();
    expect(screen.getByText("海阔天空")).toBeInTheDocument();
    expect(screen.getByText(zh.media.playsN(13))).toBeInTheDocument();
  });

  it("无播放记录：Top 榜显示空态提示", async () => {
    mockStats.mockResolvedValue({
      tracks: 0,
      artists: 0,
      albums: 0,
      totalSize: 0,
      totalDurationMs: 0,
      liked: 0,
      plays: 0,
      playedTracks: 0,
      daily: [],
      top: [],
    });
    renderPage();
    expect(await screen.findByText(zh.media.historyEmpty)).toBeInTheDocument();
  });
});
