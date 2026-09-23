// 歌词面板（P6）：LRC 解析 + 按播放位置高亮 + 自动滚动。
// P6.11：点击带时间戳的行跳转播放位置；时间轴偏移校准（±0.5s，按曲目持久化）。
//
// 数据来自 lyrics_fetch（本地缓存优先；首次打开且未缓存时联网一次——LRCLIB）。
// 纯文本歌词（无时间戳）也能显示，只是不做高亮/滚动与点击跳转。
import { useEffect, useMemo, useRef, useState } from "react";
import { IS_DESKTOP, lyricsFetch } from "./api";
import { useLang } from "./i18n";
import { activeLine, parseLrc } from "./lib/lrc";
import type { PlayerApi } from "./hooks/usePlayer";

/** 偏移步进：±0.5s（LRC 制作者常见漂移量级） */
const STEP_MS = 500;
/** 按曲目的偏移持久化键 */
const offKey = (id: number) => `mf.lrcOff.${id}`;

export default function LyricsPanel({
  player,
  onClose,
}: {
  player: PlayerApi;
  onClose: () => void;
}) {
  const { t } = useLang();
  const trackId = player.status?.trackId ?? null;
  /** undefined = 加载中；null = 无歌词；string = 歌词文本 */
  const [text, setText] = useState<string | null | undefined>(undefined);
  const [err, setErr] = useState<string | null>(null);
  /** 时间轴偏移（ms）：正值 = 歌词整体后移（行更晚出现） */
  const [offset, setOffset] = useState(0);
  const boxRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (trackId === null || !IS_DESKTOP) {
      setText(null);
      return;
    }
    let alive = true;
    setText(undefined);
    setErr(null);
    void lyricsFetch(trackId)
      .then((r) => {
        if (alive) setText(r);
      })
      .catch((e: unknown) => {
        if (alive) {
          setErr(String(e));
          setText(null);
        }
      });
    return () => {
      alive = false;
    };
  }, [trackId]);

  // 按曲目载入偏移（无记录 → 0）
  useEffect(() => {
    if (trackId === null) {
      setOffset(0);
      return;
    }
    // localStorage 在极少数 WebView2 配置下被禁用（项目其它持久化处均已包
    // try/catch，见 settings.ts / lib/session.ts）。本面板位于**所有错误边界
    // 之外**，异常一旦冒出就是整页白屏——读不到按 0 处理即可。
    let n = 0;
    try {
      n = Number(localStorage.getItem(offKey(trackId)));
    } catch {
      n = 0;
    }
    setOffset(Number.isFinite(n) ? n : 0);
  }, [trackId]);

  const lines = useMemo(() => (typeof text === "string" ? parseLrc(text) : []), [text]);
  const pos = player.status?.positionMs ?? 0;
  /** 带时间戳（可点击跳转）的歌词 */
  const timed = lines.length > 0 && lines[0].t >= 0;

  /** 当前行：行时间 + offset ≤ pos ⟺ 行时间 ≤ pos − offset */
  const active = useMemo(() => activeLine(lines, pos - offset), [lines, pos, offset]);

  const applyOffset = (delta: number) => {
    const next = offset + delta;
    setOffset(next);
    if (trackId !== null) {
      try {
        localStorage.setItem(offKey(trackId), String(next));
      } catch {
        /* 存储不可用：偏移量是装饰能力，不影响播放，静默降级 */
      }
    }
  };

  const resetOffset = () => {
    setOffset(0);
    if (trackId !== null) {
      try {
        localStorage.removeItem(offKey(trackId));
      } catch {
        /* 同上：存储不可用不影响播放 */
      }
    }
  };

  // 自动滚动：当前行进视野中部
  useEffect(() => {
    if (active < 0 || !boxRef.current) return;
    const el = boxRef.current.querySelector<HTMLElement>(`[data-line="${active}"]`);
    el?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [active]);

  // Esc 关闭（惯例同 ConfirmDialog）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-mask" role="presentation" onClick={onClose}>
      <div
        className="modal lyrics-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="lyr-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="lyr-title">{t.player.lyricsTitle(player.status?.title ?? "—")}</h3>
        </div>
        <div className="modal-body">
          <div className="lyrics-box" ref={boxRef}>
            {text === undefined ? (
              <p className="scan-note">{t.media.loading}</p>
            ) : err ? (
              <p className="scan-error">{err}</p>
            ) : lines.length === 0 ? (
              <p className="scan-note">{t.player.lyricsNone}</p>
            ) : (
              lines.map((l, i) =>
                timed ? (
                  <button
                    key={i}
                    type="button"
                    data-line={i}
                    className={"lyric-line" + (i === active ? " on" : "")}
                    onClick={() => void player.seek(Math.max(0, l.t + offset))}
                    title={t.player.lrcSeek}
                  >
                    {l.s || "…"}
                  </button>
                ) : (
                  <p
                    key={i}
                    data-line={i}
                    className={"lyric-line" + (i === active ? " on" : "")}
                  >
                    {l.s || "…"}
                  </p>
                )
              )
            )}
          </div>
        </div>
        <div className="modal-foot lrc-foot">
          {timed && (
            <div className="lrc-off">
              <button type="button" className="btn sm" onClick={() => applyOffset(-STEP_MS)}>
                −0.5s
              </button>
              <button
                type="button"
                className="btn sm"
                onClick={resetOffset}
                title={t.player.lrcOffsetReset}
              >
                {t.player.lrcOffset((offset / 1000).toFixed(1) + "s")}
              </button>
              <button type="button" className="btn sm" onClick={() => applyOffset(STEP_MS)}>
                +0.5s
              </button>
            </div>
          )}
          <button type="button" className="btn sm" onClick={onClose}>
            {t.player.close}
          </button>
        </div>
      </div>
    </div>
  );
}
