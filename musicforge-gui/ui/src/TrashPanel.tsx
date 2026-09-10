// P1：回收站还原（trash restore）——按回滚清单整体还原（组织/清洗的执行产物）。
//
// 服务端约束（AUD-6）：manifest 必须位于 `.musicforge` 体系内且为 *.jsonl，
// 否则 403 MF-TRASH-MANIFEST-INVALID（防持 token 者越权还原任意路径）。
// 还原会覆盖恢复路径上的同名文件（如存在）——因此强制二次确认。
import { useState } from "react";
import { IS_SERVER_MODE, trashRestore } from "./api";
import { useLang } from "./i18n";

export default function TrashPanel() {
  const { t } = useLang();
  const [manifest, setManifest] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    const m = manifest.trim();
    if (!m || busy) return;
    if (!window.confirm(t.trash.confirmRestore)) return;
    setBusy(true);
    setError(null);
    try {
      const r = await trashRestore(m);
      setResult(t.trash.restored(r.restored));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>{t.trash.head}</b>
        <span className="plugin-note">{t.trash.note}</span>
      </div>

      {!IS_SERVER_MODE && <div className="scan-note">{t.trash.serverOnly}</div>}

      <div className="scan-bar">
        <input
          className="val mono"
          value={manifest}
          onChange={(e) => setManifest(e.target.value)}
          placeholder={t.trash.manifestPlaceholder}
          spellCheck={false}
          disabled={busy}
        />
        <button className="btn sm primary" onClick={() => void run()} disabled={busy || !manifest.trim()}>
          {busy ? t.trash.restoring : t.trash.restoreBtn}
        </button>
      </div>

      <div className="scan-note">{t.trash.manifestHint}</div>

      {error && <div className="scan-error">✕ {error}</div>}
      {result && <div className="scan-note scan-clean">✓ {result}</div>}
    </div>
  );
}
