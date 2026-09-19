// 艺术家（P1 聚合网格 / P6 代表图）：有封面则显示（其最热专辑的封面），否则圆形首字。
// 「补全头像」需在设置中开启「在线元数据」——网络请求只由该按钮触发
// （1 req/s 限速由后端强制，mount 时的本地回填不发网络）。
import { useEffect, useState } from "react";
import { IS_DESKTOP, artistCover, artistCoversLocal, listArtists } from "./api";
import type { Artist } from "./api";
import { useLang } from "./i18n";
import { useSettings } from "./hooks/useSettings";
import { assetUrl } from "./lib/asset";
import { IconDownload } from "./icons";

export default function ArtistsPage() {
  const { t } = useLang();
  const { settings } = useSettings();
  const [rows, setRows] = useState<Artist[] | null>(null);
  /** artistId → asset URL（已有封面） */
  const [covers, setCovers] = useState<Record<number, string>>({});
  const [fetching, setFetching] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);

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
          <div className="mcard" key={a.id}>
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
