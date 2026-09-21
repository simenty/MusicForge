// 播放历史（P2）：倒序列表按「今天 / 昨天 / 更早」分组 + 清空（二次确认）。
import { useCallback, useEffect, useState } from "react";
import { IS_DESKTOP, historyClear, playHistory, removeTracks } from "./api";
import type { HistoryEntry, Track } from "./api";
import { useLang } from "./i18n";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import SortControl from "./SortControl";
import { useSort } from "./hooks/useSort";
import ConfirmDialog from "./ConfirmDialog";

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
  onPlayNext,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const [sort, setSort] = useSort("history", "default");
  // P6.23：批量从资料库移除的确认闸
  const [pendingRemove, setPendingRemove] = useState(false);
  const [busyRemove, setBusyRemove] = useState(false);
  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    playHistory(300, sort)
      .then(setRows)
      .catch(() => setRows([]));
  }, [sort]);
  useEffect(() => reload(), [reload]);

  const liked = useLiked();

  // P6.19 批量操作：历史按曲目 id 去重（同曲跨天多次出现只计一次）
  const selApi = useSelection();
  const list = rows ?? [];
  const selectedIds = new Set(list.filter((x) => selApi.sel.has(String(x.id))).map((x) => x.id));
  const seen = new Set<number>();
  const selectedTracks: Track[] = [];
  for (const e of list) {
    if (selectedIds.has(e.id) && !seen.has(e.id)) {
      seen.add(e.id);
      selectedTracks.push(e);
    }
  }
  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
  };
  // P6.23 批量从资料库移除（破坏性，经 ConfirmDialog 三级闸）；历史按曲目去重后取唯一 id
  const doRemove = async () => {
    const ids = [...new Set([...selApi.sel].map(Number))];
    if (ids.length === 0) {
      setPendingRemove(false);
      return;
    }
    setBusyRemove(true);
    try {
      await removeTracks(ids);
      selApi.toggleSelMode();
      reload();
    } catch {
      /* 失败：列表不变即为信号 */
    } finally {
      setBusyRemove(false);
      setPendingRemove(false);
    }
  };

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
          <SortControl
            value={sort}
            onChange={setSort}
            fields={[
              { value: "default", label: t.sort.playedAt },
              { value: "title", label: t.sort.title },
              { value: "artist", label: t.sort.artist },
              { value: "album", label: t.sort.album },
              { value: "duration", label: t.sort.duration },
              { value: "play_count", label: t.sort.playCount },
            ]}
          />
          <button
            className="btn sm"
            onClick={() => void clear()}
            disabled={busy || !rows || rows.length === 0}
          >
            {t.media.historyClear}
          </button>
          <button
            className={"btn sm" + (selApi.selMode ? " on" : "")}
            onClick={selApi.toggleSelMode}
            disabled={!rows || rows.length === 0}
          >
            {t.player.select}
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
      ) : sort === "default" ? (
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
                  onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
                  selectable={selApi.selMode}
                  selected={selApi.has(String(r.id))}
                  onToggleSelect={() => selApi.toggle(String(r.id))}
              />
              ))}
            </div>
          </section>
        ))
      ) : (
        <div className="tracks">
          {rows.map((r, i) => (
            <TrackRow
              key={`${r.id}-${r.playedAt}-${i}`}
              lead={timeOf(r.playedAt)}
              track={r}
              onPlay={onPlay ? () => playFrom(r) : undefined}
              liked={liked.isLiked(r.id)}
              onLike={() => void liked.toggle(r.id)}
              onQueue={onQueue ? () => void onQueue([r]) : undefined}
              onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
              selectable={selApi.selMode}
              selected={selApi.has(String(r.id))}
              onToggleSelect={() => selApi.toggle(String(r.id))}
            />
          ))}
        </div>
      )}
      {selApi.selMode && (
        <SelectionBar
          count={selApi.count}
          total={list.length}
          onAddToQueue={bulkQueue}
          onPlayNext={bulkPlayNext}
          onRemove={() => setPendingRemove(true)}
          removeLabel={t.library.removeTitle}
          onSelectAll={() => selApi.selectAll([...new Set(list.map((x) => String(x.id)))] as string[])}
          onClear={selApi.toggleSelMode}
        />
      )}
      <ConfirmDialog
        open={pendingRemove}
        busy={busyRemove}
        title={t.library.removeTitle}
        summary={t.library.removeSummary(selApi.count)}
        items={list
          .filter((x) => selApi.sel.has(String(x.id)))
          .map((x) => x.title ?? x.path)
          .slice(0, 5)}
        moreCount={Math.max(0, selApi.count - 5)}
        note={t.library.removeNote}
        ackLabel={t.library.removeAck}
        confirmLabel={t.library.removeConfirm}
        cancelLabel={t.player.cancel}
        onConfirm={doRemove}
        onCancel={() => setPendingRemove(false)}
      />
    </>
  );
}
