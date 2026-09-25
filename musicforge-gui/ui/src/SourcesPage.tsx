// 媒体源（P1）：目录授权 + 一键「添加并索引」+ 每源重新索引/移除。
//
// 语义：索引 = 只读扫描 + 读标签 + 写本地库（可再生）；移除源只清索引行，
// 音乐文件不受影响（移除按钮的确认文案已明示）。
import { useCallback, useEffect, useState } from "react";
import ConfirmDialog from "./ConfirmDialog";
import {
  IS_DESKTOP,
  indexSource,
  selectDirectory,
  sourcesAddAndIndex,
  sourcesList,
  sourcesRemove,
} from "./api";
import type { Source } from "./api";
import { useLang } from "./i18n";
import { fmtDate } from "./lib/format";
import { IconFolder } from "./icons";

/** 目录最后一段（无 label 时的显示名） */
function dirName(p: string): string {
  const parts = p.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : p;
}

export default function SourcesPage() {
  const { t } = useLang();
  const [rows, setRows] = useState<Source[] | null>(null);
  const [path, setPath] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // P2-18：移除源原本 window.confirm —— 改为 ConfirmDialog（可样式化/可测/可 i18n）
  const [pendingRemove, setPendingRemove] = useState<Source | null>(null);

  const reload = useCallback(() => {
    if (!IS_DESKTOP) return;
    sourcesList()
      .then(setRows)
      .catch(() => setRows([]));
  }, []);
  useEffect(() => reload(), [reload]);

  const pick = useCallback(async () => {
    try {
      const dir = await selectDirectory(null, t.app.pickFolderTitle);
      if (dir) setPath(dir);
    } catch {
      /* 用户取消或对话框不可用：保持现状 */
    }
  }, [t]);

  const addIndex = useCallback(async () => {
    const p = path.trim();
    if (!p || busy) return;
    setBusy(true);
    setMsg(null);
    setErr(null);
    try {
      const r = await sourcesAddAndIndex(p);
      setMsg(t.media.indexDone(r.outcome));
      setPath("");
      reload();
    } catch (e) {
      setErr(t.media.indexFail(String(e)));
    } finally {
      setBusy(false);
    }
  }, [path, busy, t, reload]);

  const reindex = useCallback(
    async (id: number) => {
      if (busy) return;
      setBusy(true);
      setMsg(null);
      setErr(null);
      try {
        const o = await indexSource(id);
        setMsg(t.media.indexDone(o));
        reload();
      } catch (e) {
        setErr(t.media.indexFail(String(e)));
      } finally {
        setBusy(false);
      }
    },
    [busy, t, reload]
  );

  const remove = useCallback(async () => {
    if (!pendingRemove) return;
    setBusy(true);
    setErr(null);
    try {
      await sourcesRemove(pendingRemove.id);
      reload();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
      setPendingRemove(null);
    }
  }, [pendingRemove, t, reload]);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  return (
    <>
      <div className="media-head">
        <div>
          <h2>{t.media.sourcesTitle}</h2>
          <p className="sub">{t.media.sourcesSub}</p>
        </div>
      </div>

      <div className="panel">
        <div className="src-inline">
          <input
            className="val mono"
            style={{ flex: "1 1 260px", minWidth: 220 }}
            value={path}
            onChange={(e) => setPath(e.target.value)}
            placeholder={t.media.srcAddPlaceholder}
            spellCheck={false}
            aria-label={t.media.srcAddPlaceholder}
          />
          <button className="btn sm" onClick={() => void pick()} disabled={busy}>
            {t.media.srcBrowse}
          </button>
          <button
            className="btn primary sm"
            onClick={() => void addIndex()}
            disabled={busy || !path.trim()}
          >
            {busy ? t.media.srcIndexing : t.media.srcAddIndex}
          </button>
        </div>
        {msg && (
          <div className="scan-note" style={{ marginTop: "var(--sp-2)" }}>
            {msg}
          </div>
        )}
        {err && (
          <div className="scan-note" style={{ marginTop: "var(--sp-2)", color: "var(--danger)" }}>
            {err}
          </div>
        )}
      </div>

      <div className="srcs">
        {rows === null ? (
          <div className="media-empty">
            <p>{t.media.loading}</p>
          </div>
        ) : rows.length === 0 ? (
          <div className="media-empty">
            <p>{t.media.srcEmpty}</p>
          </div>
        ) : (
          rows.map((s) => (
            <div className="src-card" key={s.id}>
              <div className="row1">
                <span className="ic">
                  <IconFolder />
                </span>
                <b title={s.path}>{s.label ?? dirName(s.path)}</b>
                <span className="chip green" style={{ marginLeft: "auto" }}>
                  {t.media.srcTracks(s.tracksCount)}
                </span>
              </div>
              <div className="path">{s.path}</div>
              <div className="path">{t.media.srcAddedAt(fmtDate(s.addedAt))}</div>
              <div className="act">
                <button
                  className="btn sm"
                  onClick={() => void reindex(s.id)}
                  disabled={busy}
                >
                  {busy ? t.media.srcIndexing : t.media.srcReindex}
                </button>
                <button
                  className="btn sm danger"
                  onClick={() => setPendingRemove(s)}
                  disabled={busy}
                >
                  {t.media.srcRemove}
                </button>
              </div>
            </div>
          ))
        )}
      </div>
      <ConfirmDialog
        open={pendingRemove !== null}
        busy={busy}
        title={t.media.srcRemoveConfirm}
        summary={t.media.srcRemoveConfirm}
        ackLabel={t.media.srcRemoveConfirm}
        confirmLabel={t.player.confirm}
        cancelLabel={t.player.cancel}
        onConfirm={remove}
        onCancel={() => setPendingRemove(null)}
      />
    </>
  );
}
