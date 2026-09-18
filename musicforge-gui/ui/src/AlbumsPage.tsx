// 专辑（P1）：聚合网格 —— 渐变封面占位（真机接入内嵌封面后替换为图片）。
import { useEffect, useState } from "react";
import { IS_DESKTOP, listAlbums } from "./api";
import type { Album } from "./api";
import { useLang } from "./i18n";
import { IconDisc } from "./icons";

export default function AlbumsPage() {
  const { t } = useLang();
  const [rows, setRows] = useState<Album[] | null>(null);

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

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.albumsTitle}</h2>
          <p className="sub">{t.media.albumsSub(rows.length)}</p>
        </div>
      </div>
      <div className="mgrid" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}>
        {rows.map((a, i) => (
          <div className="mcard" key={a.id}>
            <span className={"cover2 g" + ((i % 6) + 1)} aria-hidden="true">
              <IconDisc size={38} />
            </span>
            <span className="nm" title={a.title}>
              {a.title}
            </span>
            <span className="ct">
              {a.artist ?? t.media.unknownArtist}
              {a.year ? ` · ${a.year}` : ""}
            </span>
            <span className="ct">{t.media.tracksN(a.trackCount)}</span>
          </div>
        ))}
      </div>
    </>
  );
}
