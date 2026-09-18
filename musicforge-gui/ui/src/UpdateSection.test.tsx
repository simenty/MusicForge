// UpdateSection 组件测试（P5）：无更新 / 有更新两条路径。
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  checkUpdate: vi.fn(),
  installUpdate: vi.fn(),
  restartApp: vi.fn(),
}));

import { checkUpdate } from "./api";
import UpdateSection from "./UpdateSection";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";

const mockCheck = vi.mocked(checkUpdate);

function renderSection() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <UpdateSection />
    </I18nProvider>
  );
}

describe("UpdateSection（更新）", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("已是最新：检查后提示且不出现安装按钮", async () => {
    mockCheck.mockResolvedValue({ available: false });
    renderSection();

    fireEvent.click(screen.getByText(zh.update.check));
    expect(await screen.findByText(zh.update.upToDate)).toBeInTheDocument();
    expect(screen.queryByText(zh.update.install)).toBeNull();
  });

  it("发现新版本：显示版本号与安装按钮", async () => {
    mockCheck.mockResolvedValue({ available: true, version: "9.9.9" });
    renderSection();

    fireEvent.click(screen.getByText(zh.update.check));
    expect(await screen.findByText(zh.update.available("9.9.9"))).toBeInTheDocument();
    expect(screen.getByText(zh.update.install)).toBeInTheDocument();
  });
});
