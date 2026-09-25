// P1：回收站还原（trash restore）——按回滚清单整体还原（组织/清洗的执行产物）。
// 2026-09-11 视觉统一：设计语言组件（cfg 排版 + flow 说明卡）。
//
// 服务端约束（AUD-6）：manifest 必须位于 `.musicforge` 体系内且为 *.jsonl，
// 否则 403 MF-TRASH-MANIFEST-INVALID（防持 token 者越权还原任意路径）。
// 还原会覆盖恢复路径上的同名文件（如存在）——因此强制二次确认。
import { useState } from "react";
import { IS_SERVER_MODE, trashRestore } from "./api";
import ConfirmDialog from "./ConfirmDialog";
import { useLang } from "./i18n";

export default function TrashPanel() {
  const { t } = useLang();
  const [manifest, setManifest] = useState("");
  // P2-18：还原原本 window.confirm —— 改为 ConfirmDialog
  const [pendingRestore, setPendingRestore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const m = manifest.trim();
    if (!m || busy) return;
    setBusy(true);
    setError(null);
    try {
      const r = await trashRestore(m);
      setResult(t.trash.restored(r.restored));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setPendingRestore(false);
    }
  };

  return (
    <div className="panel" style={{ gap: "var(--sp-3)" }}>
      <div className="panel-head">
        <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.trash.cardTitle}</h2>
        <span className="plugin-note" style={{ marginLeft: "auto" }}>
          {t.trash.note}
        </span>
      </div>

      {!IS_SERVER_MODE && <div className="scan-note">{t.trash.serverOnly}</div>}

      <div className="cfg-block">
        <div className="cfg-row">
          <input
            id="trash-manifest"
            className="cfg-input mono"
            value={manifest}
            onChange={(e) => setManifest(e.target.value)}
            placeholder={t.trash.manifestPlaceholder}
            spellCheck={false}
            disabled={busy}
          />
          <button
            className="btn primary"
            onClick={() => setPendingRestore(true)}
            disabled={busy || !manifest.trim()}
          >
            {busy ? t.trash.restoring : t.trash.restoreBtn}
          </button>
        </div>
      </div>

      <div className="flow-card">
        <b>{t.trash.flowTitle}</b>
        <p>{t.trash.manifestHint}</p>
        <ul>
          <li>{t.trash.flowOverwrite}</li>
          <li>{t.trash.flowConfirm}</li>
        </ul>
      </div>

      {error && <div className="scan-error">✕ {error}</div>}
      {result && <div className="scan-note scan-clean">✓ {result}</div>}

      <ConfirmDialog
        open={pendingRestore}
        busy={busy}
        title={t.trash.confirmRestore}
        summary={t.trash.confirmRestore}
        ackLabel={t.trash.confirmRestore}
        confirmLabel={t.player.confirm}
        cancelLabel={t.player.cancel}
        onConfirm={() => void run()}
        onCancel={() => setPendingRestore(false)}
      />
    </div>
  );
}
