// 曲目行（P3 从 LibraryPage 提取）：虚拟窗口 / 搜索结果 / 收藏页 /
// 统计页 / 历史页共用同一行组件。
//
// 列契约（与 styles.css 的 .vt-head/.vt-row 六列网格对齐）：
//   1 前导（序号 / 时刻 / 排名）· 2 标题+艺术家 · 3 专辑 · 4 时长 · 5 格式
//   6 尾列 = `trailing`（自定义，如播放次数）或爱心按钮（提供 onLike 时）。
import type { HTMLAttributes, ReactNode } from "react";
import { useLang } from "./i18n";
import { fmtClock } from "./lib/format";
import type { Track } from "./api";

export interface TrackRowProps {
  /** 第 1 列：序号 / 播放时刻 / 排名等 */
  lead: string | number;
  track: Track;
  /** 双击播放（缺省时无双击行为） */
  onPlay?: () => void;
  liked?: boolean;
  onLike?: () => void;
  /** 「加入歌单」入口（提供时尾列渲染 ＋ 按钮，与爱心并排） */
  onAdd?: () => void;
  /** 「加入队列」入口（提供时尾列渲染 ▶ 按钮；P6.16） */
  onQueue?: () => void;
  /** 「下一首播放」入口（提供时尾列渲染 ⏭ 按钮；P6.17） */
  onPlayNext?: () => void;
  /** 选择模式（P6.19）：开启时第 1 列显示勾选框 */
  selectable?: boolean;
  /** 当前行是否被选中（选择模式下） */
  selected?: boolean;
  /** 切换选中（选择模式下） */
  onToggleSelect?: () => void;
  /** 会话自定义的第 6 列（优先于 ＋/爱心；如统计页的播放次数） */
  trailing?: ReactNode;
  /** 透传到根 div 的属性（歌单页拖拽排序用：draggable/onDragStart/onDrop…） */
  dragProps?: HTMLAttributes<HTMLDivElement>;
}

export default function TrackRow({
  lead,
  track: tr,
  onPlay,
  liked,
  onLike,
  onAdd,
  onQueue,
  onPlayNext,
  selectable,
  selected,
  onToggleSelect,
  trailing,
  dragProps,
}: TrackRowProps) {
  const { t } = useLang();
  return (
    <div className={"vt-row" + (selected ? " sel" : "")} onDoubleClick={onPlay} {...dragProps}>
      {selectable ? (
        <button
          type="button"
          className={"vt-check" + (selected ? " on" : "")}
          onClick={(e) => {
            e.stopPropagation();
            onToggleSelect?.();
          }}
          aria-pressed={!!selected}
          aria-label={t.player.select}
          title={t.player.select}
        >
          {selected && (
            <svg
              width="13"
              height="13"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2.6"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M5 12l5 5L20 6" />
            </svg>
          )}
        </button>
      ) : (
        <span className="vt-idx">{lead}</span>
      )}
      <span className="vt-main">
        <b title={tr.title ?? undefined}>{tr.title ?? "—"}</b>
        <span>{tr.artist ?? "—"}</span>
      </span>
      <span className="vt-alb" title={tr.album ?? undefined}>
        {tr.album ?? "—"}
      </span>
      <span className="vt-num">{fmtClock(tr.durationMs)}</span>
      <span className="vt-num">
        {(tr.format ?? "").toUpperCase()}
        {tr.isLossless ? " · SQ" : ""}
      </span>
      <span className="vt-heart">
        {trailing ??
          ((onAdd || onQueue || onPlayNext || onLike) && (
            <>
              {onPlayNext && (
                <button
                  className="heart-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onPlayNext();
                  }}
                  aria-label={t.player.playNext}
                  title={t.player.playNext}
                >
                  <svg
                    width="15"
                    height="15"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.7"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M5 12h11" />
                    <path d="M13 8l4 4-4 4" />
                    <path d="M20 5v14" />
                  </svg>
                </button>
              )}
              {onQueue && (
                <button
                  className="heart-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onQueue();
                  }}
                  aria-label={t.player.queueAdd}
                  title={t.player.queueAdd}
                >
                  <svg
                    width="15"
                    height="15"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.7"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M4 6h12M4 12h12M4 18h8" />
                    <path d="M16 14l4 4M20 14l-4 4" />
                  </svg>
                </button>
              )}
              {onAdd && (
                <button
                  className="heart-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onAdd();
                  }}
                  aria-label={t.pl.addTo}
                  title={t.pl.addTo}
                >
                  <svg
                    width="15"
                    height="15"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.7"
                    strokeLinecap="round"
                  >
                    <path d="M12 5v14M5 12h14" />
                  </svg>
                </button>
              )}
              {onLike && (
                <button
                  className={"heart-btn" + (liked ? " on" : "")}
                  onClick={(e) => {
                    e.stopPropagation();
                    onLike();
                  }}
                  aria-label={liked ? t.player.unlike : t.player.like}
                  title={liked ? t.player.unlike : t.player.like}
                >
                  <svg
                    width="15"
                    height="15"
                    viewBox="0 0 24 24"
                    fill={liked ? "currentColor" : "none"}
                    stroke="currentColor"
                    strokeWidth="1.7"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  >
                    <path d="M12 20s-7-4.6-7-9.6A4 4 0 0112 7a4 4 0 017 3.4c0 5-7 9.6-7 9.6z" />
                  </svg>
                </button>
              )}
            </>
          ))}
      </span>
    </div>
  );
}
