// 专辑（P1 网格 / P6 在线封面 / P6.10 详情页）：
// 点卡片进入详情（大封面 + 播放全部 + 按碟/轨号排序的曲目表）。
import { useEffect, useRef, useState } from "react";
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
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import { IconDisc, IconDownload } from "./icons";

export default function AlbumsPage({
  onPlay,
  focusId = null,
  onQueue,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.14 搜索跳转：命中的专辑 id（消费一次即展开详情） */
  focusId?: number | null;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const { settings } = useSettings();
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
    listAlbums()
      .then(setRows)
      .catch(() => setRows([]));
  }, []);

  const openAlbum = (a: Album) => {
    setSel(a);
    setTracks(null);
    setErr(null);
    albumTracks(a.id)
      .then(setTracks)
      .catch(() => setTracks([]));
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
    void albumTracks(hit.id)
      .then(setTracks)
      .catch(() => setTracks([]));
  }, [focusId, rows]);

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
    const p = await coverPickImage();
    if (!p) return;
    try {
      const stored = await coverSetLocal(a.id, p);
      setRows(
        (prev) => prev?.map((x) => (x.id === a.id ? { ...x, coverPath: stored } : x)) ?? prev
      );
      setSel((prev) => (prev && prev.id === a.id ? { ...prev, coverPath: stored } : prev));
    } catch (e) {
      setErr(String(e));
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

  // ---------------------------------------------------------------- 详情态 --
  if (sel) {
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
            <button
              className="btn sm primary"
              disabled={!tracks || tracks.length === 0}
              onClick={() => {
                if (onPlay && tracks && tracks.length > 0) void onPlay(tracks, 0);
              }}
            >
              {t.media.playAll}
            </button>
            <button className="btn sm" onClick={() => void pickCover(sel)}>
              {t.media.coverLocal}
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
            {tracks.map((r, i) => (
              <TrackRow
                key={r.id}
                lead={i + 1}
                track={r}
                onPlay={onPlay ? () => void onPlay(tracks, i) : undefined}
                onAdd={() => setAddTarget(r)}
                onQueue={onQueue ? () => void onQueue([r]) : undefined}
              />
            ))}
          </div>
        )}
        <AddToPlaylistDialog
          tracks={addTarget ? [addTarget] : null}
          onClose={() => setAddTarget(null)}
        />
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

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.albumsTitle}</h2>
          <p className="sub">
            {t.media.albumsSub(rows.length)}
            {missing.length > 0 ? ` · ${t.media.coversMissing(missing.length)}` : ""}
          </p>
        </div>
        <div className="act">
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
      <div
        className="mgrid"
        style={{ gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}
      >
        {rows.map((a, i) => {
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
    </>
  );
}
