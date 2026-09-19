// 专辑（P1 聚合网格 / P6 在线封面）：coverPath 有值时显示本地缓存封面，
// 否则渐变占位。「补全封面」按需抓取（MusicBrainz → Cover Art Archive），
// 需要先在设置里开启「在线元数据」——网络请求只由这个按钮触发。
import { useEffect, useState } from "react";
import { IS_DESKTOP, coverFetch, coverPickImage, coverSetLocal, listAlbums } from "./api";
import type { Album } from "./api";
import { useLang } from "./i18n";
import { useSettings } from "./hooks/useSettings";
import { assetUrl } from "./lib/asset";
import { IconDisc, IconDownload } from "./icons";

export default function AlbumsPage() {
  const { t } = useLang();
  const { settings } = useSettings();
  const [rows, setRows] = useState<Album[] | null>(null);
  const [fetching, setFetching] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    listAlbums()
      .then(setRows)
      .catch(() => setRows([]));
  }, []);

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
  if (rows.length === 0) {
    return (
      <div className="media-empty">
        <p>{t.media.searchEmpty}</p>
      </div>
    );
  }

  const missing = rows.filter((a) => !a.coverPath);
  const online = settings.onlineMeta;

  /** 设置本地封面（**离线能力**：选一张图片，不走网络）。 */
  const pickCover = async (a: Album) => {
    const p = await coverPickImage();
    if (!p) return;
    try {
      const stored = await coverSetLocal(a.id, p);
      setRows(
        (prev) => prev?.map((x) => (x.id === a.id ? { ...x, coverPath: stored } : x)) ?? prev
      );
    } catch (e) {
      setErr(String(e));
    }
  };

  /** 逐个补全缺失封面。网络失败即停（限速下没有重试余量，由用户稍后再来）。 */
  const fetchAll = async () => {
    if (!online || fetching || missing.length === 0) return;
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
      <div className="mgrid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}>
        {rows.map((a, i) => {
          const src = assetUrl(a.coverPath);
          return (
            <div className="mcard" key={a.id}>
              {src ? (
                <img className="cover2" src={src} alt="" loading="lazy" />
              ) : (
                <span className={"cover2 g" + ((i % 6) + 1)} aria-hidden="true">
                  <IconDisc size={38} />
                </span>
              )}
              {/* P6：本地封面（离线；hover 显现） */}
              <button
                className="cover-edit"
                onClick={() => void pickCover(a)}
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
