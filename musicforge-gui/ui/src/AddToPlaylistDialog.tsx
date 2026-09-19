// 「加入歌单」弹层（P6.4）：选择目标歌单或当场新建并加入。
// 由曲目行尾列的 ＋ 触发（音乐库/歌单详情等）。
import { useEffect, useState } from "react";
import { playlistAdd, playlistCreate, playlistsList } from "./api";
import type { Playlist, Track } from "./api";
import { useLang } from "./i18n";

export default function AddToPlaylistDialog({
  tracks,
  onClose,
}: {
  /** 待加入的曲目（null = 关闭） */
  tracks: Track[] | null;
  onClose: () => void;
}) {
  const { t } = useLang();
  const [lists, setLists] = useState<Playlist[] | null>(null);
  const [newName, setNewName] = useState("");
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    if (!tracks) return;
    setMsg(null);
    setErr(null);
    setNewName("");
    playlistsList()
      .then(setLists)
      .catch(() => setLists([]));
  }, [tracks]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  if (!tracks || tracks.length === 0) return null;
  const ids = tracks.map((x) => x.id);

  const addTo = async (p: Playlist) => {
    try {
      const n = await playlistAdd(p.id, ids);
      setMsg(n > 0 ? t.pl.addedN(n) : t.pl.alreadyIn);
      window.setTimeout(onClose, 700);
    } catch (e) {
      setErr(String(e));
    }
  };

  const createAndAdd = async () => {
    const name = newName.trim();
    if (!name) return;
    try {
      const id = await playlistCreate(name);
      await addTo({ id, name, trackCount: 0 });
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="modal-mask" role="presentation" onClick={onClose}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="pl-add-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="pl-add-title">{t.pl.addTo}</h3>
        </div>
        <div className="modal-body">
          {lists === null ? (
            <p className="scan-note">{t.media.loading}</p>
          ) : lists.length === 0 ? (
            <p className="scan-note">{t.pl.psNone}</p>
          ) : (
            <div className="pl-pick">
              {lists.map((p) => (
                <button key={p.id} className="btn sm" onClick={() => void addTo(p)}>
                  {p.name}（{p.trackCount}）
                </button>
              ))}
            </div>
          )}
          <div className="toolbar">
            <div className="tb-left">
              <input
                className="cfg-input"
                placeholder={t.pl.psNamePlaceholder}
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void createAndAdd();
                }}
              />
              <button
                className="btn sm primary"
                disabled={!newName.trim()}
                onClick={() => void createAndAdd()}
              >
                {t.pl.psCreate}
              </button>
            </div>
          </div>
          {msg && <p className="scan-note">{msg}</p>}
          {err && <p className="scan-error">{err}</p>}
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
