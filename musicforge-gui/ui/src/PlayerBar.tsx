// 播放底栏（P2）：常驻内容区底部——曲目信息 / 上一首·播放暂停·下一首 / 进度 + 音量。
// 进度拖动：拖动中本地显示（不刷后端），松手才 seek（后端重建解码有成本）。
import { useState } from "react";
import { useLang } from "./i18n";
import { fmtClock } from "./lib/format";
import type { PlayerApi } from "./hooks/usePlayer";

export default function PlayerBar({ player }: { player: PlayerApi }) {
  const { t } = useLang();
  const { status, playing } = player;
  const [drag, setDrag] = useState<number | null>(null);

  const dur = status?.durationMs ?? 0;
  const pos = drag ?? status?.positionMs ?? 0;
  const hasTrack = !!status && status.trackId !== null;
  const isError = status?.state === "error";

  return (
    <footer className="player-bar" aria-label={t.player.play}>
      <div className="pb-info">
        {hasTrack ? (
          <>
            <span className="pb-cover" aria-hidden="true">
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                <path d="M9 18V6l10-2v12" />
                <circle cx="6.5" cy="18" r="2.5" />
                <circle cx="16.5" cy="16" r="2.5" />
              </svg>
            </span>
            <span className="pb-title">
              <b title={status.title ?? undefined}>{status.title ?? "—"}</b>
              <span>
                {status.artist ?? "—"}
                {status.queueLen > 0 ? ` · ${t.player.queueN(status.queueLen)}` : ""}
              </span>
            </span>
          </>
        ) : (
          <span className="pb-idle">
            {isError ? t.player.desktopOnly : t.player.noTrack}
          </span>
        )}
      </div>

      <div className="pb-ctrl">
        <button
          className="pb-btn"
          onClick={() => void player.prev()}
          disabled={!hasTrack}
          aria-label={t.player.prev}
          title={t.player.prev}
        >
          <svg width="17" height="17" viewBox="0 0 24 24" fill="currentColor">
            <path d="M7 6h2v12H7zM20 6v12l-9-6z" />
          </svg>
        </button>
        <button
          className="pb-btn main"
          onClick={() => void player.toggle()}
          disabled={!hasTrack || isError}
          aria-label={playing ? t.player.pause : t.player.play}
          title={playing ? t.player.pause : t.player.play}
        >
          {playing ? (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M8 5h3.2v14H8zM12.8 5H16v14h-3.2z" />
            </svg>
          ) : (
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M8 5.5v13l11-6.5z" />
            </svg>
          )}
        </button>
        <button
          className="pb-btn"
          onClick={() => void player.next()}
          disabled={!hasTrack}
          aria-label={t.player.next}
          title={t.player.next}
        >
          <svg width="17" height="17" viewBox="0 0 24 24" fill="currentColor">
            <path d="M15 6h2v12h-2zM4 6v12l9-6z" />
          </svg>
        </button>
      </div>

      <div className="pb-right">
        <div className="pb-progress">
          <span className="pb-time">{fmtClock(pos)}</span>
          <input
            type="range"
            min={0}
            max={Math.max(dur, 1)}
            value={Math.min(pos, Math.max(dur, 1))}
            disabled={!hasTrack || dur <= 0}
            onChange={(e) => setDrag(Number(e.target.value))}
            onPointerUp={() => {
              if (drag !== null) {
                void player.seek(drag);
                setDrag(null);
              }
            }}
            aria-label={t.player.progress}
            title={t.player.progress}
          />
          <span className="pb-time">{dur > 0 ? fmtClock(dur) : "—"}</span>
        </div>
        <div className="pb-vol">
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.7"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M11 5L6.5 9H3v6h3.5L11 19z" />
            <path d="M15.5 9.5a4 4 0 010 5M18 7a7.5 7.5 0 010 10" />
          </svg>
          <input
            type="range"
            min={0}
            max={100}
            value={Math.round((status?.volume ?? 1) * 100)}
            onChange={(e) => player.setVolume(Number(e.target.value) / 100)}
            aria-label={t.player.volume}
            title={t.player.volume}
          />
        </div>
      </div>

      {isError && status?.error && (
        <div className="pb-err" role="alert">
          {t.player.errorPrefix(status.error)}
        </div>
      )}
    </footer>
  );
}
