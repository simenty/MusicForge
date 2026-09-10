// P1：清洗（clean）——按规则把垃圾/孤立文件移入回收站（可整体还原）。
//
// 安全语义（与后端 /api/clean/* 一致）：
// - 预览只读（dry-run）；执行需二次确认 + confirm:true；
// - 产物进 `<dir>/.musicforge/trash/<task>/`，**绝不直接删除**；
// - 执行后给出 rollback_manifest，可一键还原。
import { useState } from "react";
import {
  IS_SERVER_MODE,
  cleanApply,
  cleanPlan,
  selectDirectory,
  trashRestore,
  type CleanPlan,
} from "./api";
import { useLang } from "./i18n";

const MAX_ROWS = 80;

function shortPath(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 3 ? norm : "…/" + parts.slice(-3).join("/");
}

export default function CleanPanel() {
  const { t } = useLang();
  const [dir, setDir] = useState("");
  const [rules, setRules] = useState("");
  const [planning, setPlanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [plan, setPlan] = useState<CleanPlan | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [manifest, setManifest] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const busy = planning || applying || restoring;

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, t.clean.pickDirTitle);
    if (d) setDir(d);
  };

  const runPlan = async () => {
    if (!dir.trim() || busy) return;
    setPlanning(true);
    setError(null);
    setResult(null);
    setManifest(null);
    try {
      setPlan(await cleanPlan(dir.trim(), rules.trim() || undefined));
    } catch (e) {
      setPlan(null);
      setError(String(e));
    } finally {
      setPlanning(false);
    }
  };

  const runApply = async () => {
    if (!plan || busy || plan.actions.length === 0) return;
    if (!window.confirm(t.clean.confirmApply(plan.actions.length))) return;
    setApplying(true);
    setError(null);
    try {
      const out = await cleanApply(dir.trim(), rules.trim() || undefined);
      setResult(t.clean.resultLine(out.moved, out.dirs_removed));
      setManifest(out.rollback_manifest);
      setPlan(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const restore = async () => {
    if (!manifest || busy) return;
    if (!window.confirm(t.clean.confirmRestore)) return;
    setRestoring(true);
    setError(null);
    try {
      const r = await trashRestore(manifest);
      setResult(t.clean.restored(r.restored));
      setManifest(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoring(false);
    }
  };

  const shown = plan ? plan.actions.slice(0, MAX_ROWS) : [];

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>{t.clean.head}</b>
        <span className="plugin-note">{t.clean.trashNote}</span>
      </div>

      {!IS_SERVER_MODE && <div className="scan-note">{t.clean.serverOnly}</div>}

      <div className="scan-bar">
        <input
          className="val mono"
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder={t.clean.dirPlaceholder}
          spellCheck={false}
          disabled={busy}
        />
        <button className="btn sm" onClick={() => void browse()} disabled={busy}>
          {t.clean.browse}
        </button>
        <input
          className="val mono"
          value={rules}
          onChange={(e) => setRules(e.target.value)}
          placeholder={t.clean.rulesPlaceholder}
          spellCheck={false}
          disabled={busy}
          title={t.clean.rulesTip}
        />
        <button className="btn sm" onClick={() => void runPlan()} disabled={busy || !dir.trim()}>
          {planning ? t.clean.planning : t.clean.planBtn}
        </button>
      </div>

      {error && <div className="scan-error">✕ {error}</div>}
      {result && <div className="scan-note scan-clean">✓ {result}</div>}

      {plan && (
        <>
          <div className="scan-summary">
            <span className="sc-junk">{t.clean.nActions(plan.actions.length)}</span>
            <span>{t.clean.nEmptyDirs(plan.empty_dirs.length)}</span>
            <span className="plugin-note">{t.clean.trashRoot(plan.trash_root)}</span>
          </div>

          {shown.length > 0 ? (
            <div className="scan-table-wrap">
              <div className="scan-thead">
                <span>{t.clean.colRule}</span>
                <span>{t.clean.colPath}</span>
              </div>
              <div className="scan-table">
                {shown.map((a) => (
                  <div
                    className="scan-row"
                    key={a.path}
                    style={{ gridTemplateColumns: "120px minmax(0,1fr)" }}
                    title={a.path}
                  >
                    <code className="sc-rule-id">{a.rule_id}</code>
                    <span className="sc-path">{shortPath(a.path)}</span>
                  </div>
                ))}
              </div>
              {plan.actions.length > shown.length && (
                <div className="scan-note">
                  {t.clean.truncated(plan.actions.length, MAX_ROWS)}
                </div>
              )}
            </div>
          ) : (
            <div className="scan-note scan-clean">{t.clean.nothingToClean}</div>
          )}

          <div className="dup-actions">
            <button
              className="btn sm primary"
              onClick={() => void runApply()}
              disabled={busy || plan.actions.length === 0}
            >
              {applying ? t.clean.applying : t.clean.applyBtn(plan.actions.length)}
            </button>
            <span className="scan-note">{t.clean.applyHint}</span>
          </div>
        </>
      )}

      {manifest && (
        <div className="dup-actions">
          <button className="btn sm" onClick={() => void restore()} disabled={busy}>
            {restoring ? t.clean.restoring : t.clean.restoreBtn}
          </button>
          <span className="plugin-note">{t.clean.rollbackLine(manifest)}</span>
        </div>
      )}
    </div>
  );
}
