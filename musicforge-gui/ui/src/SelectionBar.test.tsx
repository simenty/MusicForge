// SelectionBar 组件测试（P6.19）：已选数量 / 批量动作回调 / 空选禁用。
// 不依赖具体语言文案——按钮顺序固定：全选 / 加入队列 / 下一首播放 / 取消。
import { render, screen, fireEvent } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { I18nProvider } from "./i18n";
import SelectionBar from "./SelectionBar";

function wrap(ui: ReactNode) {
  return render(<I18nProvider>{ui}</I18nProvider>);
}

describe("SelectionBar (P6.19)", () => {
  it("shows count and triggers bulk actions", () => {
    const onAdd = vi.fn();
    const onPlayNext = vi.fn();
    const onSelectAll = vi.fn();
    const onClear = vi.fn();
    wrap(
      <SelectionBar
        count={2}
        total={10}
        onAddToQueue={onAdd}
        onPlayNext={onPlayNext}
        onSelectAll={onSelectAll}
        onClear={onClear}
      />,
    );
    // 已选数量渲染（含数字）
    expect(screen.getByText(/\b2\b/)).toBeTruthy();
    const buttons = screen.getAllByRole("button");
    expect(buttons).toHaveLength(4);
    // 顺序：全选 / 加入队列 / 下一首播放 / 取消
    fireEvent.click(buttons[1]);
    expect(onAdd).toHaveBeenCalledTimes(1);
    fireEvent.click(buttons[2]);
    expect(onPlayNext).toHaveBeenCalledTimes(1);
    fireEvent.click(buttons[3]);
    expect(onClear).toHaveBeenCalledTimes(1);
  });

  it("disables bulk buttons when nothing selected", () => {
    wrap(
      <SelectionBar
        count={0}
        total={10}
        onAddToQueue={vi.fn()}
        onPlayNext={vi.fn()}
        onSelectAll={vi.fn()}
        onClear={vi.fn()}
      />,
    );
    const buttons = screen.getAllByRole("button");
    // 加入队列按钮（index 1）在空选时禁用
    expect((buttons[1] as HTMLButtonElement).disabled).toBe(true);
  });
});
