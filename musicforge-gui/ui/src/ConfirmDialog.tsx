// 通用破坏性操作确认弹层（规格 §4.1「三级闸」）
//
// 三级：① 触发（调用方按钮，未选/无对象时禁用）
//       ② 预览（本弹层：影响摘要 + 清单前 N 条 + 目标说明）
//       ③ 勾选确认（ack 文案由调用方给；未勾选时确认按钮禁用）
//
// 为什么不用 window.confirm：
// - 不可样式化（与设计系统脱节）、移动端体验差、无法展示清单预览；
// - 无法被测试（需 mock 全局 window.confirm，且无法断言"勾选前置"这类更强的约束）。
//
// 可访问性：role=dialog + aria-modal + aria-labelledby；Esc 关闭；遮罩点击关闭（busy 时禁用）。
import { useEffect, useState, type ReactNode } from "react";

export interface ConfirmDialogProps {
  open: boolean;
  title: string;
  /** 影响摘要（可含 <b> 高亮数字） */
  summary: ReactNode;
  /** 清单预览（如文件路径前 N 条；空数组则不渲染清单块） */
  items?: string[];
  /** 「…还有 M 条」的 M（0 则不渲染） */
  moreCount?: number;
  /** 目标路径 / 回滚说明等 */
  note?: ReactNode;
  /** 勾选文案（如"我了解：可通过回收站整体还原"） */
  ackLabel: string;
  confirmLabel: string;
  cancelLabel: string;
  /** 执行中：禁用所有交互（含 Esc / 遮罩点击） */
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export default function ConfirmDialog({
  open,
  title,
  summary,
  items,
  moreCount = 0,
  note,
  ackLabel,
  confirmLabel,
  cancelLabel,
  busy = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const [ack, setAck] = useState(false);

  // 每次打开重置勾选（关闭再打开不应沿用上次的确认）
  useEffect(() => {
    if (open) setAck(false);
  }, [open]);

  // Esc 关闭（busy 时不响应）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onCancel();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, busy, onCancel]);

  if (!open) return null;

  return (
    <div
      className="modal-mask"
      role="presentation"
      onClick={() => {
        if (!busy) onCancel();
      }}
    >
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="confirm-title">{title}</h3>
        </div>
        <div className="modal-body">
          <div className="modal-summary">{summary}</div>
          {items && items.length > 0 && (
            <ul className="modal-list">
              {items.map((s, i) => (
                <li key={i}>{s}</li>
              ))}
              {moreCount > 0 && <li className="more">…{moreCount}</li>}
            </ul>
          )}
          {note && <div className="modal-note">{note}</div>}
          <label className="modal-ack">
            <input
              type="checkbox"
              checked={ack}
              onChange={(e) => setAck(e.target.checked)}
              disabled={busy}
            />
            {ackLabel}
          </label>
        </div>
        <div className="modal-foot">
          <button className="btn sm" onClick={onCancel} disabled={busy}>
            {cancelLabel}
          </button>
          <button className="btn sm danger" onClick={onConfirm} disabled={!ack || busy}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
