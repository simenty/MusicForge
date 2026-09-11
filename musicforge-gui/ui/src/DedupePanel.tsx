// P4.5 重复组视图（2026-09-11 界面重构 + 状态规格回灌：设计稿布局 —— 左配置卡 / 右统计卡 + 组清单；
// 执行确认由 window.confirm 升级为**三级闸弹层**（规格 §4.1：预览清单 → 勾选确认））。
//
// 设计边界（蓝图 §P4「GUI 重复组视图」+ 项目破坏性操作分级闸门）：
// - 扫描只读；「建议保留」来自可复算评分（core 解释器），前端高亮；
// - **人工改选** = 组内 radio（用户决定保留谁），改选后前端实时重算牺牲清单；
// - 执行走 `dedupe_apply`：服务端逐条强校验路径在曲库目录内（防逃逸），
//   牺牲项全部进回收站（rollback.jsonl），**绝不直接删除**；
// - 同名候选默认仅报告（同名≠同歌；CLI `--include-same-name` 才执行）；
// - 执行前经三级闸弹层（预览牺牲清单 + 勾选确认）。
import { useState } from "react";
import {
  dedupeApply,
  dedupeScan,
  selectDirectory,
  type DedupeReport,
  type DupGroup,
} from "./api";
import ConfirmDialog from "./ConfirmDialog";
import { IconBan, IconCheckBox, IconCopy, IconWarnTri } from "./icons";
import { useLang } from "./i18n";

/** 弹层内清单预览条数（其余折叠为"…还有 M 条"） */
const PREVIEW_ROWS = 3;

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

function shortPath(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 2 ? norm : "…/" + parts.slice(-2).join("/");
}

