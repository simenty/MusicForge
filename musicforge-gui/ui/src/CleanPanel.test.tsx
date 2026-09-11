// CleanPanel 组件测试（P4-2）：清洗「计划 → 确认 → 执行 → 还原」全路径。
// 重点在安全语义：无动作时执行按钮禁用；二次确认被拒绝时绝不 cleanApply。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_SERVER_MODE: true,
  cleanPlan: vi.fn(),
  cleanApply: vi.fn(),
  trashRestore: vi.fn(),
  selectDirectory: vi.fn(),
}));

import { cleanApply, cleanPlan, type CleanPlan } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import CleanPanel from "./CleanPanel";

const mockPlan = vi.mocked(cleanPlan);
const mockApply = vi.mocked(cleanApply);

const DIR = "/m/library";

const PLAN_1: CleanPlan = {
  actions: [{ path: "/m/library/orphan.mp3", rule_id: "R-ORPHAN" }],
  empty_dirs: ["/m/library/empty"],
  trash_root: "/m/library/.musicforge/trash/t1",
};

function renderPanel() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <CleanPanel />
    </I18nProvider>
  );
}

describe("CleanPanel（清洗）", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
  });

  it("未输入目录时「生成计划」禁用", () => {
    renderPanel();
    expect(screen.getByRole("button", { name: zh.clean.planBtn })).toBeDisabled();
  });

  it("计划成功 → 显示待清理数、回收站根与可执行按钮", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_1);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.clean.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.clean.planBtn }));

    expect(mockPlan).toHaveBeenCalledWith(DIR, undefined);
    expect(await screen.findByText(zh.clean.nActions(1))).toBeInTheDocument();
    expect(screen.getByText(zh.clean.trashRoot(PLAN_1.trash_root))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.clean.applyBtn(1) })).toBeEnabled();
  });

  it("无可清理项 → 执行按钮禁用（不误触发空操作）", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue({ actions: [], empty_dirs: [], trash_root: "/m/x" });
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.clean.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.clean.planBtn }));

    expect(await screen.findByText(zh.clean.nothingToClean)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.clean.applyBtn(0) })).toBeDisabled();
  });

  it("二次确认被拒绝 → 绝不调用 cleanApply", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_1);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.clean.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.clean.planBtn }));
    await user.click(await screen.findByRole("button", { name: zh.clean.applyBtn(1) }));

    expect(confirmSpy).toHaveBeenCalledOnce();
    expect(mockApply).not.toHaveBeenCalled();
  });

  it("确认执行 → 显示结果与回滚清单，并提供还原入口", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_1);
    mockApply.mockResolvedValue({
      moved: 1,
      dirs_removed: 1,
      rollback_manifest: "/m/library/.musicforge/trash/t1/rollback.jsonl",
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.clean.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.clean.planBtn }));
    await user.click(await screen.findByRole("button", { name: zh.clean.applyBtn(1) }));

    expect(mockApply).toHaveBeenCalledWith(DIR, undefined);
    expect(await screen.findByText(`✓ ${zh.clean.resultLine(1, 1)}`)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.clean.restoreBtn })).toBeEnabled();
  });
});
