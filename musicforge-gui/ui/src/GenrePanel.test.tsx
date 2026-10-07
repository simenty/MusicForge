// GenrePanel（X15 风格码写回 genre）测试。
//
// 重点验证**安全纪律**：
// - 未规划时「写入」禁用（三级闸：未预览不得执行）；
// - 写入走二次确认（ConfirmDialog），不是直接落盘；
// - 失败数显式呈现（绝不谎报成功）。
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_DESKTOP: true,
  genrePlan: vi.fn(async () => ({
    items: [],
    will: 0,
    hasGenre: 0,
    noCode: 0,
    noLabel: 0,
  })),
  genreApply: vi.fn(async () => ({ written: 0, failed: 0 })),
  selectDirectory: vi.fn(async () => null),
}));

import { genreApply, genrePlan } from "./api";
import { I18nProvider } from "./i18n";
import GenrePanel from "./GenrePanel";
import type { GenrePlan } from "./lib/types";

const mockPlan = vi.mocked(genrePlan);
const mockApply = vi.mocked(genreApply);

const PLAN: GenrePlan = {
  items: [
    { path: "/lib/[Y23-S01] a.flac", status: "will-write", genre: "流行" },
    { path: "/lib/b.flac", status: "no-code", genre: null },
  ],
  will: 1,
  hasGenre: 0,
  noCode: 1,
  noLabel: 0,
};

function renderPanel(codebookPath = "") {
  return render(
    <I18nProvider>
      <GenrePanel codebookPath={codebookPath} />
    </I18nProvider>,
  );
}

/** 填入目录并跑一次规划（多数用例的前置） */
async function planWith(p: GenrePlan) {
  mockPlan.mockResolvedValue(p);
  renderPanel();
  fireEvent.change(screen.getByLabelText(/曲库目录/), { target: { value: "/lib" } });
  const planBtn = screen.getByRole("button", { name: "规划" });
  fireEvent.click(planBtn);
  await waitFor(() => expect(mockPlan).toHaveBeenCalled());
}

describe("GenrePanel", () => {
  beforeEach(() => {
    // 测试环境 navigator.language 为 en-US → 默认英文字典；本用例断言中文文案，
    // 故固定语言（I18nProvider 用 detectLang() 初始化，需在 render 前写入）。
    try {
      localStorage.setItem("mf.lang", "zh");
    } catch {
      /* 存储不可用则跟随系统：断言将失败，此处不静默掩盖 */
    }
    mockPlan.mockReset();
    mockApply.mockReset();
    mockPlan.mockResolvedValue({ items: [], will: 0, hasGenre: 0, noCode: 0, noLabel: 0 });
    mockApply.mockResolvedValue({ written: 2, failed: 0 });
  });

  it("无目录时规划按钮禁用", () => {
    renderPanel();
    expect(screen.getByRole("button", { name: "规划" })).toBeDisabled();
  });

  it("未规划时「写入」禁用（三级闸：未预览不得执行）", () => {
    renderPanel();
    fireEvent.change(screen.getByLabelText(/曲库目录/), { target: { value: "/lib" } });
    expect(screen.getByRole("button", { name: "写入" })).toBeDisabled();
  });

  it("规划后展示摘要与将写入预览", async () => {
    await planWith(PLAN);
    await waitFor(() => expect(screen.getByText(/将写入 1/)).toBeTruthy());
    expect(screen.getByText(/\/lib\/\[Y23-S01\] a\.flac/)).toBeTruthy();
    // 预览只列 will-write 项（no-code 不出现）
    expect(screen.queryByText(/\/lib\/b\.flac/)).toBeNull();
  });

  it("无可写项时「写入」仍禁用并给出说明", async () => {
    await planWith({ items: [{ path: "/x.flac", status: "no-code", genre: null }], will: 0, hasGenre: 0, noCode: 1, noLabel: 0 });
    await waitFor(() => expect(screen.getByText(/没有可写入的项/)).toBeTruthy());
    expect(screen.getByRole("button", { name: "写入" })).toBeDisabled();
  });

  it("写入经二次确认才落盘（不直接改文件）", async () => {
    await planWith(PLAN);
    await waitFor(() => expect(screen.getByText(/将写入 1/)).toBeTruthy());
    // 点「写入」→ 先弹确认，尚未调用 apply
    fireEvent.click(screen.getByRole("button", { name: "写入" }));
    await waitFor(() => expect(screen.getByText(/确认写入 genre/)).toBeTruthy());
    expect(mockApply).not.toHaveBeenCalled();
  });

  it("失败数显式呈现（绝不谎报成功）", async () => {
    mockApply.mockResolvedValue({ written: 1, failed: 2 });
    await planWith(PLAN);
    await waitFor(() => expect(screen.getByText(/将写入 1/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "写入" }));
    await waitFor(() => expect(screen.getByText(/确认写入 genre/)).toBeTruthy());
    // 勾选 → 确认（弹层内定位：面板自身还有一个 replaceAll 勾选框）
    const dialog = document.querySelector(".modal") as HTMLElement;
    expect(dialog).toBeTruthy();
    const ack = dialog.querySelector(".modal-ack input") as HTMLInputElement;
    fireEvent.click(ack);
    const confirmBtn = Array.from(dialog.querySelectorAll("button")).find(
      (b) => b.textContent?.trim() === "写入" && !(b as HTMLButtonElement).disabled,
    ) as HTMLButtonElement | undefined;
    expect(confirmBtn).toBeTruthy();
    fireEvent.click(confirmBtn!);
    await waitFor(() => expect(mockApply).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText(/失败 2/)).toBeTruthy());
  });

  it("未配 codebook 时提示将写入原始码", () => {
    renderPanel("");
    expect(screen.getByText(/未配置 codebook/)).toBeTruthy();
  });
});
