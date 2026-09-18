// 音乐库（P1 核心页）：十万级曲目浏览 = 分页取数 + 自建虚拟滚动；
// 搜索走 core 的 search_tracks（一次 ≤500，不走虚拟化）。
import { useEffect, useState } from "react";
import { IS_DESKTOP, libraryStats, searchTracks } from "./api";
import type { LibraryStats, Track } from "./api";
import { useLang } from "./i18n";
import { fmtClock, fmtSizeGB } from "./lib/format";
import { useWindowedTracks, TRACK_ROW_H } from "./hooks/useWindowedTracks";

/** 单行（虚拟窗口与搜索结果共用） */
function TrackRow({ idx, tr }: { idx: number; tr: Track }) {
  return (
    <div className="vt-row">
      <span className="vt-idx">{idx}</span>
      <span className="vt-main">
        <b title={tr.title ?? undefined}>{tr.title ?? "—"}</b>
        <span>{tr.artist ?? "—"}</span>
      </span>
      <span className="vt-alb" title={tr.album ?? undefined}>
        {tr.album ?? "—"}
      </span>
      <span className="vt-num">{fmtClock(tr.durationMs)}</span>
      <span className="vt-num">
        {(tr.format ?? "").toUpperCase()}
        {tr.isLossless ? " · SQ" : ""}
      </span>
    </div>
  );
}

export default function LibraryPage() {
  const { t } = useLang();
  const w = useWindowedTracks();
  const [stats, setStats] = useState<LibraryStats | null>(null);
  const [initErr, setInitErr] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Track[] | null>(null);
  const [searching, setSearching] = useState(false);

  const { reset } = w;
  useEffect(() => {
    if (!IS_DESKTOP) return;
    libraryStats()
      .then((s) => {
        setStats(s);
        reset(s.tracks);
      })
      .catch((e: unknown) => setInitErr(String(e)));
  }, [reset]);

  // 搜索：250ms 防抖；空查询回退虚拟列表
  useEffect(() => {
    const q = query.trim();
    if (!q) {
      setResults(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const id = window.setTimeout(() => {
      searchTracks(q, 500)
        .then(setResults)
        .catch(() => setResults([]))
        .finally(() => setSearching(false));
    }, 250);
    return () => window.clearTimeout(id);
  }, [query]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }
  if (initErr) {
    return (
      <div className="media-empty">
        <p>{initErr}</p>
      </div>
    );
  }

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.libTitle}</h2>
          <p className="sub">{t.media.libSub(w.total, fmtSizeGB(stats?.totalSize ?? 0))}</p>
        </div>
        <div className="act">
          <label className="search-inline">
            <svg
              width="15"
              height="15"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.9"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="11" cy="11" r="6.5" />
              <path d="M15.8 15.8L20 20" />
            </svg>
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t.media.searchPlaceholder}
              aria-label={t.media.searchPlaceholder}
            />
          </label>
        </div>
      </div>

      <div className="panel" style={{ padding: 0, overflow: "hidden" }}>
        <div className="vt-head">
          <span>{t.media.colIndex}</span>
          <span>{t.media.colTitle}</span>
          <span>{t.media.colAlbum}</span>
          <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
          <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
        </div>

        {results !== null ? (
          results.length === 0 ? (
            <div className="media-empty">
              <p>{searching ? t.media.loading : t.media.searchEmpty}</p>
            </div>
          ) : (
            <>
              <div className="vt-loading">{t.media.searchResult(results.length)}</div>
              {results.map((r, i) => (
                <TrackRow key={r.id} idx={i + 1} tr={r} />
              ))}
            </>
          )
        ) : (
          <div className="vt-scroll" onScroll={(e) => w.onScroll(e.currentTarget)}>
            <div style={{ height: w.total * TRACK_ROW_H, position: "relative" }}>
              <div style={{ transform: `translateY(${w.start * TRACK_ROW_H}px)` }}>
                {w.indices.map((i) => {
                  const tr = w.rowAt(i);
                  return tr ? (
                    <TrackRow key={tr.id} idx={i + 1} tr={tr} />
                  ) : (
                    <div className="vt-row" key={`ph-${i}`}>
                      <span className="vt-idx">{i + 1}</span>
                      <span className="vt-main">
                        <b>{t.media.loading}</b>
                      </span>
                    </div>
                  );
                })}
              </div>
            </div>
          </div>
        )}
      </div>
    </>
  );
}
