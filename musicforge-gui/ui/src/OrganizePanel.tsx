// P1：整理（organize）——按命名模板把曲库归档到目标根。
// 2026-09-11 界面重构 + 状态规格回灌：设计稿布局（左配置卡 / 右统计卡 + 归档映射表），
// 执行确认由 window.confirm 升级为**三级闸弹层**（规格 §4.1：预览清单 → 勾选确认）。
//
// 安全语义（与后端 /api/organize/* 一致）：
// - **预览只读**：plan 绝不移动任何文件；
// - **执行需二次确认**：三级闸弹层 + 后端 confirm:true（403 MF-OP-NEEDS-YES 兜底）；
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
import ConfirmDialog from "./ConfirmDialog";
import { IconBan, IconCheckBox, IconFolder, IconWarnTri } from "./icons";
import { useLang } from "./i18n";

const MAX_ROWS = 80;
const DEFAULT_TPL = "{artist}/{album}/{title}";
/** 弹层内清单预览条数（其余折叠为"…还有 M 条"） */
const PREVIEW_ROWS = 3;

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
  /** 三级闸弹层开关 */
  const [confirmOpen, setConfirmOpen] = useState(false);

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

  /** 点击「执行整理」→ 打开三级闸弹层（不在此时调用 API） */
  const requestApply = () => {
    if (!plan || busy || plan.counts.planned === 0) return;
    setConfirmOpen(true);
  };

  /** 弹层确认后执行（核心不变量：未经勾选确认，绝不调用 organizeApply） */
  const doApply = async () => {
    if (!plan || applying) return;
    setConfirmOpen(false);
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
    <div className="work2">
      {/* ---------- 左：整理配置 ---------- */}
      <aside className="panel work2-side">
        <div className="panel-head">
          <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.organize.cardTitle}</h2>
        </div>
        <p className="cfg-intro">{t.organize.panelIntro}</p>
        <div className="scan-note">{t.organize.readonlyNote}</div>
        {!IS_SERVER_MODE && <div className="scan-note">{t.organize.serverOnly}</div>}

        <div className="cfg-block">
          <label className="cfg-label" htmlFor="org-dir">
            {t.organize.dirLabel}
          </label>
          <input
            id="org-dir"
            className="cfg-input mono"
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            placeholder={t.organize.dirPlaceholder}
            spellCheck={false}
            disabled={busy}
          />
          <div className="cfg-row">
            <button className="btn sm" onClick={() => void browse()} disabled={busy}>
              {t.organize.browse}
            </button>
          </div>

          <label className="cfg-label" htmlFor="org-target">
            {t.organize.targetLabel}
          </label>
          <input
            id="org-target"
            className="cfg-input mono"
            value={targetRoot}
            onChange={(e) => setTargetRoot(e.target.value)}
            placeholder={t.organize.targetPlaceholder}
            spellCheck={false}
            disabled={busy}
          />
          <div className="cfg-row">
            <button className="btn sm" onClick={() => void browseTarget()} disabled={busy}>
              {t.organize.browse}
            </button>
          </div>

          <label className="cfg-label" htmlFor="org-template">
            {t.organize.templateLabel}
          </label>
          <input
            id="org-template"
            className="cfg-input mono"
            value={template}
            onChange={(e) => setTemplate(e.target.value)}
            placeholder={DEFAULT_TPL}
            spellCheck={false}
            disabled={busy}
            title={t.organize.templateTip}
          />
          <button
            className="btn primary wide"
            onClick={() => void runPlan()}
            disabled={busy || !dir.trim()}
          >
            {planning ? t.organize.planning : t.organize.planBtn}
          </button>
        </div>

        <div className="flow-card">
          <b>{t.organize.flowTitle}</b>
          <p>{t.organize.flowBody}</p>
          <ul>
            <li>{t.organize.flowMove}</li>
            <li>{t.organize.flowRollback}</li>
            <li>{t.organize.flowConflict}</li>
          </ul>
        </div>
      </aside>

      {/* ---------- 右：统计卡 + 归档映射 ---------- */}
      <section className="work2-main">
        {error && <div className="scan-error">✕ {error}</div>}
        {result && <div className="scan-note scan-clean">✓ {result}</div>}

        {manifest && (
          <div className="panel" style={{ gap: "var(--sp-2)" }}>
            <div className="panel-head">
              <span className="plugin-note">{t.organize.rollbackLine(manifest)}</span>
              <button
                className="btn sm"
                style={{ marginLeft: "auto" }}
                onClick={() => void restore()}
                disabled={busy}
              >
                {restoring ? t.organize.restoring : t.organize.restoreBtn}
              </button>
            </div>
          </div>
        )}

        {!plan ? (
          <div className="panel">
            <p className="cfg-intro">{t.organize.emptyGuide}</p>
          </div>
        ) : (
          <>
            <div className="stat-grid">
              <div className="panel stat-card">
                <span className="stat-ico i-audio">
                  <IconFolder size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.organize.statPlanned}</div>
                  <div className="stat-num n-audio">{plan.counts.planned}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-lyric">
                  <IconCheckBox size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.organize.statInPlace}</div>
                  <div className="stat-num n-lyric">{plan.counts.in_place}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-junk">
                  <IconWarnTri size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.organize.statSkipped}</div>
                  <div className="stat-num n-junk">{plan.counts.skipped_conflict}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-total">
                  <IconBan size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.organize.statConflict}</div>
                  <div className="stat-num">{plan.counts.conflict_never}</div>
                </div>
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.organize.resultTitle}</h2>
                <span className="chip">{t.organize.countPlanned(plan.counts.planned)}</span>
                <button
                  className="btn sm primary"
                  style={{ marginLeft: "auto" }}
                  onClick={requestApply}
                  disabled={busy || plan.counts.planned === 0}
                >
                  {applying ? t.organize.applying : t.organize.applyBtn(plan.counts.planned)}
                </button>
              </div>

              <div className="cfg-intro" style={{ fontSize: "var(--fs-xs)" }}>
                <span>{t.organize.applyHint}</span>
              </div>

              {shown.length > 0 ? (
                <>
                  <div className="dt-wrap">
                    <table className="dt">
                      <thead>
                        <tr>
                          <th>{t.organize.colSource}</th>
                          <th>{t.organize.colTarget}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {shown.map((i) => (
                          <tr key={i.source} title={`${i.source} → ${i.target}${i.note ? ` (${i.note})` : ""}`}>
                            <td>
                              <div style={{ fontSize: "var(--fs-md)" }}>{shortPath(i.source)}</div>
                              <div className="path">{i.source}</div>
                            </td>
                            <td>
                              <div style={{ fontSize: "var(--fs-md)" }}>{shortPath(i.target)}</div>
                              <div className="path">{i.target}</div>
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {plan.plan.items.length > shown.length && (
                    <div className="scan-note">
                      {t.organize.truncated(plan.plan.items.length, MAX_ROWS)}
                    </div>
                  )}
                </>
              ) : (
                <div className="scan-note scan-clean">{t.organize.noChanges}</div>
              )}
            </div>
          </>
        )}
      </section>

      {/* ---------- 三级闸：预览清单 → 勾选确认 → 执行（规格 §4.1） ---------- */}
      <ConfirmDialog
        open={confirmOpen}
        title={t.confirm.titleOrganize}
        summary={t.confirm.organizeSummary(plan?.counts.planned ?? 0)}
        items={plan ? plan.plan.items.slice(0, PREVIEW_ROWS).map((i) => `${i.source} → ${i.target}`) : []}
        moreCount={plan ? Math.max(0, plan.plan.items.length - PREVIEW_ROWS) : 0}
        note={t.confirm.noteTrash}
        ackLabel={t.confirm.ackRestore}
        confirmLabel={t.confirm.btnOrganize}
        cancelLabel={t.confirm.cancel}
        busy={applying}
        onConfirm={() => void doApply()}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}
