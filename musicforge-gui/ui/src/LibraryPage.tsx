// 音乐库（P1 核心页）：十万级曲目浏览 = 分页取数 + 自建虚拟滚动；
// 搜索走 core 的 search_tracks（一次 ≤500，不走虚拟化）。
// P2：双击行 → 以「已缓存行快照」为队列开始播放。
import { useCallback, useEffect, useMemo, useState } from "react";
import { IS_DESKTOP, countTracks, libraryStats, removeTracks } from "./api";
import type { LibraryStats, Track } from "./api";
import { useLang } from "./i18n";
import { fmtSizeGB } from "./lib/format";
import { useWindowedTracks, TRACK_ROW_H } from "./hooks/useWindowedTracks";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import ConfirmDialog from "./ConfirmDialog";
import SortControl from "./SortControl";
import { SORT_SUPPORTED, useSort } from "./hooks/useSort";
import { useRequestGuard } from "./hooks/useRequestGuard";

export default function LibraryPage({
  onPlay,
  onQueue,
  onPlayNext,
}: {
  /** 双击行 → 以当前已缓存曲目为队列播放（App 层注入 player.playTracks） */
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const [sort, setSort, sortRejected] = useSort("library", "default", SORT_SUPPORTED.library);
  /** P6.25：输入框原文；`filter` 是防抖后真正下推到服务端的过滤词 */
  const [rawQuery, setRawQuery] = useState("");
  const [filter, setFilter] = useState("");
  const w = useWindowedTracks(200, sort, filter || undefined);
  const liked = useLiked();
  // P6.19 批量操作（hook 须无条件调用，置于早返回之前）
  const selApi = useSelection();
  /** P6.4：待加入歌单的曲目（null = 弹层关闭） */
  const [addTarget, setAddTarget] = useState<Track | null>(null);
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [initErr, setInitErr] = useState<string | null>(null);
  // P6.22：批量从资料库移除的确认闸
  const [pendingRemove, setPendingRemove] = useState(false);

  const { reset, snapshot } = w;
  // P2-17：libraryStats 守卫（统计拉取与移除后刷新可能晚到，旧响应不得覆写新统计）
  const statsGuard = useRequestGuard();
  useEffect(() => {
    if (!IS_DESKTOP) return;
    const token = statsGuard.token();
    libraryStats()
      .then((s) => {
        if (!statsGuard.isStale(token)) setStats(s);
      })
      .catch((e: unknown) => {
        if (!statsGuard.isStale(token)) setInitErr(String(e));
      });
  }, []);

  // P6.25：输入防抖 250ms → 下推服务端过滤（虚拟化列表因此不必一次性拉全量）
  useEffect(() => {
    const id = window.setTimeout(() => setFilter(rawQuery.trim()), 250);
    return () => window.clearTimeout(id);
  }, [rawQuery]);

  // 过滤词/库内容变化 → 取**服务端过滤后的计数**并重设行数。
  // 行数必须等于过滤结果集大小，否则虚拟滚动会请求越界的页。
  //
  // P6.28：无筛选时**复用已拉取的全局统计**（`libraryStats` 已经在取曲目总数），
  // 不再额外发一次全表 COUNT——两个数据源指同一个数字，重复取既多一次查询、
  // 也可能在扫描进行中时给出与头部统计不一致的行数。
  // stale response 守卫：连续改筛选词时，旧计数的响应**不得**覆写新计数——
  // 虚拟化行数一旦与真实结果集错位，就会出现空白行或漏数据。
  const countGuard = useRequestGuard();
  useEffect(() => {
    if (!IS_DESKTOP) return;
    if (!filter) {
      if (stats) reset(stats.tracks);
      countGuard.bump();
      return;
    }
    countGuard.bump();
    const token = countGuard.token();
    countTracks(filter)
      .then((n) => {
        if (!countGuard.isStale(token)) reset(n);
      })
      .catch(() => {
        if (!countGuard.isStale(token)) reset(0);
      });
  }, [filter, stats, reset, countGuard]);

  const playFrom = useCallback(
    (tr: Track) => {
      if (!onPlay) return;
      const all = snapshot();
      const idx = all.findIndex((x) => x.id === tr.id);
      if (idx >= 0) void onPlay(all, idx);
      else void onPlay([tr], 0);
    },
    [onPlay, snapshot]
  );

  // P2-16：行内动作稳定化，配合 TrackRow memo（依赖具体稳定方法，避免每次渲染重建闭包）
  const handlePlay = useCallback((tr: Track) => playFrom(tr), [playFrom]);
  const handleLike = useCallback((tr: Track) => void liked.toggle(tr.id), [liked.toggle]);
  const handleAdd = useCallback((tr: Track) => setAddTarget(tr), []);
  const handleQueue = useCallback((tr: Track) => void onQueue?.([tr]), [onQueue]);
  const handlePlayNext = useCallback((tr: Track) => void onPlayNext?.([tr]), [onPlayNext]);
  const handleToggleSelect = useCallback(
    (tr: Track) => selApi.toggle(String(tr.id)),
    [selApi.toggle]
  );

  // 可见列表 = 虚拟化已加载快照（P6.25：过滤已下推到服务端，不再有独立的
  // 「搜索结果」集合，因此不受原先 500 条上限约束）；全选覆盖该集合
  // P2-15：`snapshot()` 每次调用按 `total` 全量迭代（十万曲库 = 十万次），此前渲染内
  // 被调用两次（列表 + 确认弹层 items）→ 每次 setState 约 20 万次迭代。仅在「已加载
  // 页数 / 总数 / 取数忙闲」变化时才重算；页只增不逐出，`loadedPages` 单调增可精确
  // 反映数据变化，滚动等无关渲染不再重算（修复审计 #15 首要热点）。
  //
  // 必须置于所有早返回之前：`initErr` 会在运行时被置位（批量移除失败），
  // 若在其后的早返回之后调用这些 hook 会触发「Rendered fewer hooks」。
  const list = useMemo(() => snapshot(), [w.total, w.loadedPages, w.busy]);
  const selectedTracks = useMemo(
    () => list.filter((x) => selApi.sel.has(String(x.id))),
    [list, selApi]
  );
  // P2-15：确认弹层 items 此前每次渲染都现算（即便弹层关闭），叠加 snapshot 全量迭代。
  // 仅在弹层开启时构造，并复用上面的 memo `list`。
  const removeItems = useMemo(
    () =>
      pendingRemove
        ? list
            .filter((x) => selApi.sel.has(String(x.id)))
            .map((x) => x.title ?? x.path)
            .slice(0, 5)
        : [],
    [pendingRemove, list, selApi]
  );

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }
  if (initErr) {
    return (
      <div className="media-empty">
        <p>{initErr}</p>
      </div>
    );
  }

  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
  };
  // P6.22 批量从资料库移除（破坏性，经 ConfirmDialog 三级闸）
  const doRemove = async () => {
    const ids = [...selApi.sel].map(Number);
    if (ids.length === 0) {
      setPendingRemove(false);
      return;
    }
    try {
      await removeTracks(ids);
      // 重取真相，不做减法推算：
      // 有筛选 → 按筛选结果重新计数；无筛选 → 刷新统计，行数由上方 effect 复用 stats.tracks
      if (filter) {
        const n = await countTracks(filter);
        reset(n);
      }
      const t2 = statsGuard.token();
      libraryStats()
        .then((s) => {
          if (!statsGuard.isStale(t2)) setStats(s);
        })
        .catch(() => {});
      selApi.toggleSelMode();
    } catch (e) {
      setInitErr(String(e));
    } finally {
      setPendingRemove(false);
    }
  };

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.libTitle}</h2>
          <p className="sub">{t.media.libSub(w.total, fmtSizeGB(stats?.totalSize ?? 0))}</p>
        </div>
        <div className="act">
          <SortControl
            value={sort}
            onChange={setSort}
            note={sortRejected ? t.sort.unsupported : null}
            fields={[
              { value: "default", label: t.sort.def },
              { value: "title", label: t.sort.title },
              { value: "artist", label: t.sort.artist },
              { value: "album", label: t.sort.album },
              { value: "duration", label: t.sort.duration },
              { value: "play_count", label: t.sort.playCount },
            ]}
          />
          <label className="search-inline">
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.9"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="6.5" />
              <path d="M15.8 15.8L20 20" />
            </svg>
            <input
              type="search"
              value={rawQuery}
              onChange={(e) => setRawQuery(e.target.value)}
              placeholder={t.media.searchPlaceholder}
              aria-label={t.media.searchPlaceholder}
            />
          </label>
          <button
            className={"btn sm" + (selApi.selMode ? " on" : "")}
            onClick={selApi.toggleSelMode}
            disabled={w.total === 0}
          >
            {t.player.select}
          </button>
        </div>
      </div>

      <div className="panel" style={{ padding: 0, overflow: "hidden" }}>
        <div className="vt-head">
          <span>{t.media.colIndex}</span>
          <span>{t.media.colTitle}</span>
          <span>{t.media.colAlbum}</span>
          <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
          <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
        </div>

        {/* P6.25：过滤下推服务端后恒走虚拟化。
            P6.28：空库此前渲染成 0 高度的空白面板（无任何提示），补空态区分两种零行：
            有筛选词 = 没匹配到；无筛选词 = 曲库本身就是空的。 */}
        {w.total === 0 ? (
          <div className="media-empty">
            <p>{filter ? t.listFilter.noResult : t.media.libEmpty}</p>
          </div>
        ) : (
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
                      onAdd={handleAdd}
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
      </div>

      {/* P6.4：加入歌单弹层 */}
      <AddToPlaylistDialog
        tracks={addTarget ? [addTarget] : null}
        onClose={() => setAddTarget(null)}
      />
      {selApi.selMode && (
        <SelectionBar
          count={selApi.count}
          total={list.length}
          onAddToQueue={bulkQueue}
          onPlayNext={bulkPlayNext}
          onRemove={() => setPendingRemove(true)}
          removeLabel={t.library.removeTitle}
          onSelectAll={() => selApi.selectAll(list.map((x) => String(x.id)))}
          onClear={selApi.toggleSelMode}
        />
      )}
      <ConfirmDialog
        open={pendingRemove}
        title={t.library.removeTitle}
        summary={t.library.removeSummary(selApi.count)}
        items={removeItems}
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
