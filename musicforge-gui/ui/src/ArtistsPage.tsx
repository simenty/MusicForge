// 艺术家（P1）：聚合网格 —— 圆形首字头像（占位，真机接入图片后替换）。
import { useEffect, useState } from "react";
import { IS_DESKTOP, listArtists } from "./api";
import type { Artist } from "./api";
import { useLang } from "./i18n";

export default function ArtistsPage() {
  const { t } = useLang();
  const [rows, setRows] = useState<Artist[] | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    listArtists()
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

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.artistsTitle}</h2>
          <p className="sub">{t.media.artistsSub(rows.length)}</p>
        </div>
      </div>
      <div className="mgrid">
        {rows.map((a, i) => (
          <div className="mcard" key={a.id}>
            <span className={"ava g" + ((i % 6) + 1)} aria-hidden="true">
              {a.name.slice(0, 1).toUpperCase()}
            </span>
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
