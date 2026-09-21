// 音乐库（P1 核心页）：十万级曲目浏览 = 分页取数 + 自建虚拟滚动；
// 搜索走 core 的 search_tracks（一次 ≤500，不走虚拟化）。
// P2：双击行 → 以「已缓存行快照」为队列开始播放。
import { useCallback, useEffect, useState } from "react";
import { IS_DESKTOP, libraryStats, searchTracks } from "./api";
import type { LibraryStats, Track } from "./api";
import { useLang } from "./i18n";
import { fmtSizeGB } from "./lib/format";
import { useWindowedTracks, TRACK_ROW_H } from "./hooks/useWindowedTracks";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import SortControl from "./SortControl";
import { useSort } from "./hooks/useSort";

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
  const [sort, setSort] = useSort("library", "default");
  const w = useWindowedTracks(200, sort);
  const liked = useLiked();
  // P6.19 批量操作（hook 须无条件调用，置于早返回之前）
  const selApi = useSelection();
  /** P6.4：待加入歌单的曲目（null = 弹层关闭） */
  const [addTarget, setAddTarget] = useState<Track | null>(null);
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [initErr, setInitErr] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Track[] | null>(null);
  const [searching, setSearching] = useState(false);

  const { reset, snapshot } = w;
  useEffect(() => {
    if (!IS_DESKTOP) return;
    libraryStats()
      .then((s) => {
        setStats(s);
        reset(s.tracks);
      })
      .catch((e: unknown) => setInitErr(String(e)));
  }, [reset]);

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

  // 搜索：250ms 防抖；空查询回退虚拟列表
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const id = window.setTimeout(() => {
      searchTracks(q, 500)
        .then(setResults)
        .catch(() => setResults([]))
        .finally(() => setSearching(false));
    }, 250);
    return () => window.clearTimeout(id);
  }, [query]);

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

  // 可见列表（搜索态用结果，否则用虚拟化已加载快照）；全选覆盖该集合
  const list = results ?? snapshot();
  const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
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
              value={query}
              onChange={(e) => setQuery(e.target.value)}
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

        {results !== null ? (
          results.length === 0 ? (
            <div className="media-empty">
              <p>{searching ? t.media.loading : t.media.searchEmpty}</p>
            </div>
          ) : (
            <>
              <div className="vt-loading">{t.media.searchResult(results.length)}</div>
              {results.map((r, i) => (
                <TrackRow
                  key={r.id}
                  lead={i + 1}
                  track={r}
                  onPlay={onPlay ? () => playFrom(r) : undefined}
                  liked={liked.isLiked(r.id)}
                  onLike={() => void liked.toggle(r.id)}
                  onAdd={() => setAddTarget(r)}
                  onQueue={onQueue ? () => void onQueue([r]) : undefined}
                  onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
                  selectable={selApi.selMode}
                  selected={selApi.has(String(r.id))}
                  onToggleSelect={() => selApi.toggle(String(r.id))}
                />
              ))}
            </>
          )
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
                      onPlay={onPlay ? () => playFrom(tr) : undefined}
                      liked={liked.isLiked(tr.id)}
                      onLike={() => void liked.toggle(tr.id)}
                      onAdd={() => setAddTarget(tr)}
                      onQueue={onQueue ? () => void onQueue([tr]) : undefined}
                      onPlayNext={onPlayNext ? () => void onPlayNext([tr]) : undefined}
                      selectable={selApi.selMode}
                      selected={selApi.has(String(tr.id))}
                      onToggleSelect={() => selApi.toggle(String(tr.id))}
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
          onSelectAll={() => selApi.selectAll(list.map((x) => String(x.id)))}
          onClear={selApi.toggleSelMode}
        />
      )}
    </>
  );
}
