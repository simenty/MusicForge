// 我喜欢的音乐（P3）：liked_tracks 列表 + 播放全部 + 行内取消喜欢。
//
// 取消喜欢后该行**立即从列表消失**（本地过滤，不回后端重拉）——
// 过滤只在初始 liked 集合加载完成后生效（否则会把整页误滤为空）。
import { useCallback, useEffect, useMemo, useState } from "react";
import { IS_DESKTOP, likedTracks } from "./api";
import type { Track } from "./api";
import { useLang } from "./i18n";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import SortControl from "./SortControl";
import FilterInput from "./FilterInput";
import { SORT_SUPPORTED, useSort } from "./hooks/useSort";
import { useRequestGuard } from "./hooks/useRequestGuard";
import { useWindowedTracks, TRACK_ROW_H } from "./hooks/useWindowedTracks";
import ConfirmDialog from "./ConfirmDialog";

export default function FavoritesPage({
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
  const [rows, setRows] = useState<Track[] | null>(null);
  const liked = useLiked();
  const selApi = useSelection();
  const [sort, setSort, sortRejected] = useSort(
    "favorites",
    "default",
    SORT_SUPPORTED.favorites
  );
  // P6.23：批量取消喜欢的确认闸
  const [pendingUnlike, setPendingUnlike] = useState(false);
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
    likedTracks(500, 0, sort, filter || undefined)
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

  // P2-20：复用 useWindowedTracks 虚拟化收藏列表（内存 slice，免改后端）。
  // live 提升为 useMemo 并置于早返回之前，使其引用稳定（避免 fetchPage 每帧重建导致死循环重取）。
  const live = useMemo(
    () => (rows && liked.loaded ? rows.filter((r) => liked.isLiked(r.id)) : rows),
    [rows, liked.loaded, liked.isLiked]
  );
  const favFetch = useCallback(
    (off: number, lim: number) => Promise.resolve((live ?? []).slice(off, off + lim)),
    [live]
  );
  const w = useWindowedTracks(200, undefined, undefined, favFetch);
  useEffect(() => {
    w.reset(live?.length ?? 0);
  }, [live, w.reset]);

  // P2-16：playFrom 改为稳定 useCallback（依赖稳定原语，避免每帧重建闭包），配合 TrackRow memo。
  // 必须置于 if (!IS_DESKTOP) 早返回之前，遵守 hooks 顺序（react-hooks/rules-of-hooks）。
  const handlePlay = useCallback(
    (tr: Track) => {
      if (!onPlay || !rows) return;
      const arr = liked.loaded ? rows.filter((r) => liked.isLiked(r.id)) : rows;
      const idx = arr.findIndex((x) => x.id === tr.id);
      void onPlay(arr, idx >= 0 ? idx : 0);
    },
    [onPlay, rows, liked.loaded, liked.isLiked]
  );

  // P2-16：行内动作稳定化，配合 TrackRow memo
  const handleLike = useCallback((tr: Track) => void liked.toggle(tr.id), [liked.toggle]);
  const handleQueue = useCallback((tr: Track) => void onQueue?.([tr]), [onQueue]);
  const handlePlayNext = useCallback((tr: Track) => void onPlayNext?.([tr]), [onPlayNext]);
  const handleToggleSelect = useCallback(
    (tr: Track) => selApi.toggle(String(tr.id)),
    [selApi.toggle]
  );

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  // P6.19 批量操作（selApi 已在组件顶部无条件初始化）
  const list = live ?? [];
  const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
  };
  // P6.23 批量取消喜欢（破坏性于「收藏」语义，但曲库/文件不动；经 ConfirmDialog 三级闸）
  const doUnlike = async () => {
    const ids = [...selApi.sel].map(Number);
    if (ids.length === 0) {
      setPendingUnlike(false);
      return;
    }
    await liked.unlikeMany(ids); // 成功即乐观更新本地集合 → 对应行消失
    selApi.toggleSelMode();
    setPendingUnlike(false);
  };

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.tabFavorites}</h2>
          <p className="sub">{t.media.favSub(live?.length ?? 0)}</p>
        </div>
        <div className="act">
          <FilterInput
            value={rawQuery}
            onChange={setRawQuery}
            placeholder={t.listFilter.placeholder}
            clearLabel={t.listFilter.clear}
          />
          <SortControl
            value={sort}
            onChange={setSort}
            note={sortRejected ? t.sort.unsupported : null}
            fields={[
              { value: "default", label: t.sort.likedAt },
              { value: "title", label: t.sort.title },
              { value: "artist", label: t.sort.artist },
              { value: "album", label: t.sort.album },
              { value: "duration", label: t.sort.duration },
              { value: "play_count", label: t.sort.playCount },
            ]}
          />
          <button
            className="btn sm primary"
            onClick={() => {
              if (onPlay && live && live.length > 0) void onPlay(live, 0);
            }}
            disabled={!live || live.length === 0}
          >
            {t.media.playAll}
          </button>
          <button
            className={"btn sm" + (selApi.selMode ? " on" : "")}
            onClick={selApi.toggleSelMode}
            disabled={!live || live.length === 0}
          >
            {t.player.select}
          </button>
        </div>
      </div>

      {live === null ? (
        <div className="media-empty">
          <p>{t.media.loading}</p>
        </div>
      ) : live.length === 0 ? (
        <div className="media-empty">
          <p>{t.media.favEmpty}</p>
        </div>
      ) : (
        <div className="panel" style={{ padding: 0, overflow: "hidden" }}>
          <div className="vt-head">
            <span>{t.media.colIndex}</span>
            <span>{t.media.colTitle}</span>
            <span>{t.media.colAlbum}</span>
            <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
            <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
            <span />
          </div>
          <div className="vt-scroll" onScroll={(e) => w.onScroll(e.currentTarget)}>
            <div style={{ height: w.total * TRACK_ROW_H, position: "relative" }}>
              <div style={{ transform: `translateY(${w.start * TRACK_ROW_H}px)` }}>
                {w.indices.map((i) => {
                  const tr = w.rowAt(i);
                  return tr ? (
                    <TrackRow
                      key={tr.id}
                      lead={i + 1}
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
        </div>
      )}
      {selApi.selMode && (
        <SelectionBar
          count={selApi.count}
          total={list.length}
          onAddToQueue={bulkQueue}
          onPlayNext={bulkPlayNext}
          onRemove={() => setPendingUnlike(true)}
          removeLabel={t.media.unlikeTitle}
          onSelectAll={() => selApi.selectAll(list.map((x) => String(x.id)))}
          onClear={selApi.toggleSelMode}
        />
      )}
      <ConfirmDialog
        open={pendingUnlike}
        title={t.media.unlikeTitle}
        summary={t.media.unlikeSummary(selApi.count)}
        note={t.media.unlikeNote}
        ackLabel={t.media.unlikeAck}
        confirmLabel={t.media.unlikeConfirm}
        cancelLabel={t.player.cancel}
        onConfirm={doUnlike}
        onCancel={() => setPendingUnlike(false)}
      />
    </>
  );
}
