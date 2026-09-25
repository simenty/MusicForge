// AlbumsPage 组件测试（P6.10）：列表 → 详情（曲目 + 播放全部）→ 返回。
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  listAlbums: vi.fn(),
  albumTracks: vi.fn(),
  coverFetch: vi.fn(),
  coverPickImage: vi.fn(),
  coverSetLocal: vi.fn(),
}));

vi.mock("./hooks/useSettings", () => ({
  useSettings: () => ({ settings: { onlineMeta: false } }),
}));

import { albumTracks, listAlbums } from "./api";
import AlbumsPage from "./AlbumsPage";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";

const mockList = vi.mocked(listAlbums);
const mockTracks = vi.mocked(albumTracks);

function track(id: number, title: string) {
  return {
    id,
    sourceId: 1,
    path: `/m/${id}.flac`,
    size: 1,
    title,
    artist: "Beyond",
    album: "乐与怒",
    trackNo: 1,
    durationMs: 1000,
    format: "flac",
    sampleRate: 44100,
    bitDepth: 16,
    channels: 2,
    isLossless: true,
  };
}

function renderPage() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <AlbumsPage />
    </I18nProvider>
  );
}

describe("AlbumsPage（专辑详情）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // jsdom 无布局：虚拟化滚动容器 clientHeight 为 0 → 仅渲染首行。
    // 模拟容器高度，使窗口覆盖全部曲目（与生产虚拟滚动行为无关，仅让组件测试可见全量）。
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 800,
    });
  });

  afterEach(() => {
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get: () => 0,
    });
  });

  it("点卡片进入详情：曲目表 + 播放全部；返回回到列表", async () => {
    mockList.mockResolvedValue([
      { id: 1, title: "乐与怒", artist: "Beyond", year: 1993, trackCount: 2, coverPath: null },
    ]);
    mockTracks.mockResolvedValue([track(11, "海阔天空"), track(12, "爸爸妈妈")]);
    renderPage();

    fireEvent.click(await screen.findByText("乐与怒"));
    expect(await screen.findByText("海阔天空")).toBeInTheDocument();
    expect(screen.getByText("爸爸妈妈")).toBeInTheDocument();
    expect(mockTracks).toHaveBeenCalledWith(1);
    expect(screen.getByText(zh.media.playAll)).toBeInTheDocument();

    fireEvent.click(screen.getByText(new RegExp(zh.media.back)));
    expect(await screen.findByText(zh.media.albumsTitle)).toBeInTheDocument();
  });
});
