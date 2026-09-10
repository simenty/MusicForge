// P1：整理（organize）——按命名模板把曲库归档到目标根。
//
// 安全语义（与后端 /api/organize/* 一致）：
// - **预览只读**：plan 绝不移动任何文件；
// - **执行需二次确认**：window.confirm + 后端 confirm:true（403 MF-OP-NEEDS-YES 兜底）；
// - **可整体还原**：执行后给出 rollback_manifest，可一键还原。
//
// 形态边界：桌面（Tauri）无对应命令 → api 层抛 MF-SERVER-ONLY，面板顶部显式提示。
import { useState } from "react";
import {
  IS_SERVER_MODE,
  organizeApply,
  organizePlan,
  selectDirectory,
  trashRestore,
  type OrganizePlan,
} from "./api";
import { useLang } from "./i18n";

const MAX_ROWS = 80;
const DEFAULT_TPL = "{artist}/{album}/{title}";

function shortPath(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 3 ? norm : "…/" + parts.slice(-3).join("/");
}

export default function OrganizePanel() {
  const { t } = useLang();
  const [dir, setDir] = useState("");
  const [targetRoot, setTargetRoot] = useState("");
  const [template, setTemplate] = useState(DEFAULT_TPL);
  const [planning, setPlanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [plan, setPlan] = useState<OrganizePlan | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [manifest, setManifest] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const busy = planning || applying || restoring;

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, t.organize.pickDirTitle);
    if (d) setDir(d);
  };
  const browseTarget = async () => {
    const d = await selectDirectory(targetRoot.trim() || null, t.organize.pickTargetTitle);
    if (d) setTargetRoot(d);
  };

  const runPlan = async () => {
    if (!dir.trim() || busy) return;
    setPlanning(true);
    setError(null);
    setResult(null);
    setManifest(null);
    try {
      setPlan(
        await organizePlan({
          dir: dir.trim(),
          template: template.trim() || DEFAULT_TPL,
          targetRoot: targetRoot.trim() || null,
        })
      );
    } catch (e) {
      setPlan(null);
      setError(String(e));
    } finally {
      setPlanning(false);
    }
  };

  const runApply = async () => {
    if (!plan || busy) return;
    const n = plan.counts.planned;
    if (!window.confirm(t.organize.confirmApply(n))) return;
    setApplying(true);
    setError(null);
    try {
      const out = await organizeApply({
        dir: dir.trim(),
        template: template.trim() || DEFAULT_TPL,
        targetRoot: targetRoot.trim() || null,
      });
      setResult(t.organize.resultLine(out.moved, out.skipped, out.failed));
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
    if (!window.confirm(t.organize.confirmRestore)) return;
    setRestoring(true);
    setError(null);
    try {
      const r = await trashRestore(manifest);
      setResult(t.organize.restored(r.restored));
      setManifest(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setRestoring(false);
    }
  };

  const shown = plan ? plan.plan.items.slice(0, MAX_ROWS) : [];

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>{t.organize.head}</b>
        <span className="plugin-note">{t.organize.readonlyNote}</span>
      </div>

      {!IS_SERVER_MODE && <div className="scan-note">{t.organize.serverOnly}</div>}

      <div className="scan-bar">
        <input
          className="val mono"
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder={t.organize.dirPlaceholder}
          spellCheck={false}
          disabled={busy}
        />
        <button className="btn sm" onClick={() => void browse()} disabled={busy}>
          {t.organize.browse}
        </button>
      </div>

      <div className="scan-bar">
        <input
          className="val mono"
          value={targetRoot}
          onChange={(e) => setTargetRoot(e.target.value)}
          placeholder={t.organize.targetPlaceholder}
          spellCheck={false}
          disabled={busy}
        />
        <button className="btn sm" onClick={() => void browseTarget()} disabled={busy}>
          {t.organize.browse}
        </button>
        <input
          className="val mono"
          value={template}
          onChange={(e) => setTemplate(e.target.value)}
          placeholder={DEFAULT_TPL}
          spellCheck={false}
          disabled={busy}
          title={t.organize.templateTip}
        />
        <button
          className="btn sm"
          onClick={() => void runPlan()}
          disabled={busy || !dir.trim()}
        >
          {planning ? t.organize.planning : t.organize.planBtn}
        </button>
      </div>

      {error && <div className="scan-error">✕ {error}</div>}
      {result && <div className="scan-note scan-clean">✓ {result}</div>}

      {plan && (
        <>
          <div className="scan-summary">
            <span className="sc-audio">{t.organize.countPlanned(plan.counts.planned)}</span>
            <span>{t.organize.countInPlace(plan.counts.in_place)}</span>
            <span className="sc-junk">
              {t.organize.countSkipped(plan.counts.skipped_conflict)}
            </span>
            <span>{t.organize.countConflict(plan.counts.conflict_never)}</span>
          </div>

          {shown.length > 0 ? (
            <div className="scan-table-wrap">
              <div className="scan-thead">
                <span>{t.organize.colSource}</span>
                <span>{t.organize.colTarget}</span>
              </div>
              <div className="scan-table">
                {shown.map((i) => (
                  <div
                    className="scan-row"
                    key={i.source}
                    style={{ gridTemplateColumns: "minmax(0,1fr) minmax(0,1fr)" }}
                    title={`${i.source} → ${i.target}${i.note ? ` (${i.note})` : ""}`}
                  >
                    <span className="sc-path">{shortPath(i.source)}</span>
                    <span className="sc-path">{shortPath(i.target)}</span>
                  </div>
                ))}
              </div>
              {plan.plan.items.length > shown.length && (
                <div className="scan-note">
                  {t.organize.truncated(plan.plan.items.length, MAX_ROWS)}
                </div>
              )}
            </div>
          ) : (
            <div className="scan-note scan-clean">{t.organize.noChanges}</div>
          )}

          <div className="dup-actions">
            <button
              className="btn sm primary"
              onClick={() => void runApply()}
              disabled={busy || plan.counts.planned === 0}
            >
              {applying ? t.organize.applying : t.organize.applyBtn(plan.counts.planned)}
            </button>
            <span className="scan-note">{t.organize.applyHint}</span>
          </div>
        </>
      )}

      {manifest && (
        <div className="dup-actions">
          <button className="btn sm" onClick={() => void restore()} disabled={busy}>
            {restoring ? t.organize.restoring : t.organize.restoreBtn}
          </button>
          <span className="plugin-note">{t.organize.rollbackLine(manifest)}</span>
        </div>
      )}
    </div>
  );
}
