// 播放底栏（P2）：常驻内容区底部——曲目信息 / 上一首·播放暂停·下一首 / 进度 + 音量。
// 进度拖动：拖动中本地显示（不刷后端），松手才 seek（后端重建解码有成本）。
import { useEffect, useRef, useState } from "react";
import { IS_DESKTOP, trackCover } from "./api";
import { useLang } from "./i18n";
import { fmtClock } from "./lib/format";
import { assetUrl } from "./lib/asset";
import type { PlayerApi } from "./hooks/usePlayer";
import LyricsPanel from "./LyricsPanel";

export default function PlayerBar({ player }: { player: PlayerApi }) {
  const { t } = useLang();
  const { status, playing, restored } = player;
  /** 会话恢复的当前曲目（引擎空闲时的展示数据；P6.12） */
  const resumed = restored?.items[restored.index] ?? null;
  const [drag, setDrag] = useState<number | null>(null);
  const [qOpen, setQOpen] = useState(false);
  const [lyrOpen, setLyrOpen] = useState(false);
  /** 当前曲目封面（本地缓存路径 → asset URL；无 → null 显示图标占位） */
  const [cover, setCover] = useState<string | null>(null);
  /** P6.11 睡眠定时：菜单开合 / 到期时刻（epoch ms，null = 未启用） / 剩余毫秒 */
  const [sleepOpen, setSleepOpen] = useState(false);
  const [sleepUntil, setSleepUntil] = useState<number | null>(null);
  const [sleepLeftMs, setSleepLeftMs] = useState(0);

  // 曲目切换时拉一次封面（纯本地 db 查询，不发网络）；会话恢复时同样展示这张
  const trackId = status?.trackId ?? resumed?.trackId ?? null;
  useEffect(() => {
    if (trackId === null || !IS_DESKTOP) {
      setCover(null);
      return;
    }
    let alive = true;
    void trackCover(trackId)
      .then((p) => {
        if (alive) setCover(assetUrl(p));
      })
      .catch(() => {
        if (alive) setCover(null);
      });
    return () => {
      alive = false;
    };
  }, [trackId]);

  // P6.11 睡眠定时：每秒刷新剩余时间，到期暂停。
  // playerRef 镜像：avoid 把 500ms 轮询产生的新对象拖进依赖（否则 interval 每次轮询重建）。
  const playerRef = useRef(player);
  playerRef.current = player;
  useEffect(() => {
    if (sleepUntil === null) {
      setSleepLeftMs(0);
      return;
    }
    const tick = () => {
      const left = sleepUntil - Date.now();
      setSleepLeftMs(Math.max(0, left));
      if (left <= 0) {
        setSleepUntil(null);
        void playerRef.current.pause(); // pause 自带 playing 守卫：到期只暂停、绝不反向唤醒
      }
    };
    tick();
    // 10s 粒度：显示到分钟级，无需秒级唤醒（省电 + 测试快进成本低）
    const timer = window.setInterval(tick, 10_000);
    return () => window.clearInterval(timer);
  }, [sleepUntil]);

  /** 设定睡眠定时（分钟；≤0 = 关闭）。 */
  const setSleep = (minutes: number) => {
    setSleepOpen(false);
    setSleepUntil(minutes <= 0 ? null : Date.now() + minutes * 60_000);
  };

  const dur = status?.durationMs ?? 0;
  const pos = drag ?? status?.positionMs ?? 0;
  const hasTrack = !!status && status.trackId !== null;

  /** 封面缺省占位（无封面 / 会话恢复共用） */
  const coverFallback = (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 18V6l10-2v12" />
      <circle cx="6.5" cy="18" r="2.5" />
      <circle cx="16.5" cy="16" r="2.5" />
    </svg>
  );

  return (
    <>
    <footer className="player-bar" aria-label={t.player.play}>
      <div className="pb-info">
        {hasTrack ? (
          <>
            <span className="pb-cover" aria-hidden="true">
              {cover ? <img src={cover} alt="" /> : coverFallback}
            </span>
            <span className="pb-title">
              <b title={status.title ?? undefined}>{status.title ?? "—"}</b>
              <span>
                {status.artist ?? "—"}
                {status.queueLen > 0 ? ` · ${t.player.queueN(status.queueLen)}` : ""}
              </span>
            </span>
          </>
        ) : resumed ? (
          <>
            <span className="pb-cover" aria-hidden="true">
              {cover ? <img src={cover} alt="" /> : coverFallback}
            </span>
            <span className="pb-title">
              <b title={resumed.title ?? undefined}>{resumed.title ?? "—"}</b>
              <span>
                {resumed.artist ?? "—"} · {t.player.sessionResume}
              </span>
            </span>
          </>
        ) : (
          <span className="pb-idle">
            {IS_DESKTOP ? t.player.noTrack : t.player.desktopOnly}
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
          onClick={() => void (hasTrack ? player.toggle() : player.resume())}
          disabled={!hasTrack && !restored}
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
        {/* P6：歌词（LRCLIB；打开且未缓存时联网一次） */}
        <button
          className="pb-btn"
          onClick={() => setLyrOpen(true)}
          disabled={!hasTrack}
          aria-label={t.player.lyrics}
          title={t.player.lyrics}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
          >
            <path d="M5 7h9M5 12h6" />
            <path d="M17 11v6.5" />
            <circle cx="15" cy="17.5" r="2" />
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
        {/* P6.11 睡眠定时（图标激活态显示剩余分钟于 title） */}
        <button
          className={"pb-btn" + (sleepUntil !== null ? " on" : "")}
          onClick={() => setSleepOpen((v) => !v)}
          aria-label={t.player.sleep}
          title={
            sleepUntil !== null
              ? t.player.sleepLeft(Math.max(1, Math.ceil(sleepLeftMs / 60_000)))
              : t.player.sleep
          }
          aria-expanded={sleepOpen}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M20 14.5A8.5 8.5 0 019.5 4a7.5 7.5 0 1010.5 10.5z" />
          </svg>
        </button>
        <button
          className="pb-btn"
          onClick={() => setQOpen((v) => !v)}
          disabled={player.queue.length === 0}
          aria-label={t.player.queue}
          title={t.player.queue}
          aria-expanded={qOpen}
        >
          <svg
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
          >
            <path d="M4 7h16M4 12h16M4 17h10" />
          </svg>
        </button>
      </div>

      {qOpen && player.queue.length > 0 && (
        <div className="pb-queue" role="listbox" aria-label={t.player.queue}>
          {player.queue.map((it, i) => (
            <button
              key={`${it.trackId}-${i}`}
              className={"qd-item" + (i === status?.queueIndex ? " on" : "")}
              onClick={() => {
                void player.jump(i);
                setQOpen(false);
              }}
              role="option"
              aria-selected={i === status?.queueIndex}
            >
              <span className="qd-idx">{i + 1}</span>
              <span className="qd-t">
                <b title={it.title ?? undefined}>{it.title ?? "—"}</b>
                <span>{it.artist ?? "—"}</span>
              </span>
            </button>
          ))}
        </div>
      )}

      {sleepOpen && (
        <div className="pb-sleep" role="menu" aria-label={t.player.sleep}>
          <button type="button" role="menuitem" onClick={() => setSleep(0)}>
            {t.player.sleepOff}
          </button>
          {[15, 30, 60].map((m) => (
            <button key={m} type="button" role="menuitem" onClick={() => setSleep(m)}>
              {t.player.sleepMin(m)}
            </button>
          ))}
        </div>
      )}

      {status?.error && (
        <div className="pb-err" role="alert">
          {t.player.errorPrefix(status.error)}
        </div>
      )}
      </footer>

      {/* P6：歌词面板（LRCLIB；打开且未缓存时联网一次） */}
      {lyrOpen && <LyricsPanel player={player} onClose={() => setLyrOpen(false)} />}
      </>
    );
}
