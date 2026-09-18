// SourcesPage 组件测试（P1）：媒体源添加/索引关键路径。
// 重点：未输入路径禁用；成功显示索引结果并刷新列表；失败显示错误。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  sourcesList: vi.fn(),
  sourcesAddAndIndex: vi.fn(),
  indexSource: vi.fn(),
  sourcesRemove: vi.fn(),
  selectDirectory: vi.fn(),
}));

import { sourcesAddAndIndex, sourcesList } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import SourcesPage from "./SourcesPage";

const mockList = vi.mocked(sourcesList);
const mockAddAndIndex = vi.mocked(sourcesAddAndIndex);

function renderPage() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <SourcesPage />
    </I18nProvider>
  );
}

describe("SourcesPage（媒体源）", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
    mockList.mockResolvedValue([]);
  });

  it("未输入路径时「添加并索引」禁用；空列表显示引导", async () => {
    renderPage();
    expect(await screen.findByText(zh.media.srcEmpty)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.media.srcAddIndex })).toBeDisabled();
  });

  it("添加并索引：成功后显示结果（含失败/清理为 0 时的精简文案）", async () => {
    const user = userEvent.setup();
    mockAddAndIndex.mockResolvedValue({
      id: 1,
      outcome: {
        scannedFiles: 10,
        audio: 8,
        indexed: 8,
        tagged: 6,
        untagged: 2,
        failed: 0,
        removed: 0,
      },
    });
    renderPage();

    await user.type(screen.getByPlaceholderText(zh.media.srcAddPlaceholder), "/vol/music");
    await user.click(screen.getByRole("button", { name: zh.media.srcAddIndex }));

    expect(mockAddAndIndex).toHaveBeenCalledWith("/vol/music");
    expect(
      await screen.findByText(zh.media.indexDone({ indexed: 8, removed: 0, failed: 0 }))
    ).toBeInTheDocument();
  });

  it("索引失败：显示错误文案（不静默）", async () => {
    const user = userEvent.setup();
    mockAddAndIndex.mockRejectedValue(new Error("目录不可读"));
    renderPage();

    await user.type(screen.getByPlaceholderText(zh.media.srcAddPlaceholder), "/vol/missing");
    await user.click(screen.getByRole("button", { name: zh.media.srcAddIndex }));

    expect(
      await screen.findByText(zh.media.indexFail("Error: 目录不可读"))
    ).toBeInTheDocument();
  });
});