/** @param hideCollapse 同 ScanPanel：左侧二级菜单场景下默认展开且隐藏「收起」 */
export default function DedupePanel({ hideCollapse = false }: { hideCollapse?: boolean } = {}) {
  const { t } = useLang();
  const [open, setOpen] = useState(true);
  const [dir, setDir] = useState("");
  const [scanning, setScanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [report, setReport] = useState<DedupeReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  /** 用户改选：sha256 -> 保留路径（缺省 = 建议保留） */
  const [keeps, setKeeps] = useState<Map<string, string>>(new Map());
  /** 三级闸弹层开关 */
  const [confirmOpen, setConfirmOpen] = useState(false);

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, t.dedupe.pickDirTitle);
    if (d) setDir(d);
  };

  const run = async () => {
    const target = dir.trim();
    if (!target || scanning) return;
    setScanning(true);
    setError(null);
    setResult(null);
    setKeeps(new Map());
    try {
      setReport(await dedupeScan(target));
    } catch (e) {
      setReport(null);
      setError(String(e));
    } finally {
      setScanning(false);
    }
  };

  if (!open) {
    return (
      <button className="scan-toggle" onClick={() => setOpen(true)}>
        {t.dedupe.toggle}
      </button>
    );
  }

  const keepOf = (g: DupGroup): string => keeps.get(g.sha256) ?? g.keep.path;

  /** 改选后的牺牲清单 = 每组里非保留成员 */
  const finalSacrifices = report
    ? report.groups.flatMap((g) =>
        g.all.filter((f) => f.path !== keepOf(g)).map((f) => f.path)
      )
    : [];
  const savedBytes = report
    ? report.groups.flatMap((g) => g.all).reduce((acc, f) => {
        return finalSacrifices.includes(f.path) ? acc + f.size : acc;
      }, 0)
    : 0;

  /** 点击「执行去重」→ 打开三级闸弹层（不在此时调用 API） */
  const requestExecute = () => {
    if (!report || applying || finalSacrifices.length === 0) return;
    setConfirmOpen(true);
  };

  /** 弹层确认后执行（核心不变量：未经勾选确认，绝不调用 dedupeApply） */
  const doExecute = async () => {
    if (!report || applying) return;
    setConfirmOpen(false);
    setApplying(true);
    setError(null);
    try {
      const r = await dedupeApply(report.dir, finalSacrifices);
      setResult(t.dedupe.movedResult(r.moved, r.rollback ?? "—"));
      // 重新扫描刷新视图
      setReport(await dedupeScan(report.dir));
      setKeeps(new Map());
    } catch (e) {
      setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  return (
    <div className="work2">
      {/* ---------- 左：扫描配置 ---------- */}
      <aside className="panel work2-side">
        <div className="panel-head">
          <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.dedupe.cardTitle}</h2>
          {!hideCollapse && (
            <button className="btn sm" style={{ marginLeft: "auto" }} onClick={() => setOpen(false)}>
              {t.dedupe.collapse}
            </button>
          )}
        </div>
        <p className="cfg-intro">{t.dedupe.panelIntro}</p>

        <div className="cfg-block">
          <label className="cfg-label" htmlFor="dup-dir">
            {t.dedupe.dirLabel}
          </label>
          <input
            id="dup-dir"
            className="cfg-input mono"
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            placeholder={t.dedupe.dirPlaceholder}
            spellCheck={false}
            disabled={scanning || applying}
          />
          <div className="cfg-row">
            <button className="btn sm" onClick={browse} disabled={scanning || applying}>
              {t.dedupe.browse}
            </button>
          </div>
          <button
            className="btn primary wide"
            onClick={run}
            disabled={scanning || applying || !dir.trim()}
          >
            {scanning ? t.dedupe.scanning : t.dedupe.scan}
          </button>
        </div>

        <div className="flow-card">
          <b>{t.dedupe.flowTitle}</b>
          <p>{t.dedupe.flowBody}</p>
          <ul>
            <li>{t.dedupe.flowKeep}</li>
            <li>{t.dedupe.flowSacrifice}</li>
            <li>{t.dedupe.flowSameName}</li>
          </ul>
        </div>
      </aside>

      {/* ---------- 右：统计卡 + 组清单 ---------- */}
      <section className="work2-main">
        {error && <div className="scan-error">✕ {error}</div>}
        {result && <div className="scan-note scan-clean">✓ {result}</div>}

        {!report ? (
          <div className="panel">
            <p className="cfg-intro">{t.dedupe.emptyGuide}</p>
          </div>
        ) : (
          <>
            <div className="stat-grid">
              <div className="panel stat-card">
                <span className="stat-ico i-total">
                  <IconCopy size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.dedupe.statSeen}</div>
                  <div className="stat-num">{report.filesSeen}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-audio">
                  <IconCheckBox size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.dedupe.statGroups}</div>
                  <div className="stat-num n-audio">{report.groups.length}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-junk">
                  <IconWarnTri size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.dedupe.statSacrifice}</div>
                  <div className="stat-num n-junk">{finalSacrifices.length}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-lyric">
                  <IconBan size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.dedupe.statSameName}</div>
                  <div className="stat-num n-lyric">{report.sameName.length}</div>
                </div>
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.dedupe.resultTitle}</h2>
                <span className="chip">{t.dedupe.groups(report.groups.length)}</span>
                <button
                  className="btn sm primary"
                  style={{ marginLeft: "auto" }}
                  onClick={requestExecute}
                  disabled={applying || finalSacrifices.length === 0}
                >
                  {applying ? t.dedupe.executing : t.dedupe.execute(finalSacrifices.length)}
                </button>
              </div>

              <div className="cfg-intro" style={{ fontSize: "var(--fs-xs)" }}>
                {t.dedupe.willSacrifice(finalSacrifices.length, fmtSize(savedBytes))} ·{" "}
                {t.dedupe.executeHint}
              </div>

              {report.groups.length === 0 ? (
                <div className="scan-note scan-clean">{t.dedupe.noDuplicates}</div>
              ) : (
                report.groups.map((g, gi) => {
                  const cur = keepOf(g);
                  return (
                    <div key={g.sha256} className="dup-group">
                      <div className="dup-group-head">
                        {t.dedupe.groupHead(gi + 1, report.groups.length, g.sha256.slice(0, 8), fmtSize(g.size))}
                      </div>
                      {g.all.map((f) => {
                        const isKeep = f.path === cur;
                        return (
                          <label
                            key={f.path}
                            className={"dup-row" + (isKeep ? " dup-keep" : "")}
                            title={isKeep ? g.keep.detail : ""}
                          >
                            <input
                              type="radio"
                              name={`dup-${g.sha256}`}
                              checked={isKeep}
                              onChange={() =>
                                setKeeps((m) => new Map(m).set(g.sha256, f.path))
                              }
                              disabled={applying}
                            />
                            <span className={"chip " + (isKeep ? "cat-audio" : "cat-junk")}>
                              {isKeep ? t.dedupe.keep : t.dedupe.sacrifice}
                            </span>
                            <span className="sc-path" title={f.path}>
                              {shortPath(f.path)}
                            </span>
                            <span className="sc-size mono">{t.dedupe.score(f.score)}</span>
                          </label>
                        );
                      })}
                      {g.all
                        .filter((f) => f.path !== cur)
                        .map((f) => {
                          const sac = g.sacrifices.find((s) => s.path === f.path);
                          const reason = sac?.reason ?? t.dedupe.overriddenReason;
                          return (
                            <div key={f.path + "-r"} className="dup-reason">
                              {reason}
                            </div>
                          );
                        })}
                    </div>
                  );
                })
              )}

              {report.sameName.length > 0 && (
                <div className="dup-samename">
                  <b>{t.dedupe.sameNameHead}</b>
                  {report.sameName.map((g) => (
                    <div key={g.stem} className="dup-row dup-row-plain">
                      <span className="chip cat-other">{t.dedupe.candidate}</span>
                      <span className="sc-path" title={g.keep.path}>
                        {t.dedupe.sameNameLine(
                          g.stem,
                          shortPath(g.keep.path),
                          g.keep.score,
                          g.candidates.length
                        )}
                      </span>
                    </div>
                  ))}
                  <div className="scan-note">{t.dedupe.sameNameCli}</div>
                </div>
              )}
            </div>
          </>
        )}
      </section>

      {/* ---------- 三级闸：预览牺牲清单 → 勾选确认 → 执行（规格 §4.1） ---------- */}
      <ConfirmDialog
        open={confirmOpen}
        title={t.confirm.titleDedupe}
        summary={t.confirm.dedupeSummary(finalSacrifices.length)}
        items={finalSacrifices.slice(0, PREVIEW_ROWS).map(shortPath)}
        moreCount={Math.max(0, finalSacrifices.length - PREVIEW_ROWS)}
        note={t.confirm.noteTrash}
        ackLabel={t.confirm.ackRestore}
        confirmLabel={t.confirm.btnDedupe}
        cancelLabel={t.confirm.cancel}
        busy={applying}
        onConfirm={() => void doExecute()}
        onCancel={() => setConfirmOpen(false)}
      />
    </div>
  );
}
