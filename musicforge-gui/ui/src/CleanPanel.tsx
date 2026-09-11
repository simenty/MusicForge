// P1：清洗（clean）——按规则把垃圾/孤立文件移入回收站（可整体还原）。
// 2026-09-11 界面重构 + 状态规格回灌：设计稿布局（左配置卡 / 右统计卡 + 计划清单），
// 执行确认由 window.confirm 升级为**三级闸弹层**（规格 §4.1：预览清单 → 勾选确认）。
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
import ConfirmDialog from "./ConfirmDialog";
import { IconFolder, IconWarnTri } from "./icons";
import { useLang } from "./i18n";

const MAX_ROWS = 80;
/** 弹层内清单预览条数（其余折叠为"…还有 M 条"，完整清单走导出 CSV） */
const PREVIEW_ROWS = 3;

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
  /** 三级闸弹层开关（确认后才真正执行） */
  const [confirmOpen, setConfirmOpen] = useState(false);

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

  /** 点击「移入回收站」→ 打开三级闸弹层（不在此时调用 API） */
  const requestApply = () => {
    if (!plan || busy || plan.actions.length === 0) return;
    setConfirmOpen(true);
  };

  /** 弹层确认后执行（核心不变量：未经勾选确认，绝不调用 cleanApply） */
  const doApply = async () => {
    if (!plan || applying) return;
    setConfirmOpen(false);
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
    <div className="work2">
      {/* ---------- 左：清洗配置 ---------- */}
      <aside className="panel work2-side">
        <div className="panel-head">
          <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.clean.cardTitle}</h2>
        </div>
        <p className="cfg-intro">{t.clean.panelIntro}</p>
        {!IS_SERVER_MODE && <div className="scan-note">{t.clean.serverOnly}</div>}

        <div className="cfg-block">
          <label className="cfg-label" htmlFor="clean-dir">
            {t.clean.dirLabel}
          </label>
          <input
            id="clean-dir"
            className="cfg-input mono"
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            placeholder={t.clean.dirPlaceholder}
            spellCheck={false}
            disabled={busy}
          />
          <div className="cfg-row">
            <button className="btn sm" onClick={() => void browse()} disabled={busy}>
              {t.clean.browse}
            </button>
          </div>
          <label className="cfg-label" htmlFor="clean-rules">
            {t.clean.rulesLabel}
          </label>
          <input
            id="clean-rules"
            className="cfg-input mono"
            value={rules}
            onChange={(e) => setRules(e.target.value)}
            placeholder={t.clean.rulesPlaceholder}
            spellCheck={false}
            disabled={busy}
            title={t.clean.rulesTip}
          />
          <button
            className="btn primary wide"
            onClick={() => void runPlan()}
            disabled={busy || !dir.trim()}
          >
            {planning ? t.clean.planning : t.clean.planBtn}
          </button>
        </div>

        <div className="flow-card">
          <b>{t.clean.flowTitle}</b>
          <p>{t.clean.flowBody}</p>
          <ul>
            <li>{t.clean.flowTrash}</li>
            <li>{t.clean.flowRollback}</li>
            <li>{t.clean.flowRules}</li>
          </ul>
        </div>
      </aside>

      {/* ---------- 右：统计卡 + 计划清单 ---------- */}
      <section className="work2-main">
        {error && <div className="scan-error">✕ {error}</div>}
        {result && <div className="scan-note scan-clean">✓ {result}</div>}

        {manifest && (
          <div className="panel" style={{ gap: "var(--sp-2)" }}>
            <div className="panel-head">
              <span className="plugin-note">{t.clean.rollbackLine(manifest)}</span>
              <button
                className="btn sm"
                style={{ marginLeft: "auto" }}
                onClick={() => void restore()}
                disabled={busy}
              >
                {restoring ? t.clean.restoring : t.clean.restoreBtn}
              </button>
            </div>
          </div>
        )}

        {!plan ? (
          <div className="panel">
            <p className="cfg-intro">{t.clean.emptyGuide}</p>
          </div>
        ) : (
          <>
            <div className="stat-grid c2">
              <div className="panel stat-card">
                <span className="stat-ico i-junk">
                  <IconWarnTri size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.clean.statActions}</div>
                  <div className="stat-num n-junk">{plan.actions.length}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-total">
                  <IconFolder size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.clean.statEmptyDirs}</div>
                  <div className="stat-num">{plan.empty_dirs.length}</div>
                </div>
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.clean.resultTitle}</h2>
                <span className="chip">{t.clean.nActions(plan.actions.length)}</span>
                <button
                  className="btn sm primary"
                  style={{ marginLeft: "auto" }}
                  onClick={requestApply}
                  disabled={busy || plan.actions.length === 0}
                >
                  {applying ? t.clean.applying : t.clean.applyBtn(plan.actions.length)}
                </button>
              </div>

              <div className="cfg-intro" style={{ fontSize: "var(--fs-xs)" }}>
                <span>{t.clean.nEmptyDirs(plan.empty_dirs.length)}</span> ·{" "}
                <span>{t.clean.trashRoot(plan.trash_root)}</span> ·{" "}
                <span>{t.clean.applyHint}</span>
              </div>

              {shown.length > 0 ? (
                <>
                  <div className="dt-wrap">
                    <table className="dt">
                      <thead>
                        <tr>
                          <th>{t.clean.colRule}</th>
                          <th>{t.clean.colPath}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {shown.map((a) => (
                          <tr key={a.path}>
                            <td>
                              <code className="chip">{a.rule_id}</code>
                            </td>
                            <td>
                              <div style={{ fontSize: "var(--fs-md)" }}>{shortPath(a.path)}</div>
                              <div className="path" title={a.path}>
                                {a.path}
                              </div>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {plan.actions.length > shown.length && (
                    <div className="scan-note">
                      {t.clean.truncated(plan.actions.length, MAX_ROWS)}
                    </div>
                  )}
                </>
              ) : (
                <div className="scan-note scan-clean">{t.clean.nothingToClean}</div>
              )}
            </div>
          </>
        )}
      </section>

      {/* ---------- 三级闸：预览清单 → 勾选确认 → 执行（规格 §4.1） ---------- */}
      <ConfirmDialog
        open={confirmOpen}
        title={t.confirm.titleClean}
        summary={t.confirm.cleanSummary(plan?.actions.length ?? 0)}
        items={plan ? plan.actions.slice(0, PREVIEW_ROWS).map((a) => a.path) : []}
        moreCount={plan ? Math.max(0, plan.actions.length - PREVIEW_ROWS) : 0}
        note={t.confirm.noteTrash}
        ackLabel={t.confirm.ackRestore}
        confirmLabel={t.confirm.btnClean}
        cancelLabel={t.confirm.cancel}
        busy={applying}
        onConfirm={() => void doApply()}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}
