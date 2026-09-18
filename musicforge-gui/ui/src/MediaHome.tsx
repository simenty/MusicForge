// 媒体库概览（P1 建立 / P3 升级）：问候 + 统计卡 + 快捷入口 + 继续聆听。
import { useEffect, useState } from "react";
import { IS_DESKTOP, listTracks, recentPlays, statsOverview } from "./api";
import type { StatsOverview, Track } from "./api";
import { useLang } from "./i18n";
import { fmtHours, fmtSizeGB } from "./lib/format";
import { IconClock, IconDisc, IconFolder, IconHeart, IconMusic, IconUser } from "./icons";
import TrackRow from "./TrackRow";

/**
 * 概览页。`goSources` / `onNavigate` / `onPlay` 由外层注入——
 * 本组件不持有导航或播放状态。
 */
export default function MediaHome({
  goSources,
  onNavigate,
  onPlay,
}: {
  goSources: () => void;
  onNavigate?: (tab: "favorites" | "history" | "sources") => void;
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
}) {
  const { t } = useLang();
  const [stats, setStats] = useState<StatsOverview | null>(null);
  const [recent, setRecent] = useState<Track[]>([]);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    // statsOverview 一次拿全（曲库规模 + 喜欢数）——避免两次 IPC
    statsOverview()
      .then((s) => {
        setStats(s);
        if (s.tracks === 0) return;
        // 继续聆听：优先「最近播放」，无历史时回退到按路径序的前 8 首
        return recentPlays(8).then((r) => {
          if (r.length > 0) setRecent(r);
          else return listTracks(8, 0).then(setRecent);
        });
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

  const hour = new Date().getHours();
  const greeting =
    hour < 12
      ? t.media.greetingMorning
      : hour < 18
        ? t.media.greetingAfternoon
        : t.media.greetingEvening;

  const playFrom = (tr: Track) => {
    if (!onPlay) return;
    const idx = recent.findIndex((x) => x.id === tr.id);
    void onPlay(recent, idx >= 0 ? idx : 0);
  };

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{greeting}</h2>
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

      {onNavigate && (
        <div className="quick-grid">
          <button className="quick-card" onClick={() => onNavigate("favorites")}>
            <span className="ic">
              <IconHeart size={18} />
            </span>
            <span>
              <b>{t.media.quickFav}</b>
              <span>{t.media.favCount(stats.liked)}</span>
            </span>
          </button>
          <button className="quick-card" onClick={() => onNavigate("history")}>
            <span className="ic">
              <IconClock size={18} />
            </span>
            <span>
              <b>{t.media.quickHistory}</b>
              <span>{t.media.continueSub}</span>
            </span>
          </button>
          <button className="quick-card" onClick={() => onNavigate("sources")}>
            <span className="ic">
              <IconFolder size={18} />
            </span>
            <span>
              <b>{t.media.quickSources}</b>
              <span>{t.media.sourcesSub}</span>
            </span>
          </button>
        </div>
      )}

      <section style={{ marginTop: "var(--sp-5)" }}>
        <div className="section-head">
          <h2 style={{ fontSize: 16 }}>{t.media.continueTitle}</h2>
          <span className="hint" style={{ fontSize: "var(--fs-xs)", color: "var(--text-faint)" }}>
            {t.media.continueSub}
          </span>
        </div>
        <div className="panel" style={{ padding: 0, overflow: "hidden", marginTop: "var(--sp-3)" }}>
          <div className="vt-head">
            <span>{t.media.colIndex}</span>
            <span>{t.media.colTitle}</span>
            <span>{t.media.colAlbum}</span>
            <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
            <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
            <span />
          </div>
          {recent.map((tr, i) => (
            <TrackRow
              key={tr.id}
              lead={i + 1}
              track={tr}
              onPlay={onPlay ? () => playFrom(tr) : undefined}
            />
          ))}
        </div>
      </section>
    </>
  );
}
