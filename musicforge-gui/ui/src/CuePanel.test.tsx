// CuePanel 组件测试（P4）：选择 → 检视摘要 → 音频缺失守卫。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  cuePick: vi.fn(),
  cueInspect: vi.fn(),
  cueSplit: vi.fn(),
  selectDirectory: vi.fn(),
}));

import { cueInspect, cuePick } from "./api";
import CuePanel from "./CuePanel";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";

const mockPick = vi.mocked(cuePick);
const mockInspect = vi.mocked(cueInspect);

const INSPECT = {
  cue: "C:\\m\\album.cue",
  album: "乐与怒",
  performer: "Beyond",
  date: "1993",
  genre: "Rock",
  audioFile: "album.flac",
  audioExists: true,
  needsFfmpeg: false,
  tracks: [
    { number: 1, title: "海阔天空", performer: null },
    { number: 2, title: "爸爸妈妈", performer: null },
  ],
};

function renderPanel() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <CuePanel />
    </I18nProvider>
  );
}

describe("CuePanel（CUE 分轨）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("选择 CUE 后展示检视摘要、曲目表与默认输出目录", async () => {
    mockPick.mockResolvedValue("C:\\m\\album.cue");
    mockInspect.mockResolvedValue(INSPECT);
    renderPanel();

    fireEvent.click(screen.getByText(zh.cue.pick));
    expect(await screen.findByText("海阔天空")).toBeInTheDocument();
    expect(screen.getByText("爸爸妈妈")).toBeInTheDocument();
    expect(screen.getByText(/共 2 轨/)).toBeInTheDocument();
    // 默认输出目录 = CUE 同目录
    expect(screen.getByText("C:\\m")).toBeInTheDocument();
  });

  it("音频缺失：显示错误且切分按钮禁用", async () => {
    mockPick.mockResolvedValue("C:\\m\\album.cue");
    mockInspect.mockResolvedValue({ ...INSPECT, audioExists: false });
    renderPanel();

    fireEvent.click(screen.getByText(zh.cue.pick));
    expect(await screen.findByText(zh.cue.audioMissing)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.cue.splitAction })).toBeDisabled();
  });
});
