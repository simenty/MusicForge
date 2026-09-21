// 歌单（P6.4）：列表态（新建/打开）与详情态（播放全部/移除曲目/重命名/删除）。
// 曲目来源：音乐库行尾的 ＋（AddToPlaylistDialog）。
import { useCallback, useEffect, useRef, useState } from "react";
import {
  IS_DESKTOP,
  playlistCleanupPreview,
  playlistCreate,
  playlistDelete,
  playlistExport,
  playlistMove,
  playlistRemove,
  playlistRename,
  playlistSmartCleanup,
  playlistTracks,
  playlistsCovers,
  playlistsList,
} from "./api";
import type { Playlist, Track } from "./api";
import ConfirmDialog from "./ConfirmDialog";
import TrackRow from "./TrackRow";
import SelectionBar from "./SelectionBar";
import { useSelection } from "./hooks/useSelection";
import { useLang } from "./i18n";
import { assetUrl } from "./lib/asset";
import { IconList, IconPlus } from "./icons";

export default function PlaylistsPage({
  onPlay,
  focusId = null,
  onQueue,
  onPlayNext,
}: {
  onPlay?: (tracks: Track[], index: number) => Promise<void>;
  /** P6.14 搜索跳转：命中的歌单 id（消费一次即进入详情） */
  focusId?: number | null;
  /** P6.16 加入队列 */
  onQueue?: (tracks: Track[]) => Promise<void>;
  /** P6.17 下一首播放 */
  onPlayNext?: (tracks: Track[]) => Promise<void>;
}) {
  const { t } = useLang();
  const selApi = useSelection();
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
  /** 智能清理（P6.24）：弹层 / 预览 / 执行中 */
  const [cleanupOpen, setCleanupOpen] = useState(false);
  const [cleanupPreview, setCleanupPreview] = useState<{
    duplicateCount: number;
    orphanCount: number;
  } | null>(null);
  const [cleanupBusy, setCleanupBusy] = useState(false);

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
    // P6.19 批量操作
    const list = items ?? [];
    const selectedTracks = list.filter((x) => selApi.sel.has(String(x.id)));
    const bulkQueue = async () => {
      if (selectedTracks.length && onQueue) await onQueue(selectedTracks);
      selApi.toggleSelMode();
    };
    const bulkPlayNext = async () => {
      if (selectedTracks.length && onPlayNext) await onPlayNext(selectedTracks);
      selApi.toggleSelMode();
    };
    const bulkRemove = async () => {
      if (!open) return;
      const ids = [...selApi.sel].map(Number);
      for (const id of ids) {
        try {
          await playlistRemove(open.id, id);
        } catch (e) {
          setErr(String(e));
        }
      }
      setItems((prev) => prev?.filter((x) => !selApi.sel.has(String(x.id))) ?? prev);
      setOpen((p) => (p ? { ...p, trackCount: Math.max(0, p.trackCount - ids.length) } : p));
      reload();
      selApi.toggleSelMode();
    };
    /** 智能清理（P6.24）：先拉预览，无问题则直接提示、不弹确认层 */
    const openCleanup = async () => {
      if (!open) return;
      setCleanupBusy(true);
      try {
        const p = await playlistCleanupPreview(open.id);
        if (p.duplicateCount === 0 && p.orphanCount === 0) {
          setNote(t.pl.psCleanupNone);
          return;
        }
        setCleanupPreview(p);
        setCleanupOpen(true);
      } catch (e) {
        setErr(String(e));
      } finally {
        setCleanupBusy(false);
      }
    };
    const doCleanup = async () => {
      if (!open) return;
      setCleanupBusy(true);
      try {
        const r = await playlistSmartCleanup(open.id);
        setNote(t.pl.psCleanupDone(r.removedDuplicates, r.removedOrphans));
        setCleanupOpen(false);
        setCleanupPreview(null);
        const fresh = await playlistTracks(open.id).catch(() => []);
        setItems(fresh);
        reload();
      } catch (e) {
        setErr(String(e));
      } finally {
        setCleanupBusy(false);
      }
    };
    /** 重复副本标题预览（来自当前显示列表；失效条目无名称，不单列） */
    const dupIds = (items ?? []).map((x) => x.id);
    const dupOnly: number[] = dupIds.filter((id, i) => dupIds.indexOf(id) !== i);
    const uniqueDupCount = new Set(dupOnly).size;
    const dupPreview: string[] = Array.from(new Set(dupOnly))
      .slice(0, 5)
      .map((id) => {
        const tr = (items ?? []).find((x) => x.id === id);
        return tr ? (tr.title ?? tr.path) : String(id);
      });
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
              className={"btn sm" + (selApi.selMode ? " on" : "")}
              onClick={selApi.toggleSelMode}
              disabled={!items || items.length === 0}
            >
              {t.player.select}
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
            <button
              className="btn sm"
              onClick={() => void openCleanup()}
              disabled={!items || items.length === 0 || cleanupBusy}
            >
              {t.pl.psCleanup}
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
                onQueue={onQueue ? () => void onQueue([r]) : undefined}
                onPlayNext={onPlayNext ? () => void onPlayNext([r]) : undefined}
                selectable={selApi.selMode}
                selected={selApi.has(String(r.id))}
                onToggleSelect={() => selApi.toggle(String(r.id))}
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
        <ConfirmDialog
          open={cleanupOpen}
          title={t.pl.psCleanupTitle}
          summary={
            cleanupPreview
              ? t.pl.psCleanupSummary(
                  cleanupPreview.duplicateCount,
                  cleanupPreview.orphanCount
                )
              : ""
          }
          items={dupPreview}
          moreCount={Math.max(0, uniqueDupCount - dupPreview.length)}
          ackLabel={t.pl.psCleanupAck}
          confirmLabel={t.pl.psCleanupConfirm}
          cancelLabel={t.confirm.cancel}
          busy={cleanupBusy}
          onConfirm={() => void doCleanup()}
          onCancel={() => {
            setCleanupOpen(false);
            setCleanupPreview(null);
          }}
        />
        {selApi.selMode && (
          <SelectionBar
            count={selApi.count}
            total={list.length}
            onAddToQueue={bulkQueue}
            onPlayNext={bulkPlayNext}
            onRemove={bulkRemove}
            removeLabel={t.pl.psRemove}
            onSelectAll={() => selApi.selectAll(list.map((x) => String(x.id)))}
            onClear={selApi.toggleSelMode}
          />
        )}
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
