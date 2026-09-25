// 统计（P3）：曲库规模 + 行为计数 + 近 7 天条形图（纯 CSS）+ 最常播放 Top 10。
import { useEffect, useState, useCallback } from "react";
import { IS_DESKTOP, statsOverview } from "./api";
import { useRequestGuard } from "./hooks/useRequestGuard";
import type { StatsOverview, Track } from "./api";
import { useLang } from "./i18n";
import { fmtHours, fmtSizeGB } from "./lib/format";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
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
  onQueue,
  onPlayNext,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const [data, setData] = useState<StatsOverview | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // P6.19 批量操作（hook 须无条件调用，置于早返回之前）
  const selApi = useSelection();
  const list = data?.top ?? [];
  const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
  };

  // P2-17：统计拉取守卫（晚到响应不得覆写新数据）
  const statsGuard = useRequestGuard();
  useEffect(() => {
    if (!IS_DESKTOP) return;
    const token = statsGuard.token();
    statsOverview()
      .then((d) => {
        if (!statsGuard.isStale(token)) setData(d);
      })
      .catch((e: unknown) => {
        if (!statsGuard.isStale(token)) setErr(String(e));
      });
  }, [statsGuard]);

  // P2-16：行内动作稳定化，配合 TrackRow memo（须置于早返回之前，遵守 hooks 顺序）
  const handlePlay = useCallback(
    (tr: Track) => {
      if (!onPlay || !data) return;
      const queue: Track[] = data.top;
      const idx = queue.findIndex((x) => x.id === tr.id);
      void onPlay(queue, idx >= 0 ? idx : 0);
    },
    [onPlay, data]
  );
  const handleQueue = useCallback((tr: Track) => void onQueue?.([tr]), [onQueue]);
  const handlePlayNext = useCallback((tr: Track) => void onPlayNext?.([tr]), [onPlayNext]);
  const handleToggleSelect = useCallback(
    (tr: Track) => selApi.toggle(String(tr.id)),
    [selApi.toggle]
  );

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

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.tabStats}</h2>
          <p className="sub">
            {t.media.statsSub} · {fmtSizeGB(data.totalSize)} · {fmtHours(data.totalDurationMs)}
          </p>
        </div>
        <div className="act">
          <button
            className={"btn sm" + (selApi.selMode ? " on" : "")}
            onClick={selApi.toggleSelMode}
            disabled={!data || data.top.length === 0}
          >
            {t.player.select}
          </button>
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
                onPlay={onPlay ? handlePlay : undefined}
                onQueue={onQueue ? handleQueue : undefined}
                onPlayNext={onPlayNext ? handlePlayNext : undefined}
                selectable={selApi.selMode}
                selected={selApi.has(String(tr.id))}
                onToggleSelect={handleToggleSelect}
              />
            ))
          )}
        </div>
      </section>
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
