// 歌词面板（P6）：LRC 解析 + 按播放位置高亮 + 自动滚动。
//
// 数据来自 lyrics_fetch（本地缓存优先；首次打开且未缓存时联网一次——LRCLIB）。
// 纯文本歌词（无时间戳）也能显示，只是不做高亮/滚动。
import { useEffect, useMemo, useRef, useState } from "react";
import { IS_DESKTOP, lyricsFetch } from "./api";
import { useLang } from "./i18n";
import { activeLine, parseLrc } from "./lib/lrc";
import type { PlayerApi } from "./hooks/usePlayer";

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

  const lines = useMemo(() => (typeof text === "string" ? parseLrc(text) : []), [text]);
  const pos = player.status?.positionMs ?? 0;

  /** 当前行：最后一个 t ≤ pos 的行（纯文本 → -1 不高亮） */
  const active = useMemo(() => activeLine(lines, pos), [lines, pos]);

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
              lines.map((l, i) => (
                <p
                  key={i}
                  data-line={i}
                  className={"lyric-line" + (i === active ? " on" : "")}
                >
                  {l.s || "…"}
                </p>
              ))
            )}
          </div>
        </div>
        <div className="modal-foot">
          <button className="btn sm" onClick={onClose}>
            {t.player.close}
          </button>
        </div>
      </div>
    </div>
  );
}
