// TrashPanel 组件测试（P4-2）：回收站还原关键路径。
// 重点是**安全语义**：未输入清单时按钮不可点；二次确认被拒绝时绝不调用 API。
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  IS_SERVER_MODE: true,
  trashRestore: vi.fn(),
}));

import { trashRestore } from "./api";
import { I18nProvider } from "./i18n";
import { zh } from "./i18n/zh";
import TrashPanel from "./TrashPanel";

const mockRestore = vi.mocked(trashRestore);

/** 固定中文渲染（detectLang 读 localStorage 的 mf.lang，须在 render 前设置）。 */
function renderPanel() {
  localStorage.setItem("mf.lang", "zh");
  return render(
    <I18nProvider>
      <TrashPanel />
    </I18nProvider>
  );
}

describe("TrashPanel（回收站还原）", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    vi.clearAllMocks();
  });

  it("未输入清单路径时，「整体还原」禁用", () => {
    renderPanel();
    expect(screen.getByRole("button", { name: zh.trash.restoreBtn })).toBeDisabled();
  });

  it("二次确认被拒绝（取消弹层）→ 绝不调用 trashRestore", async () => {
    const user = userEvent.setup();
    renderPanel();

    await user.type(
      screen.getByPlaceholderText(zh.trash.manifestPlaceholder),
      "/m/.musicforge/trash/t1/rollback.jsonl"
    );
    await user.click(screen.getByRole("button", { name: zh.trash.restoreBtn }));
    // 弹层打开后取消（拒绝二次确认）
    await user.click(screen.getByRole("button", { name: zh.player.cancel }));

    expect(mockRestore).not.toHaveBeenCalled();
  });

  it("三级闸：勾选确认后调用 trashRestore 并显示还原数量", async () => {
    const user = userEvent.setup();
    mockRestore.mockResolvedValue({ restored: 3 });
    renderPanel();

    await user.type(
      screen.getByPlaceholderText(zh.trash.manifestPlaceholder),
      "/m/.musicforge/trash/t1/rollback.jsonl"
    );
    await user.click(screen.getByRole("button", { name: zh.trash.restoreBtn }));
    // 三级闸：勾选确认 + 点击确认按钮（未勾选时确认按钮禁用）
    await user.click(screen.getByRole("checkbox"));
    await user.click(screen.getByRole("button", { name: zh.player.confirm }));

    expect(mockRestore).toHaveBeenCalledWith("/m/.musicforge/trash/t1/rollback.jsonl");
    expect(await screen.findByText(`✓ ${zh.trash.restored(3)}`)).toBeInTheDocument();
  });
});
