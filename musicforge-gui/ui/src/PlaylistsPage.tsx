// 歌单（P6.4）：列表态（新建/打开）与详情态（播放全部/移除曲目/重命名/删除）。
// 曲目来源：音乐库行尾的 ＋（AddToPlaylistDialog）。
import { useCallback, useEffect, useRef, useState } from "react";
import {
  IS_DESKTOP,
  playlistCreate,
  playlistDelete,
  playlistExport,
  playlistMove,
  playlistRemove,
  playlistRename,
  playlistTracks,
  playlistsCovers,
  playlistsList,
} from "./api";
import type { Playlist, Track } from "./api";
import ConfirmDialog from "./ConfirmDialog";
import TrackRow from "./TrackRow";
import { useLang } from "./i18n";
import { assetUrl } from "./lib/asset";
import { IconList, IconPlus } from "./icons";

export default function PlaylistsPage({
  onPlay,
  focusId = null,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.14 搜索跳转：命中的歌单 id（消费一次即进入详情） */
  focusId?: number | null;
}) {
  const { t } = useLang();
  const [lists, setLists] = useState<Playlist[] | null>(null);
  /** 封面拼贴（歌单 id → 至多 4 张专辑封面路径；P6.12） */
  const [covers, setCovers] = useState<Record<string, string[]>>({});
  const [open, setOpen] = useState<Playlist | null>(null);
  const [items, setItems] = useState<Track[] | null>(null);
  const [newName, setNewName] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [delTarget, setDelTarget] = useState<Playlist | null>(null);
  const [renaming, setRenaming] = useState(false);
  const [renameVal, setRenameVal] = useState("");
  /** 拖拽排序：当前拖起的行下标（P6.6） */
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  /** 操作反馈（导出成功等） */
  const [note, setNote] = useState<string | null>(null);

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    playlistsList()
      .then(setLists)
      .catch(() => setLists([]));
    playlistsCovers()
      .then(setCovers)
      .catch(() => setCovers({}));
  }, []);
  useEffect(() => reload(), [reload]);

  const openList = useCallback((p: Playlist) => {
    setOpen(p);
    setItems(null);
    setErr(null);
    playlistTracks(p.id)
      .then(setItems)
      .catch(() => setItems([]));
  }, []);

  // P6.14 搜索跳转：列表就绪后进入命中歌单（ref 标记已消费，避免反复重拉）
  const consumedFocus = useRef<number | null>(null);
  useEffect(() => {
    if (focusId === null || !lists || consumedFocus.current === focusId) return;
    const hit = lists.find((p) => p.id === focusId);
    if (!hit) return;
    consumedFocus.current = focusId;
    setOpen(hit);
    setItems(null);
    void playlistTracks(hit.id)
      .then(setItems)
      .catch(() => setItems([]));
  }, [focusId, lists]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }
  if (!lists) {
    return (
      <div className="media-empty">
        <p>{t.media.loading}</p>
      </div>
    );
  }

  const create = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      const id = await playlistCreate(name);
      setNewName("");
      reload();
      openList({ id, name, trackCount: 0 });
    } catch (e) {
      setErr(String(e));
    }
  };

  const removeItem = async (trackId: number) => {
    if (!open) return;
    try {
      await playlistRemove(open.id, trackId);
      setItems((prev) => prev?.filter((x) => x.id !== trackId) ?? prev);
      setOpen((p) => (p ? { ...p, trackCount: Math.max(0, p.trackCount - 1) } : p));
      reload();
    } catch (e) {
      setErr(String(e));
    }
  };

  /** 拖拽落位：本地乐观重排 + 后端持久化（失败仅提示，刷新即回到真相） */
  const moveTo = async (toIdx: number) => {
    if (!open || dragIdx === null || !items || dragIdx === toIdx) {
      setDragIdx(null);
      return;
    }
    const from = dragIdx;
    setDragIdx(null);
    const next = items.slice();
    const [moved] = next.splice(from, 1);
    next.splice(toIdx, 0, moved);
    setItems(next);
    try {
      await playlistMove(open.id, moved.id, toIdx);
    } catch (e) {
      setErr(String(e));
    }
  };

  /** 导出为 M3U8（原生保存对话框） */
  const doExport = async () => {
    if (!open) return;
    setNote(null);
    try {
      const r = await playlistExport(open.id);
      if (r) setNote(t.pl.psExported(r.tracks));
    } catch (e) {
      setErr(String(e));
    }
  };

  const doRename = async () => {
    if (!open) return;
    const name = renameVal.trim();
    if (!name) return;
    try {
      await playlistRename(open.id, name);
      setOpen({ ...open, name });
      setRenaming(false);
      reload();
    } catch (e) {
      setErr(String(e));
    }
  };

  const doDelete = async () => {
    if (!delTarget) return;
    try {
      await playlistDelete(delTarget.id);
      setDelTarget(null);
      if (open && open.id === delTarget.id) {
        setOpen(null);
        setItems(null);
      }
      reload();
    } catch (e) {
      setErr(String(e));
      setDelTarget(null);
    }
  };

  // ---------------------------------------------------------------- 详情态 --
  if (open) {
    return (
      <>
        <div className="media-head">
          <div>
            <button
              className="btn sm"
              onClick={() => {
                setOpen(null);
                setItems(null);
                setRenaming(false);
              }}
            >
              ← {t.pl.psBack}
            </button>
          </div>
          <div>
            <h2>{open.name}</h2>
            <p className="sub">
              {t.pl.psCount(items?.length ?? open.trackCount)} · {t.pl.psDragHint}
            </p>
          </div>
          <div className="act">
            <button
              className="btn sm primary"
              disabled={!items || items.length === 0}
              onClick={() => {
                if (onPlay && items && items.length > 0) void onPlay(items, 0);
              }}
            >
              {t.media.playAll}
            </button>
            <button
              className="btn sm"
              onClick={() => {
                setRenaming(true);
                setRenameVal(open.name);
              }}
            >
              {t.pl.psRename}
            </button>
            <button className="btn sm" onClick={() => void doExport()}>
              {t.pl.psExport}
            </button>
            <button className="btn sm" onClick={() => setDelTarget(open)}>
              {t.pl.psDelete}
            </button>
          </div>
        </div>

        {renaming && (
          <div className="toolbar">
            <div className="tb-left">
              <input
                className="cfg-input"
                value={renameVal}
                onChange={(e) => setRenameVal(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void doRename();
                }}
              />
              <button className="btn sm primary" onClick={() => void doRename()}>
                {t.pl.psOk}
              </button>
              <button className="btn sm" onClick={() => setRenaming(false)}>
                {t.pl.psCancel}
              </button>
            </div>
          </div>
        )}
        {err && <p className="scan-error">{err}</p>}
        {note && <p className="scan-note">{note}</p>}

        {items === null ? (
          <p className="scan-note">{t.media.loading}</p>
        ) : items.length === 0 ? (
          <div className="media-empty">
            <p>{t.pl.psEmpty}</p>
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
            {items.map((r, i) => (
              <TrackRow
                key={r.id}
                lead={i + 1}
                track={r}
                dragProps={{
                  draggable: true,
                  onDragStart: () => setDragIdx(i),
                  onDragOver: (e) => e.preventDefault(),
                  onDrop: () => void moveTo(i),
                  style: { cursor: "grab" },
                }}
                onPlay={
                  onPlay
                    ? () => {
                        const idx = items.findIndex((x) => x.id === r.id);
                        void onPlay(items, idx >= 0 ? idx : 0);
                      }
                    : undefined
                }
                trailing={
                  <button
                    className="row-mini"
                    onClick={() => void removeItem(r.id)}
                    title={t.pl.psRemove}
                    aria-label={t.pl.psRemove}
                  >
                    ✕
                  </button>
                }
              />
            ))}
          </div>
        )}

        <ConfirmDialog
          open={delTarget !== null}
          title={t.pl.psDeleteTitle}
          summary={t.pl.psDeleteSummary(delTarget?.name ?? "")}
          ackLabel={t.pl.psDeleteAck}
          confirmLabel={t.pl.psDelete}
          cancelLabel={t.confirm.cancel}
          onConfirm={() => void doDelete()}
          onCancel={() => setDelTarget(null)}
        />
      </>
    );
  }

  // ---------------------------------------------------------------- 列表态 --
  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.pl.psTitle}</h2>
          <p className="sub">{t.pl.psSub(lists.length)}</p>
        </div>
        <div className="act">
          <input
            className="cfg-input"
            placeholder={t.pl.psNamePlaceholder}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void create();
            }}
          />
          <button className="btn sm primary" onClick={() => void create()} disabled={!newName.trim()}>
            <IconPlus size={14} /> {t.pl.psCreate}
          </button>
        </div>
      </div>
      {err && <p className="scan-error">{err}</p>}
      {lists.length === 0 ? (
        <div className="media-empty">
          <p>{t.pl.psNone}</p>
        </div>
      ) : (
        <div
          className="mgrid"
          style={{ gridTemplateColumns: "repeat(auto-fill, minmax(180px, 1fr))" }}
        >
          {lists.map((p) => {
            const cov = covers[String(p.id)] ?? [];
            // 拼贴布局随张数变化（1 张铺满 / 2 张左右分栏 / 3 张左大右二 / 4 张 2×2）
            const cls =
              cov.length === 1 ? " c1" : cov.length === 2 ? " c2" : cov.length === 3 ? " c3" : "";
            return (
              <button className="mcard pl-card" key={p.id} onClick={() => openList(p)}>
                <span
                  className={"cover2 g3" + (cov.length > 0 ? " pl-mosaic" + cls : "")}
                  aria-hidden="true"
                >
                  {cov.length > 0 ? (
                    cov.map((c, i) => <img key={i} src={assetUrl(c) ?? ""} alt="" />)
                  ) : (
                    <IconList size={34} />
                  )}
                </span>
                <span className="nm" title={p.name}>
                  {p.name}
                </span>
                <span className="ct">{t.media.tracksN(p.trackCount)}</span>
              </button>
            );
          })}
        </div>
      )}
    </>
  );
}
