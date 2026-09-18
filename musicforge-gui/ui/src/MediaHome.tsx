// 媒体库概览（P1 曲库体验）：曲库统计卡 + 前 8 首预览；空库时给添加引导。
import { useEffect, useState } from "react";
import { IS_DESKTOP, libraryStats, listTracks } from "./api";
import type { LibraryStats, Track } from "./api";
import { useLang } from "./i18n";
import { fmtClock, fmtHours, fmtSizeGB } from "./lib/format";
import { IconClock, IconDisc, IconMusic, IconUser } from "./icons";

/**
 * 概览页。`goSources` 由外层注入（切到媒体源页）——本组件不持有导航状态。
 */
export default function MediaHome({ goSources }: { goSources: () => void }) {
  const { t } = useLang();
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [preview, setPreview] = useState<Track[]>([]);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    libraryStats()
      .then((s) => {
        setStats(s);
        if (s.tracks > 0) void listTracks(8, 0).then(setPreview);
      })
      .catch((e: unknown) => setErr(String(e)));
  }, []);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }
  if (err) {
    return (
      <div className="media-empty">
        <p>{err}</p>
      </div>
    );
  }
  if (!stats) {
    return (
      <div className="media-empty">
        <p>{t.media.loading}</p>
      </div>
    );
  }
  if (stats.tracks === 0) {
    return (
      <div className="media-empty">
        <h3>{t.media.emptyTitle}</h3>
        <p>{t.media.emptyBody}</p>
        <button className="btn primary" onClick={goSources}>
          {t.media.goSources}
        </button>
      </div>
    );
  }

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.homeTitle}</h2>
          <p className="sub">
            {t.media.homeSub} · {fmtSizeGB(stats.totalSize)}
          </p>
        </div>
      </div>

      <div className="stat-grid">
        <div className="stat-card">
          <span className="stat-ico i-total">
            <IconMusic size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statTracks}</span>
            <span className="stat-num">{stats.tracks.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-audio">
            <IconUser size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statArtists}</span>
            <span className="stat-num">{stats.artists.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-cover">
            <IconDisc size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statAlbums}</span>
            <span className="stat-num">{stats.albums.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-time">
            <IconClock size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statDuration}</span>
            <span className="stat-num">{fmtHours(stats.totalDurationMs)}</span>
          </span>
        </div>
      </div>

      <section className="section" style={{ marginTop: "var(--sp-5)" }}>
        <div className="media-head" style={{ marginBottom: 0 }}>
          <div>
            <h2 style={{ fontSize: 16 }}>{t.media.previewTitle}</h2>
            <p className="sub">{t.media.previewSub}</p>
          </div>
        </div>
        <div className="panel" style={{ padding: 0, overflow: "hidden" }}>
          {preview.map((tr, i) => (
            <div className="vt-row" key={tr.id}>
              <span className="vt-idx">{i + 1}</span>
              <span className="vt-main">
                <b title={tr.title ?? undefined}>{tr.title ?? "—"}</b>
                <span>{tr.artist ?? "—"}</span>
              </span>
              <span className="vt-alb" title={tr.album ?? undefined}>
                {tr.album ?? "—"}
              </span>
              <span className="vt-num">{fmtClock(tr.durationMs)}</span>
              <span className="vt-num">{(tr.format ?? "").toUpperCase()}</span>
            </div>
          ))}
        </div>
      </section>
    </>
  );
}
