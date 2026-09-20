// 播放历史（P2）：倒序列表按「今天 / 昨天 / 更早」分组 + 清空（二次确认）。
import { useCallback, useEffect, useState } from "react";
import { IS_DESKTOP, historyClear, playHistory } from "./api";
import type { HistoryEntry, Track } from "./api";
import { useLang } from "./i18n";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";

type DayKey = "today" | "yesterday" | "earlier";

/** 播放时刻 → 分组键（本地时区） */
function dayKey(sec: number): DayKey {
  const now = new Date();
  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const t = sec * 1000;
  if (t >= startToday) return "today";
  if (t >= startToday - 86_400_000) return "yesterday";
  return "earlier";
}

function timeOf(sec: number): string {
  const d = new Date(sec * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export default function HistoryPage({
  onPlay,
  onQueue,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    playHistory(300)
      .then(setRows)
      .catch(() => setRows([]));
  }, []);
  useEffect(() => reload(), [reload]);

  const liked = useLiked();

  /** 双击历史行播放：队列 = 历史去重后的曲目（同一首歌只入队一次） */
  const playFrom = useCallback(
    (entry: HistoryEntry) => {
      if (!onPlay || !rows) return;
      const seen = new Set<number>();
      const queue: Track[] = [];
      for (const r of rows) {
        if (!seen.has(r.id)) {
          seen.add(r.id);
          queue.push(r);
        }
      }
      const idx = queue.findIndex((x) => x.id === entry.id);
      void onPlay(queue, idx >= 0 ? idx : 0);
    },
    [onPlay, rows]
  );

  const clear = useCallback(async () => {
    if (busy || !window.confirm(t.media.historyClearConfirm)) return;
    setBusy(true);
    try {
      const r = await historyClear();
      setMsg(t.media.historyCleared(r.cleared));
      reload();
    } catch {
      /* 静默：列表未变即为失败信号 */
    } finally {
      setBusy(false);
    }
  }, [busy, t, reload]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  // 已倒序 → 顺序扫描切分组（不重排、不改序）
  const grouped: { key: DayKey; items: HistoryEntry[] }[] = [];
  if (rows) {
    for (const r of rows) {
      const k = dayKey(r.playedAt);
      const last = grouped[grouped.length - 1];
      if (last && last.key === k) last.items.push(r);
      else grouped.push({ key: k, items: [r] });
    }
  }
  const labelOf = (k: DayKey) =>
    k === "today" ? t.media.today : k === "yesterday" ? t.media.yesterday : t.media.earlier;

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.historyTitle}</h2>
          <p className="sub">{t.media.historySub}</p>
        </div>
        <div className="act">
          <button
            className="btn sm"
            onClick={() => void clear()}
            disabled={busy || !rows || rows.length === 0}
          >
            {t.media.historyClear}
          </button>
        </div>
      </div>

      {msg && <div className="scan-note">{msg}</div>}

      {rows === null ? (
        <div className="media-empty">
          <p>{t.media.loading}</p>
        </div>
      ) : rows.length === 0 ? (
        <div className="media-empty">
          <p>{t.media.historyEmpty}</p>
        </div>
      ) : (
        grouped.map((g) => (
          <section key={g.key} className="tl-group">
            <h3>{labelOf(g.key)}</h3>
            <div className="tracks">
              {g.items.map((r, i) => (
                <TrackRow
                  key={`${r.id}-${r.playedAt}-${i}`}
                  lead={timeOf(r.playedAt)}
                  track={r}
                  onPlay={onPlay ? () => playFrom(r) : undefined}
                  liked={liked.isLiked(r.id)}
                  onLike={() => void liked.toggle(r.id)}
                  onQueue={onQueue ? () => void onQueue([r]) : undefined}
                />
              ))}
            </div>
          </section>
        ))
      )}
    </>
  );
}
