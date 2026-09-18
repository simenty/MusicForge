// 播放历史（P2）：倒序列表按「今天 / 昨天 / 更早」分组 + 清空（二次确认）。
import { useCallback, useEffect, useState } from "react";
import { IS_DESKTOP, historyClear, playHistory } from "./api";
import type { HistoryEntry } from "./api";
import { useLang } from "./i18n";
import { fmtClock } from "./lib/format";

type DayKey = "today" | "yesterday" | "earlier";

/** 播放时刻 → 分组键（本地时区） */
function dayKey(sec: number): DayKey {
  const now = new Date();
  const startToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const t = sec * 1000;
  if (t >= startToday) return "today";
  if (t >= startToday - 86_400_000) return "yesterday";
  return "earlier";
}

function timeOf(sec: number): string {
  const d = new Date(sec * 1000);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export default function HistoryPage() {
  const { t } = useLang();
  const [rows, setRows] = useState<HistoryEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    playHistory(300)
      .then(setRows)
      .catch(() => setRows([]));
  }, []);
  useEffect(() => reload(), [reload]);

  const clear = useCallback(async () => {
    if (busy || !window.confirm(t.media.historyClearConfirm)) return;
    setBusy(true);
    try {
      const r = await historyClear();
      setMsg(t.media.historyCleared(r.cleared));
      reload();
    } catch {
      /* 静默：列表未变即为失败信号 */
    } finally {
      setBusy(false);
    }
  }, [busy, t, reload]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  // 已倒序 → 顺序扫描切分组（不重排、不改序）
  const grouped: { key: DayKey; items: HistoryEntry[] }[] = [];
  if (rows) {
    for (const r of rows) {
      const k = dayKey(r.playedAt);
      const last = grouped[grouped.length - 1];
      if (last && last.key === k) last.items.push(r);
      else grouped.push({ key: k, items: [r] });
    }
  }
  const labelOf = (k: DayKey) =>
    k === "today" ? t.media.today : k === "yesterday" ? t.media.yesterday : t.media.earlier;

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.historyTitle}</h2>
          <p className="sub">{t.media.historySub}</p>
        </div>
        <div className="act">
          <button
            className="btn sm"
            onClick={() => void clear()}
            disabled={busy || !rows || rows.length === 0}
          >
            {t.media.historyClear}
          </button>
        </div>
      </div>

      {msg && <div className="scan-note">{msg}</div>}

      {rows === null ? (
        <div className="media-empty">
          <p>{t.media.loading}</p>
        </div>
      ) : rows.length === 0 ? (
        <div className="media-empty">
          <p>{t.media.historyEmpty}</p>
        </div>
      ) : (
        grouped.map((g) => (
          <section key={g.key} className="tl-group">
            <h3>{labelOf(g.key)}</h3>
            <div className="tracks">
              {g.items.map((r, i) => (
                <div className="vt-row" key={`${r.id}-${r.playedAt}-${i}`}>
                  <span className="vt-idx">{timeOf(r.playedAt)}</span>
                  <span className="vt-main">
                    <b title={r.title ?? undefined}>{r.title ?? "—"}</b>
                    <span>{r.artist ?? "—"}</span>
                  </span>
                  <span className="vt-alb" title={r.album ?? undefined}>
                    {r.album ?? "—"}
                  </span>
                  <span className="vt-num">{fmtClock(r.durationMs)}</span>
                  <span className="vt-num">{(r.format ?? "").toUpperCase()}</span>
                  <span />
                </div>
              ))}
            </div>
          </section>
        ))
      )}
    </>
  );
}
