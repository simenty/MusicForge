// PlaylistsPage 组件测试（P6.4）：空态创建 → 详情；列表 → 详情曲目。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  playlistsList: vi.fn(),
  playlistsCovers: vi.fn(),
  playlistCreate: vi.fn(),
  playlistTracks: vi.fn(),
  playlistRemove: vi.fn(),
  playlistRename: vi.fn(),
  playlistDelete: vi.fn(),
}));

import { playlistCreate, playlistTracks, playlistsCovers, playlistsList } from "./api";
import PlaylistsPage from "./PlaylistsPage";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";

const mockList = vi.mocked(playlistsList);
const mockCovers = vi.mocked(playlistsCovers);
const mockCreate = vi.mocked(playlistCreate);
const mockTracks = vi.mocked(playlistTracks);

function renderPage() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <PlaylistsPage />
    </I18nProvider>
  );
}

describe("PlaylistsPage（歌单）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockTracks.mockResolvedValue([]);
    mockCovers.mockResolvedValue({});
  });

  it("封面拼贴：有封面的歌单渲染到拼贴容器，无封面保持图标占位（P6.12）", async () => {
    mockList.mockResolvedValue([
      { id: 1, name: "有封面", trackCount: 2 },
      { id: 2, name: "无封面", trackCount: 0 },
    ]);
    mockCovers.mockResolvedValue({ "1": ["C:\\cov\\a.jpg", "C:\\cov\\b.jpg"] });
    renderPage();

    await screen.findByText("有封面");
    const mosaics = document.querySelectorAll(".pl-mosaic");
    expect(mosaics.length).toBe(1);
    expect(mosaics[0].querySelectorAll("img").length).toBe(2);
    expect(document.querySelector(".pl-mosaic.c2")).not.toBeNull();
  });

  it("空态：输入名称创建并进入详情", async () => {
    mockList.mockResolvedValue([]);
    mockCreate.mockResolvedValue(7);
    renderPage();

    expect(await screen.findByText(zh.pl.psNone)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText(zh.pl.psNamePlaceholder), {
      target: { value: "夜跑" },
    });
    fireEvent.click(screen.getByText(zh.pl.psCreate));

    expect(await screen.findByText(zh.pl.psEmpty)).toBeInTheDocument();
    expect(mockCreate).toHaveBeenCalledWith("夜跑");
    expect(mockTracks).toHaveBeenCalledWith(7);
  });

  it("列表态：打开歌单显示曲目与播放全部", async () => {
    mockList.mockResolvedValue([{ id: 1, name: "晨跑", trackCount: 1 }]);
    mockTracks.mockResolvedValue([
      {
        id: 11,
        sourceId: 1,
        path: "/m/a.flac",
        size: 1,
        title: "A",
        artist: "X",
        album: null,
        trackNo: null,
        durationMs: 1000,
        format: "flac",
        sampleRate: 44100,
        bitDepth: 16,
        channels: 2,
        isLossless: true,
      },
    ]);
    renderPage();

    fireEvent.click(await screen.findByText("晨跑"));
    expect(await screen.findByText("A")).toBeInTheDocument();
    expect(screen.getByText(zh.media.playAll)).toBeInTheDocument();
    expect(mockTracks).toHaveBeenCalledWith(1);
  });
});
