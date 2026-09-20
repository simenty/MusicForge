// SearchPalette 组件测试（P6.14）：分组渲染 / 回车播放曲目 / 跳转回调。
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  searchAll: vi.fn(async () => ({
    tracks: [],
    albums: [],
    artists: [],
    playlists: [],
  })),
}));

import { searchAll } from "./api";
import type { SearchResults, Track } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import SearchPalette from "./SearchPalette";

const track = (id: number, title: string): Track => ({
  id,
  sourceId: 1,
  path: `C:\\m\\${id}.flac`,
  size: 1024,
  title,
  artist: "Beyond",
  album: "乐与怒",
  trackNo: 1,
  durationMs: 200_000,
  format: "flac",
  sampleRate: 44100,
  bitDepth: 16,
  channels: 2,
  isLossless: true,
});

const results: SearchResults = {
  tracks: [track(1, "海阔天空")],
  albums: [
    { id: 11, title: "乐与怒", artist: "Beyond", year: 1993, trackCount: 2, coverPath: null },
  ],
  artists: [{ id: 21, name: "Beyond", trackCount: 2 }],
  playlists: [{ id: 31, name: "夜跑", trackCount: 0 }],
};

function renderPalette() {
  const onPlay = vi.fn(async (tracks: Track[], _index: number) => {
    void tracks;
  });
  const onOpen = vi.fn();
  localStorage.setItem("mf.lang", "zh");
  render(
    <I18nProvider>
      <SearchPalette onClose={() => {}} onPlay={onPlay} onOpen={onOpen} />
    </I18nProvider>
  );
  return { onPlay, onOpen };
}

describe("SearchPalette（全局搜索）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(searchAll).mockResolvedValue(results);
  });

  it("输入后按四组渲染结果（去抖后一次查询）", async () => {
    const user = userEvent.setup();
    renderPalette();
    await user.type(screen.getByRole("textbox"), "beyond");

    await waitFor(() => expect(screen.getByText("海阔天空")).toBeInTheDocument());
    expect(screen.getByText(zh.search.groupTracks)).toBeInTheDocument();
    expect(screen.getByText(zh.search.groupAlbums)).toBeInTheDocument();
    expect(screen.getByText(zh.search.groupArtists)).toBeInTheDocument();
    expect(screen.getByText(zh.search.groupPlaylists)).toBeInTheDocument();
    expect(screen.getByText("夜跑")).toBeInTheDocument();
  });

  it("回车：首项（曲目）直接播放；下移后落到专辑 → 走跳转回调", async () => {
    const user = userEvent.setup();
    const { onPlay, onOpen } = renderPalette();
    await user.type(screen.getByRole("textbox"), "beyond");
    await waitFor(() => expect(screen.getByText("海阔天空")).toBeInTheDocument());

    await user.keyboard("{Enter}");
    expect(onPlay).toHaveBeenCalledTimes(1);
    expect(onPlay.mock.calls[0][0][0].id).toBe(1);

    // 曲目组只有 1 条 → ↓ 一步即到专辑组首行
    await user.keyboard("{ArrowDown}");
    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledWith({ kind: "album", id: 11 });
  });
});
