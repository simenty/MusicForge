// 我喜欢的音乐（P3）：liked_tracks 列表 + 播放全部 + 行内取消喜欢。
//
// 取消喜欢后该行**立即从列表消失**（本地过滤，不回后端重拉）——
// 过滤只在初始 liked 集合加载完成后生效（否则会把整页误滤为空）。
import { useCallback, useEffect, useState } from "react";
import { IS_DESKTOP, likedTracks } from "./api";
import type { Track } from "./api";
import { useLang } from "./i18n";
import { useLiked } from "./hooks/useLiked";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";

export default function FavoritesPage({
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
  const [rows, setRows] = useState<Track[] | null>(null);
  const liked = useLiked();
  const selApi = useSelection();

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    likedTracks(500, 0)
      .then(setRows)
      .catch(() => setRows([]));
  }, []);
  useEffect(() => reload(), [reload]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  const live = rows && liked.loaded ? rows.filter((r) => liked.isLiked(r.id)) : rows;

  // P6.19 批量操作（selApi 已在组件顶部无条件初始化）
  const list = live ?? [];
  const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
  const bulkQueue = async () => {
    if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
    selApi.toggleSelMode();
  };
  const bulkPlayNext = async () => {
    if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
    selApi.toggleSelMode();
  };

  const playFrom = (tr: Track) => {
    if (!onPlay || !live) return;
    const idx = live.findIndex((x) => x.id === tr.id);
    void onPlay(live, idx >= 0 ? idx : 0);
  };

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.tabFavorites}</h2>
          <p className="sub">{t.media.favSub(live?.length ?? 0)}</p>
        </div>
        <div className="act">
          <button
            className="btn sm primary"
            onClick={() => {
              if (onPlay && live && live.length > 0) void onPlay(live, 0);
            }}
            disabled={!live || live.length === 0}
          >
            {t.media.playAll}
          </button>
          <button
            className={"btn sm" + (selApi.selMode ? " on" : "")}
            onClick={selApi.toggleSelMode}
            disabled={!live || live.length === 0}
          >
            {t.player.select}
          </button>
        </div>
      </div>

      {live === null ? (
        <div className="media-empty">
          <p>{t.media.loading}</p>
        </div>
      ) : live.length === 0 ? (
        <div className="media-empty">
          <p>{t.media.favEmpty}</p>
        </div>
      ) : (
        <div className="panel" style={{ padding: 0, overflow: "hidden" }}>
          <div className="vt-head">
            <span>{t.media.colIndex}</span>
            <span>{t.media.colTitle}</span>
            <span>{t.media.colAlbum}</span>
            <span style={{ textAlign: "right" }}>{t.media.colTime}</span>
            <span style={{ textAlign: "right" }}>{t.media.colFormat}</span>
            <span />
          </div>
          {live.map((r, i) => (
            <TrackRow
              key={r.id}
              lead={i + 1}
              track={r}
              onPlay={onPlay ? () => playFrom(r) : undefined}
              liked={liked.isLiked(r.id)}
              onLike={() => void liked.toggle(r.id)}
              onQueue={onQueue ? () => void onQueue([r]) : undefined}
              onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
              selectable={selApi.selMode}
              selected={selApi.has(String(r.id))}
              onToggleSelect={() => selApi.toggle(String(r.id))}
              />
          )          )}
        </div>
      )}
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
