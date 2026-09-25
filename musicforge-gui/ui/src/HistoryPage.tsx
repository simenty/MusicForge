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
import FilterInput from "./FilterInput";
import { useSort } from "./hooks/useSort";
import { useRequestGuard } from "./hooks/useRequestGuard";
import { useWindowedTracks, TRACK_ROW_H } from "./hooks/useWindowedTracks";
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
  // P2-18：清空历史原本 window.confirm —— 改为 ConfirmDialog（可样式化/可测/可 i18n）
  const [pendingClear, setPendingClear] = useState(false);

  const [sort, setSort] = useSort("history", "default");
  // P6.23：批量从资料库移除的确认闸
  const [pendingRemove, setPendingRemove] = useState(false);
  const [busyRemove, setBusyRemove] = useState(false);
  /** P6.25：输入框原文；`filter` 为防抖后下推服务端的过滤词 */
  const [rawQuery, setRawQuery] = useState("");
  const [filter, setFilter] = useState("");

  useEffect(() => {
    const id = window.setTimeout(() => setFilter(rawQuery.trim()), 250);
    return () => window.clearTimeout(id);
  }, [rawQuery]);

  /** stale response 守卫：连续改排序/筛选词时，旧响应不得覆写新结果 */
  const reloadGuard = useRequestGuard();

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    const token = reloadGuard.token();
    playHistory(300, sort, filter || undefined)
      .then((rows) => {
        if (!reloadGuard.isStale(token)) setRows(rows);
      })
      .catch(() => {
        if (!reloadGuard.isStale(token)) setRows([]);
      });
  }, [sort, filter, reloadGuard]);
  useEffect(() => {
    reloadGuard.bump(); // 参数已变 → 作废旧代（须在 reload 取令牌之前）
    reload();
  }, [reload, reloadGuard]);

  // P2-20：复用 useWindowedTracks 虚拟化历史列表（内存 slice）。仅 non-default 扁平视图；
  // 默认「今天/昨天/更早」分组视图为变高布局，统一行高窗口器不兼容，保持全量渲染（上限 300 条）。
  const historyFetch = useCallback(
    (off: number, lim: number) => Promise.resolve((rows ?? []).slice(off, off + lim)),
    [rows]
  );
  const w = useWindowedTracks(200, undefined, undefined, historyFetch);
  useEffect(() => {
    w.reset(rows?.length ?? 0);
  }, [rows, w.reset]);

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
    (tr: Track) => {
      if (!onPlay || !rows) return;
      const seen = new Set<number>();
      const queue: Track[] = [];
      for (const r of rows) {
        if (!seen.has(r.id)) {
          seen.add(r.id);
          queue.push(r);
        }
      }
      const idx = queue.findIndex((x) => x.id === tr.id);
      void onPlay(queue, idx >= 0 ? idx : 0);
    },
    [onPlay, rows]
  );

  // P2-16：行内动作稳定化，配合 TrackRow memo
  const handlePlay = useCallback((tr: Track) => playFrom(tr), [playFrom]);
  const handleLike = useCallback((tr: Track) => void liked.toggle(tr.id), [liked.toggle]);
  const handleQueue = useCallback((tr: Track) => void onQueue?.([tr]), [onQueue]);
  const handlePlayNext = useCallback((tr: Track) => void onPlayNext?.([tr]), [onPlayNext]);
  const handleToggleSelect = useCallback(
    (tr: Track) => selApi.toggle(String(tr.id)),
    [selApi.toggle]
  );

  const clear = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      const r = await historyClear();
      setMsg(t.media.historyCleared(r.cleared));
      reload();
    } catch {
      /* 静默：列表未变即为失败信号 */
    } finally {
      setBusy(false);
      setPendingClear(false);
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
          <FilterInput
            value={rawQuery}
            onChange={setRawQuery}
            placeholder={t.listFilter.placeholder}
            clearLabel={t.listFilter.clear}
          />
          <button
            className="btn sm"
            onClick={() => setPendingClear(true)}
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
                  onPlay={onPlay ? handlePlay : undefined}
                  liked={liked.isLiked(r.id)}
                  onLike={handleLike}
                  onQueue={onQueue ? handleQueue : undefined}
                  onPlayNext={onPlayNext ? handlePlayNext : undefined}
                  selectable={selApi.selMode}
                  selected={selApi.has(String(r.id))}
                  onToggleSelect={handleToggleSelect}
              />
              ))}
            </div>
          </section>
        ))
      ) : (
        <div className="vt-scroll" onScroll={(e) => w.onScroll(e.currentTarget)}>
          <div style={{ height: w.total * TRACK_ROW_H, position: "relative" }}>
            <div style={{ transform: `translateY(${w.start * TRACK_ROW_H}px)` }}>
              {w.indices.map((i) => {
                const tr = w.rowAt(i) as HistoryEntry | undefined;
                return tr ? (
                  <TrackRow
                    key={tr.id}
                    lead={timeOf(tr.playedAt)}
                    track={tr}
                    onPlay={onPlay ? handlePlay : undefined}
                    liked={liked.isLiked(tr.id)}
                    onLike={handleLike}
                    onQueue={onQueue ? handleQueue : undefined}
                    onPlayNext={onPlayNext ? handlePlayNext : undefined}
                    selectable={selApi.selMode}
                    selected={selApi.has(String(tr.id))}
                    onToggleSelect={handleToggleSelect}
                  />
                ) : (
                  <div className="vt-row" key={`ph-${i}`}>
                    <span className="vt-idx">{i + 1}</span>
                    <span className="vt-main">
                      <b>{t.media.loading}</b>
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
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
      <ConfirmDialog
        open={pendingClear}
        busy={busy}
        title={t.media.historyClearConfirm}
        summary={t.media.historyClearConfirm}
        ackLabel={t.media.historyClearConfirm}
        confirmLabel={t.player.confirm}
        cancelLabel={t.player.cancel}
        onConfirm={clear}
        onCancel={() => setPendingClear(false)}
      />
    </>
  );
}
