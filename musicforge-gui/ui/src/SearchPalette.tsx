// 全局搜索面板（P6.14）：Ctrl/Cmd+K 唤起——跨曲目 / 专辑 / 艺术家 / 歌单一次检索。
//
// 键盘：↑↓ 选择、Enter 打开（曲目即播放）、Esc 关闭。
// 查询去抖 200ms：本地库查询很快，但十万级曲库下逐键查询没必要。
// 一次 IPC 拿四组结果（search_all）——面板不因分组而放大请求数。
import { useEffect, useMemo, useRef, useState } from "react";
import { IS_DESKTOP, searchAll } from "./api";
import type { Album, Artist, Playlist, SearchResults, Track } from "./api";
import { useLang } from "./i18n";
import { fmtClock } from "./lib/format";
import { IconSearch } from "./icons";

/** 可跳转的搜索命中 */
export type SearchTarget =
  | { kind: "track"; track: Track }
  | { kind: "album"; id: number }
  | { kind: "artist"; id: number }
  | { kind: "playlist"; id: number };

export default function SearchPalette({
  onClose,
  onPlay,
  onOpen,
}: {
  onClose: () => void;
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  onOpen?: (target: SearchTarget) => void;
}) {
  const { t } = useLang();
  const [q, setQ] = useState("");
  const [res, setRes] = useState<SearchResults | null>(null);
  const [busy, setBusy] = useState(false);
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 查询（去抖）
  useEffect(() => {
    if (!IS_DESKTOP) return;
    const term = q.trim();
    if (!term) {
      setRes(null);
      setBusy(false);
      return;
    }
    setBusy(true);
    const timer = window.setTimeout(() => {
      void searchAll(term, 8)
        .then((r) => {
          setRes(r);
          setCursor(0);
        })
        .catch(() => setRes(null))
        .finally(() => setBusy(false));
    }, 200);
    return () => window.clearTimeout(timer);
  }, [q]);

  // 扁平化：键盘导航与 Enter 统一按一个序列处理（顺序即分组顺序）
  const flat = useMemo<SearchTarget[]>(() => {
    if (!res) return [];
    return [
      ...res.tracks.map((track) => ({ kind: "track" as const, track })),
      ...res.albums.map((a) => ({ kind: "album" as const, id: a.id })),
      ...res.artists.map((a) => ({ kind: "artist" as const, id: a.id })),
      ...res.playlists.map((p) => ({ kind: "playlist" as const, id: p.id })),
    ];
  }, [res]);

  const commit = (target: SearchTarget) => {
    if (target.kind === "track") {
      if (onPlay) void onPlay([target.track], 0);
    } else {
      onOpen?.(target);
    }
    onClose();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setCursor((c) => Math.min(c + 1, Math.max(0, flat.length - 1)));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setCursor((c) => Math.max(0, c - 1));
    } else if (e.key === "Enter") {
      const sel = flat[cursor];
      if (sel) commit(sel);
    }
  };

  // 分组起点（用于把扁平光标映射回各组）
  const nTracks = res?.tracks.length ?? 0;
  const nAlbums = res?.albums.length ?? 0;
  const nArtists = res?.artists.length ?? 0;
  const offAlbum = nTracks;
  const offArtist = offAlbum + nAlbums;
  const offPlaylist = offArtist + nArtists;

  const row = (
    target: SearchTarget,
    i: number,
    primary: string,
    secondary: string,
    meta?: string
  ) => (
    <button
      key={`${target.kind}-${target.kind === "track" ? target.track.id : target.id}-${i}`}
      className={"sp-row" + (i === cursor ? " on" : "")}
      onMouseEnter={() => setCursor(i)}
      onClick={() => commit(target)}
      role="option"
      aria-selected={i === cursor}
    >
      <span className="sp-main">
        <b title={primary}>{primary}</b>
        <span>{secondary}</span>
      </span>
      {meta && <span className="sp-meta">{meta}</span>}
    </button>
  );

  return (
    <div className="modal-mask" role="presentation" onClick={onClose}>
      <div
        className="modal search-modal"
        role="dialog"
        aria-modal="true"
        aria-label={t.search.title}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className="sp-input">
          <IconSearch size={16} />
          <input
            ref={inputRef}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder={t.search.placeholder}
            aria-label={t.search.title}
            spellCheck={false}
          />
        </div>

        <div className="sp-body" role="listbox" aria-label={t.search.title}>
          {!q.trim() ? (
            <p className="sp-hint">{t.search.empty}</p>
          ) : busy && !res ? (
            <p className="sp-hint">{t.search.searching}</p>
          ) : flat.length === 0 ? (
            <p className="sp-hint">{t.search.none}</p>
          ) : (
            <>
              {nTracks > 0 && (
                <div className="sp-group">
                  <span className="sp-ghead">{t.search.groupTracks}</span>
                  {res!.tracks.map((tr, i) =>
                    row(
                      { kind: "track", track: tr },
                      i,
                      tr.title ?? "—",
                      `${tr.artist ?? "—"} · ${tr.album ?? "—"}`,
                      fmtClock(tr.durationMs)
                    )
                  )}
                </div>
              )}
              {nAlbums > 0 && (
                <div className="sp-group">
                  <span className="sp-ghead">{t.search.groupAlbums}</span>
                  {res!.albums.map((a: Album, i) =>
                    row(
                      { kind: "album", id: a.id },
                      offAlbum + i,
                      a.title ?? "—",
                      a.artist ?? "—",
                      t.media.tracksN(a.trackCount)
                    )
                  )}
                </div>
              )}
              {nArtists > 0 && (
                <div className="sp-group">
                  <span className="sp-ghead">{t.search.groupArtists}</span>
                  {res!.artists.map((a: Artist, i) =>
                    row(
                      { kind: "artist", id: a.id },
                      offArtist + i,
                      a.name,
                      t.search.openArtist,
                      t.media.tracksN(a.trackCount)
                    )
                  )}
                </div>
              )}
              {res!.playlists.length > 0 && (
                <div className="sp-group">
                  <span className="sp-ghead">{t.search.groupPlaylists}</span>
                  {res!.playlists.map((p: Playlist, i) =>
                    row(
                      { kind: "playlist", id: p.id },
                      offPlaylist + i,
                      p.name,
                      t.search.openPlaylist,
                      t.media.tracksN(p.trackCount)
                    )
                  )}
                </div>
              )}
            </>
          )}
        </div>

        <div className="sp-foot">{t.search.hint}</div>
      </div>
    </div>
  );
}
