// 批量操作栏（P6.19）：选择模式下固定在底部，展示已选数量并提供批量动作。
import { useLang } from "./i18n";

export interface SelectionBarProps {
  /** 已选数量 */
  count: number;
  /** 当前可见列表总数（用于「全选」可用性与禁用判定） */
  total: number;
  /** 批量加入队列 */
  onAddToQueue: () => void;
  /** 批量下一首播放 */
  onPlayNext: () => void;
  /** 全选当前可见列表 */
  onSelectAll: () => void;
  /** 取消选择（退出选择模式） */
  onClear: () => void;
}

export default function SelectionBar({
  count,
  total,
  onAddToQueue,
  onPlayNext,
  onSelectAll,
  onClear,
}: SelectionBarProps) {
  const { t } = useLang();
  return (
    <div className="sel-bar" role="toolbar" aria-label={t.player.selection}>
      <span className="sel-count">{t.player.selected(count)}</span>
      <div className="sel-acts">
        <button
          className="btn sm"
          onClick={onSelectAll}
          disabled={count >= total || total === 0}
          aria-label={t.player.selectAll}
        >
          {t.player.selectAll}
        </button>
        <button
          className="btn sm primary"
          onClick={onAddToQueue}
          disabled={count === 0}
          aria-label={t.player.queueAdd}
        >
          {t.player.queueAdd}
        </button>
        <button
          className="btn sm"
          onClick={onPlayNext}
          disabled={count === 0}
          aria-label={t.player.playNext}
        >
          {t.player.playNext}
        </button>
        <button className="btn sm ghost" onClick={onClear} aria-label={t.player.cancel}>
          {t.player.cancel}
        </button>
      </div>
    </div>
  );
}
