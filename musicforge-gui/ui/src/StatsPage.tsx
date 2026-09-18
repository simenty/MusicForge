// 统计（P3）：曲库规模 + 行为计数 + 近 7 天条形图（纯 CSS）+ 最常播放 Top 10。
import { useEffect, useState } from "react";
import { IS_DESKTOP, statsOverview } from "./api";
import type { StatsOverview, Track } from "./api";
import { useLang } from "./i18n";
import { fmtHours, fmtSizeGB } from "./lib/format";
import TrackRow from "./TrackRow";
import { IconClock, IconDisc, IconMusic, IconUser } from "./icons";

/** 近 7 天日期键（本地时区 `YYYY-MM-DD`，与后端 `date(...,'localtime')` 同口径） */
function last7Days(): string[] {
  const out: string[] = [];
  const now = new Date();
  for (let i = 6; i >= 0; i--) {
    const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() - i);
    const mm = String(d.getMonth() + 1).padStart(2, "0");
    const dd = String(d.getDate()).padStart(2, "0");
    out.push(`${d.getFullYear()}-${mm}-${dd}`);
  }
  return out;
}

export default function StatsPage({
  onPlay,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
}) {
  const { t } = useLang();
  const [data, setData] = useState<StatsOverview | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    statsOverview()
      .then(setData)
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
  if (!data) {
    return (
      <div className="media-empty">
        <p>{t.media.loading}</p>
      </div>
    );
  }

  const days = last7Days();
  const counts = new Map(data.daily.map((d) => [d.day, d.count]));
  const weekVals = days.map((d) => counts.get(d) ?? 0);
  const max = Math.max(1, ...weekVals);

  const playTop = (tr: Track) => {
    if (!onPlay) return;
    const queue: Track[] = data.top;
    const idx = queue.findIndex((x) => x.id === tr.id);
    void onPlay(queue, idx >= 0 ? idx : 0);
  };

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.tabStats}</h2>
          <p className="sub">
            {t.media.statsSub} · {fmtSizeGB(data.totalSize)} · {fmtHours(data.totalDurationMs)}
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
            <span className="stat-num">{data.tracks.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-time">
            <IconUser size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statLiked}</span>
            <span className="stat-num">{data.liked.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-audio">
            <IconClock size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statPlays}</span>
            <span className="stat-num">{data.plays.toLocaleString()}</span>
          </span>
        </div>
        <div className="stat-card">
          <span className="stat-ico i-cover">
            <IconDisc size={18} />
          </span>
          <span className="stat-meta">
            <span className="stat-lbl">{t.media.statPlayedTracks}</span>
            <span className="stat-num">{data.playedTracks.toLocaleString()}</span>
          </span>
        </div>
      </div>

      <section style={{ marginTop: "var(--sp-5)" }}>
        <div className="section-head">
          <h2 style={{ fontSize: 16 }}>{t.media.weekTitle}</h2>
          <span className="hint" style={{ fontSize: "var(--fs-xs)", color: "var(--text-faint)" }}>
            {t.media.weekHint}
          </span>
        </div>
        <div className="chart7-wrap">
          <div className="chart7">
            {days.map((d, i) => (
              <div className="c7col" key={d} title={`${d} · ${t.media.playsN(weekVals[i])}`}>
                <b>{weekVals[i] > 0 ? weekVals[i] : ""}</b>
                <i style={{ height: `${Math.max(4, (weekVals[i] / max) * 100)}%` }} />
                <em>{d.slice(5).replace("-", "/")}</em>
              </div>
            ))}
          </div>
          {data.plays === 0 && <p className="c7-empty">{t.media.weekEmpty}</p>}
        </div>
      </section>

      <section style={{ marginTop: "var(--sp-5)" }}>
        <div className="section-head">
          <h2 style={{ fontSize: 16 }}>{t.media.topTitle}</h2>
          <span className="hint" style={{ fontSize: "var(--fs-xs)", color: "var(--text-faint)" }}>
            {t.media.topHint}
          </span>
        </div>
        <div className="panel" style={{ padding: 0, overflow: "hidden", marginTop: "var(--sp-3)" }}>
          <div className="vt-head">
            <span>{t.media.colIndex}</span>
            <span>{t.media.colTitle}</span>
            <span>{t.media.colAlbum}</span>
            <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
            <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
            <span style={{ textAlign: "right" }}>{t.media.colPlays}</span>
          </div>
          {data.top.length === 0 ? (
            <div className="media-empty">
              <p>{t.media.historyEmpty}</p>
            </div>
          ) : (
            data.top.map((tr, i) => (
              <TrackRow
                key={tr.id}
                lead={i + 1}
                track={tr}
                trailing={<span className="vt-num">{t.media.playsN(tr.playCount)}</span>}
                onPlay={onPlay ? () => playTop(tr) : undefined}
              />
            ))
          )}
        </div>
      </section>
    </>
  );
}
