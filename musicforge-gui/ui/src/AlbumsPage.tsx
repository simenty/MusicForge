// 专辑（P1 网格 / P6 在线封面 / P6.10 详情页）：
// 点卡片进入详情（大封面 + 播放全部 + 按碟/轨号排序的曲目表）。
import { useEffect, useRef, useState, useCallback } from "react";
import {
  IS_DESKTOP,
  albumTracks,
  coverFetch,
  coverPickImage,
  coverSetLocal,
  listAlbums,
} from "./api";
import type { Album, Track } from "./api";
import { useLang } from "./i18n";
import { useSettings } from "./hooks/useSettings";
import { assetUrl } from "./lib/asset";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import SortControl from "./SortControl";
import FilterInput from "./FilterInput";
import { useSort } from "./hooks/useSort";
import { useRequestGuard } from "./hooks/useRequestGuard";
import { sortTracks } from "./lib/sortTracks";
import { filterAlbums, filterTracks } from "./lib/filterTracks";
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import { IconDisc, IconDownload } from "./icons";

export default function AlbumsPage({
  onPlay,
  focusId = null,
  onQueue,
  onPlayNext,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.14 搜索跳转：命中的专辑 id（消费一次即展开详情） */
  focusId?: number | null;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const { settings } = useSettings();
  const selApi = useSelection();
  const [sort, setSort] = useSort("album-detail", "default");
  /** P6.26 详情页筛选：整段曲目已在内存 → 纯前端过滤，无需防抖/IPC */
  const [filter, setFilter] = useState("");
  /** P6.27 列表网格筛选（与详情页筛选**互相独立**：两个态不同时可见，
   *  共用一个状态会导致「筛网格 → 进入详情后曲目被意外过滤」） */
  const [gridFilter, setGridFilter] = useState("");
  const [rows, setRows] = useState<Album[] | null>(null);
  const [fetching, setFetching] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  /** 详情态：选中的专辑（null = 列表态） */
  const [sel, setSel] = useState<Album | null>(null);
  const [tracks, setTracks] = useState<Track[] | null>(null);
  /** 加入歌单弹层 */
  const [addTarget, setAddTarget] = useState<Track | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    const token = listGuard.token();
    listAlbums()
      .then((r) => {
        if (!listGuard.isStale(token)) setRows(r);
      })
      .catch(() => {
        if (!listGuard.isStale(token)) setRows([]);
      });
  }, []);

  /** stale response 守卫：连点不同专辑时，先发但**晚到**的请求不得覆盖后发的结果 */
  const tracksGuard = useRequestGuard();
  // P2-17：列表/封面写入守卫
  const listGuard = useRequestGuard();
  const coverGuard = useRequestGuard();

  const openAlbum = (a: Album) => {
    setSel(a);
    setTracks(null);
    setErr(null);
    tracksGuard.bump();
    const token = tracksGuard.token();
    albumTracks(a.id)
      .then((rows) => {
        if (!tracksGuard.isStale(token)) setTracks(rows);
      })
      .catch(() => {
        if (!tracksGuard.isStale(token)) setTracks([]);
      });
  };

  // P6.14 搜索跳转：列表就绪后展开命中专辑（ref 标记已消费，避免反复重拉）
  const consumedFocus = useRef<number | null>(null);
  useEffect(() => {
    if (focusId === null || !rows || consumedFocus.current === focusId) return;
    const hit = rows.find((a) => a.id === focusId);
    if (!hit) return;
    consumedFocus.current = focusId;
    setSel(hit);
    setTracks(null);
    tracksGuard.bump();
    const token = tracksGuard.token();
    void albumTracks(hit.id)
      .then((rows) => {
        if (!tracksGuard.isStale(token)) setTracks(rows);
      })
      .catch(() => {
        if (!tracksGuard.isStale(token)) setTracks([]);
      });
  }, [focusId, rows, tracksGuard]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }
  if (!rows) {
    return (
      <div className="media-empty">
        <p>{t.media.loading}</p>
      </div>
    );
  }

  /** 设置本地封面（**离线能力**：选一张图片，不走网络）。 */
  const pickCover = async (a: Album) => {
    const token = coverGuard.token();
    const p = await coverPickImage();
    if (!p) return;
    try {
      const stored = await coverSetLocal(a.id, p);
      if (!coverGuard.isStale(token)) {
        setRows(
          (prev) => prev?.map((x) => (x.id === a.id ? { ...x, coverPath: stored } : x)) ?? prev
        );
        setSel((prev) => (prev && prev.id === a.id ? { ...prev, coverPath: stored } : prev));
      }
    } catch (e) {
      if (!coverGuard.isStale(token)) setErr(String(e));
    }
  };

  /** 逐个补全缺失封面。网络失败即停（限速下没有重试余量，由用户稍后再来）。 */
  const fetchAll = async () => {
    const missing = rows.filter((a) => !a.coverPath);
    if (!settings.onlineMeta || fetching || missing.length === 0) return;
    setFetching(true);
    setErr(null);
    let done = 0;
    const total = missing.length;
    setProgress({ done, total });
    for (const a of missing) {
      try {
        const p = await coverFetch(a.id);
        if (p) {
          setRows(
            (prev) => prev?.map((x) => (x.id === a.id ? { ...x, coverPath: p } : x)) ?? prev
          );
        }
      } catch (e) {
        setErr(String(e));
        break;
      }
      done += 1;
      setProgress({ done, total });
    }
    setFetching(false);
    setProgress(null);
  };

  // P2-16：行内动作稳定化（详情态映射用），配合 TrackRow memo
  const handlePlay = useCallback(
    (tr: Track) => {
      if (!onPlay || !tracks) return;
      const arr = tracks;
      const idx = arr.findIndex((x) => x.id === tr.id);
      void onPlay(arr, idx >= 0 ? idx : 0);
    },
    [onPlay, tracks]
  );
  const handleAdd = useCallback((tr: Track) => setAddTarget(tr), []);
  const handleQueue = useCallback((tr: Track) => void onQueue?.([tr]), [onQueue]);
  const handlePlayNext = useCallback((tr: Track) => void onPlayNext?.([tr]), [onPlayNext]);
  const handleToggleSelect = useCallback(
    (tr: Track) => selApi.toggle(String(tr.id)),
    [selApi.toggle]
  );

  // ---------------------------------------------------------------- 详情态 --
  if (sel) {
    // P6.19 批量操作；P6.21：详情页按 sort 前端排序（整段已在内存）
    const list = filterTracks(sortTracks(tracks ?? [], sort), filter);
    const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
    const bulkQueue = async () => {
      if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
      selApi.toggleSelMode();
    };
    const bulkPlayNext = async () => {
      if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
      selApi.toggleSelMode();
    };
    const src = assetUrl(sel.coverPath);
    return (
      <>
        <div className="media-head">
          <div className="detail-top">
            {src ? (
              <img className="detail-cover" src={src} alt="" />
            ) : (
              <span className="detail-cover g3" aria-hidden="true">
                <IconDisc size={44} />
              </span>
            )}
            <div>
              <button
                className="btn sm"
                onClick={() => {
                  setSel(null);
                  setTracks(null);
                }}
              >
                ← {t.media.back}
              </button>
              <h2 style={{ marginTop: 8 }}>{sel.title}</h2>
              <p className="sub">
                {sel.artist ?? t.media.unknownArtist}
                {sel.year ? ` · ${sel.year}` : ""} ·{" "}
                {t.media.tracksN(tracks?.length ?? sel.trackCount)}
              </p>
            </div>
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
              ]}
            />
            <FilterInput
              value={filter}
              onChange={setFilter}
              placeholder={t.listFilter.placeholder}
              clearLabel={t.listFilter.clear}
            />
            <button
              className="btn sm primary"
              disabled={!tracks || tracks.length === 0}
              onClick={() => {
                if (onPlay && list.length > 0) void onPlay(list, 0);
              }}
            >
              {t.media.playAll}
            </button>
            <button className="btn sm" onClick={() => void pickCover(sel)}>
              {t.media.coverLocal}
            </button>
            <button
              className={"btn sm" + (selApi.selMode ? " on" : "")}
              onClick={selApi.toggleSelMode}
              disabled={!tracks || tracks.length === 0}
            >
              {t.player.select}
            </button>
          </div>
        </div>
        {err && <p className="scan-error">{err}</p>}
        {tracks === null ? (
          <p className="scan-note">{t.media.loading}</p>
        ) : tracks.length === 0 ? (
          <div className="media-empty">
            <p>{t.media.searchEmpty}</p>
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
            {list.map((r, i) => (
              <TrackRow
                key={r.id}
                lead={i + 1}
                track={r}
                onPlay={onPlay ? handlePlay : undefined}
                onAdd={handleAdd}
                onQueue={onQueue ? handleQueue : undefined}
                onPlayNext={onPlayNext ? handlePlayNext : undefined}
                selectable={selApi.selMode}
                selected={selApi.has(String(r.id))}
                onToggleSelect={handleToggleSelect}
              />
            ))}
          </div>
        )}
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

  // ---------------------------------------------------------------- 列表态 --
  if (rows.length === 0) {
    return (
      <div className="media-empty">
        <p>{t.media.searchEmpty}</p>
      </div>
    );
  }
  const missing = rows.filter((a) => !a.coverPath);
  const online = settings.onlineMeta;
  /** P6.27：网格筛选后的可见集合（专辑列表一次性取全 → 纯前端过滤） */
  const visible = filterAlbums(rows, gridFilter);

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.albumsTitle}</h2>
          <p className="sub">
            {t.media.albumsSub(visible.length)}
            {missing.length > 0 ? ` · ${t.media.coversMissing(missing.length)}` : ""}
          </p>
        </div>
        <div className="act">
          <FilterInput
            value={gridFilter}
            onChange={setGridFilter}
            placeholder={t.listFilter.placeholder}
            clearLabel={t.listFilter.clear}
          />
          <button
            className="btn sm"
            onClick={() => void fetchAll()}
            disabled={!online || fetching || missing.length === 0}
            title={online ? undefined : t.media.coversNeedOnline}
          >
            <IconDownload size={14} />
            {fetching && progress
              ? t.media.coversProgress(progress.done, progress.total)
              : t.media.coversFetchAll}
          </button>
        </div>
      </div>
      {!online && <p className="scan-note">{t.media.coversNeedOnline}</p>}
      {err && <p className="scan-error">{err}</p>}
      {visible.length === 0 ? (
        <div className="media-empty">
          <p>{t.listFilter.noResult}</p>
        </div>
      ) : (
        <div
          className="mgrid"
          style={{ gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}
        >
          {visible.map((a, i) => {
          const src = assetUrl(a.coverPath);
          return (
            <div className="mcard pl-card" key={a.id} onClick={() => openAlbum(a)}>
              {src ? (
                <img className="cover2" src={src} alt="" loading="lazy" />
              ) : (
                <span className={"cover2 g" + ((i % 6) + 1)} aria-hidden="true">
                  <IconDisc size={38} />
                </span>
              )}
              <button
                className="cover-edit"
                onClick={(e) => {
                  e.stopPropagation();
                  void pickCover(a);
                }}
                title={t.media.coverLocal}
                aria-label={t.media.coverLocal}
              >
                <svg
                  width="13"
                  height="13"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <rect x="3" y="5" width="18" height="14" rx="2" />
                  <circle cx="9" cy="10" r="1.6" />
                  <path d="M4 17l5-4 4 3 3-2 4 3" />
                </svg>
              </button>
              <span className="nm" title={a.title}>
                {a.title}
              </span>
              <span className="ct">
                {a.artist ?? t.media.unknownArtist}
                {a.year ? ` · ${a.year}` : ""}
              </span>
              <span className="ct">{t.media.tracksN(a.trackCount)}</span>
            </div>
          );
          })}
        </div>
      )}
    </>
  );
}
