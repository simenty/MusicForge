// 艺术家（P1 网格 / P6.2 代表图 / P6.10 详情页）：
// 有封面则显示（其最热专辑的封面），否则圆形首字；点卡片进入详情（播放全部 + 曲目表）。
// 「补全头像」需在设置中开启「在线元数据」——网络请求只由该按钮触发。
import { useEffect, useRef, useState } from "react";
import {
  IS_DESKTOP,
  artistCover,
  artistCoversLocal,
  artistTracks,
  listArtists,
} from "./api";
import type { Artist, Track } from "./api";
import { useLang } from "./i18n";
import { useSettings } from "./hooks/useSettings";
import { assetUrl } from "./lib/asset";
import TrackRow from "./TrackRow";
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import { IconDownload } from "./icons";

export default function ArtistsPage({
  onPlay,
  focusId = null,
  onQueue,
  onPlayNext,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.14 搜索跳转：命中的艺术家 id（消费一次即展开详情） */
  focusId?: number | null;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const { settings } = useSettings();
  const [rows, setRows] = useState<Artist[] | null>(null);
  /** artistId → asset URL（已有封面） */
  const [covers, setCovers] = useState<Record<number, string>>({});
  const [fetching, setFetching] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  /** 详情态：选中的艺术家（null = 列表态） */
  const [sel, setSel] = useState<Artist | null>(null);
  const [tracks, setTracks] = useState<Track[] | null>(null);
  const [addTarget, setAddTarget] = useState<Track | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    listArtists()
      .then((list) => {
        setRows(list);
        // 已有封面批量回填（一次 IPC；纯本地查询，不发网络）
        void artistCoversLocal(list.map((a) => a.id))
          .then((m) => {
            const out: Record<number, string> = {};
            for (const [k, p] of Object.entries(m)) {
              const url = assetUrl(p);
              if (url) out[Number(k)] = url;
            }
            setCovers(out);
          })
          .catch(() => {
            /* 回填失败：全部显示首字头像 */
          });
      })
      .catch(() => setRows([]));
  }, []);

  const openArtist = (a: Artist) => {
    setSel(a);
    setTracks(null);
    setErr(null);
    artistTracks(a.id)
      .then(setTracks)
      .catch(() => setTracks([]));
  };

  // P6.14 搜索跳转：列表就绪后展开命中艺术家。
  // 用 ref 标记「已消费」而非把 openArtist 列进依赖——后者每次渲染都变，会反复重拉。
  const consumedFocus = useRef<number | null>(null);
  useEffect(() => {
    if (focusId === null || !rows || consumedFocus.current === focusId) return;
    const hit = rows.find((a) => a.id === focusId);
    if (!hit) return;
    consumedFocus.current = focusId;
    setSel(hit);
    setTracks(null);
    void artistTracks(hit.id)
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

  const online = settings.onlineMeta;
  const missing = rows.filter((a) => !covers[a.id]);

  /** 逐个补全（在线；已抓过的秒回）。网络失败即停。 */
  const fetchAll = async () => {
    if (!online || fetching || missing.length === 0) return;
    setFetching(true);
    setErr(null);
    let done = 0;
    const total = missing.length;
    setProgress({ done, total });
    for (const a of missing) {
      try {
        const p = await artistCover(a.id);
        const url = assetUrl(p);
        if (url) setCovers((prev) => ({ ...prev, [a.id]: url }));
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
    const url = covers[sel.id];
    return (
      <>
        <div className="media-head">
          <div className="detail-top">
            {url ? (
              <img className="detail-cover" src={url} alt="" />
            ) : (
              <span className="detail-cover g3" aria-hidden="true">
                {sel.name.slice(0, 1).toUpperCase()}
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
              <h2 style={{ marginTop: 8 }}>{sel.name}</h2>
              <p className="sub">{t.media.tracksN(tracks?.length ?? sel.trackCount)}</p>
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
                onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
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

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.artistsTitle}</h2>
          <p className="sub">
            {t.media.artistsSub(rows.length)}
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
              ? t.media.artistsProgress(progress.done, progress.total)
              : t.media.artistsFetchAll}
          </button>
        </div>
      </div>
      {!online && <p className="scan-note">{t.media.coversNeedOnline}</p>}
      {err && <p className="scan-error">{err}</p>}
      <div className="mgrid">
        {rows.map((a, i) => (
          <div className="mcard pl-card" key={a.id} onClick={() => openArtist(a)}>
            {covers[a.id] ? (
              <span className="ava" aria-hidden="true">
                <img src={covers[a.id]} alt="" loading="lazy" />
              </span>
            ) : (
              <span className={"ava g" + ((i % 6) + 1)} aria-hidden="true">
                {a.name.slice(0, 1).toUpperCase()}
              </span>
            )}
            <span className="nm" title={a.name}>
              {a.name}
            </span>
            <span className="ct">{t.media.tracksN(a.trackCount)}</span>
          </div>
        ))}
      </div>
    </>
  );
}
