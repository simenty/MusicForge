// OrganizePanel 组件测试（P4-2）：整理「计划 → 确认 → 执行」路径。
// 重点：planned=0（全部已就位）时不可执行；二次确认被拒绝时绝不 organizeApply。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_SERVER_MODE: true,
  organizePlan: vi.fn(),
  organizeApply: vi.fn(),
  trashRestore: vi.fn(),
  selectDirectory: vi.fn(),
}));

import { organizeApply, organizePlan, type OrganizePlan } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import OrganizePanel from "./OrganizePanel";

const mockPlan = vi.mocked(organizePlan);
const mockApply = vi.mocked(organizeApply);

const DIR = "/m/library";
const DEFAULT_TPL = "{artist}/{album}/{title}";

const PLAN_2: OrganizePlan = {
  counts: { planned: 2, in_place: 1, skipped_conflict: 0, conflict_never: 0 },
  plan: {
    items: [
      { source: "/m/library/a.mp3", target: "/lib/A/a.mp3", status: "planned", note: null },
      { source: "/m/library/b.mp3", target: "/lib/A/b.mp3", status: "planned", note: null },
    ],
    template: DEFAULT_TPL,
    strategy: "move",
  },
};

function renderPanel() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <OrganizePanel />
    </I18nProvider>
  );
}

describe("OrganizePanel（整理）", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
  });

  it("未输入目录时「生成计划」禁用；顶部承诺预览只读", () => {
    renderPanel();
    expect(screen.getByRole("button", { name: zh.organize.planBtn })).toBeDisabled();
    expect(screen.getByText(zh.organize.readonlyNote)).toBeInTheDocument();
  });

  it("计划成功 → 显示待归档数与可执行按钮", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_2);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.organize.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.organize.planBtn }));

    expect(await screen.findByText(zh.organize.countPlanned(2))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.organize.applyBtn(2) })).toBeEnabled();
  });

  it("planned=0（全部已就位）→ 执行按钮禁用", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue({
      counts: { planned: 0, in_place: 3, skipped_conflict: 0, conflict_never: 0 },
      plan: { items: [], template: DEFAULT_TPL, strategy: "move" },
    });
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.organize.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.organize.planBtn }));

    expect(await screen.findByText(zh.organize.noChanges)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.organize.applyBtn(0) })).toBeDisabled();
  });

  it("二次确认被拒绝 → 绝不调用 organizeApply", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_2);
    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.organize.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.organize.planBtn }));
    await user.click(await screen.findByRole("button", { name: zh.organize.applyBtn(2) }));

    expect(confirmSpy).toHaveBeenCalledOnce();
    expect(mockApply).not.toHaveBeenCalled();
  });

  it("确认执行 → 显示结果与回滚清单，计划表被清空", async () => {
    const user = userEvent.setup();
    mockPlan.mockResolvedValue(PLAN_2);
    mockApply.mockResolvedValue({
      moved: 2,
      skipped: 0,
      failed: 0,
      rollback_manifest: "/m/library/.musicforge/trash/t9/rollback.jsonl",
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    renderPanel();

    await user.type(screen.getByPlaceholderText(zh.organize.dirPlaceholder), DIR);
    await user.click(screen.getByRole("button", { name: zh.organize.planBtn }));
    await user.click(await screen.findByRole("button", { name: zh.organize.applyBtn(2) }));

    expect(mockApply).toHaveBeenCalledWith({
      dir: DIR,
      template: DEFAULT_TPL,
      targetRoot: null,
    });
    expect(await screen.findByText(`✓ ${zh.organize.resultLine(2, 0, 0)}`)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: zh.organize.restoreBtn })).toBeEnabled();
    // 执行成功后计划表消失（避免"还能再点一次"的错觉）
    expect(screen.queryByText(zh.organize.countPlanned(2))).not.toBeInTheDocument();
  });
});
