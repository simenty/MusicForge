// CUE 分轨（P4 工具箱）：选择 → 检视 → 三级闸确认 → 切分 → 报告。
//
// 能力全在 Rust 侧（core::cue + commands::cue_tool）；本组件只管流程状态与呈现。
// 检视**不触碰音频**（存在性 + ffmpeg 需求判定）；切分是长任务（阻塞池执行），
// 失败轨不落盘——报告与 CLI `musicforge split --json` 同形。
import { useState } from "react";
import { useRequestGuard } from "./hooks/useRequestGuard";
import { IS_DESKTOP, cueInspect, cuePick, cueSplit, selectDirectory } from "./api";
import type { CueInspect, CueSplitReport } from "./api";
import ConfirmDialog from "./ConfirmDialog";
import { useLang } from "./i18n";
import { IconCheckBox, IconFolder, IconWarnTri } from "./icons";

export default function CuePanel() {
  const { t } = useLang();
  // P2-17：选档守卫
  const cueGuard = useRequestGuard();
  const [cuePath, setCuePath] = useState<string | null>(null);
  const [info, setInfo] = useState<CueInspect | null>(null);
  const [outDir, setOutDir] = useState("");
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<CueSplitReport | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [confirm, setConfirm] = useState(false);

  if (!IS_DESKTOP) {
    return (
      <div className="media-empty">
        <p>{t.media.desktopOnly}</p>
      </div>
    );
  }

  const pick = async () => {
    const token = cueGuard.token();
    setErr(null);
    try {
      const p = await cuePick();
      if (!p) return; // 用户取消
      if (!cueGuard.isStale(token)) {
        setReport(null);
        setCuePath(p);
        // 默认输出目录 = CUE 同目录（最常见的期望）
        const sep = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
        setOutDir(sep > 0 ? p.slice(0, sep) : "");
        setInfo(await cueInspect(p));
      }
    } catch (e) {
      if (!cueGuard.isStale(token)) {
        setInfo(null);
        setErr(String(e));
      }
    }
  };

  const pickOut = async () => {
    const d = await selectDirectory(outDir || null, t.cue.outDir);
    if (d) setOutDir(d);
  };

  const doSplit = async () => {
    if (!cuePath || !outDir) return;
    setConfirm(false);
    setBusy(true);
    setErr(null);
    try {
      setReport(await cueSplit(cuePath, outDir));
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  const n = info?.tracks.length ?? 0;
  const ok = report?.tracks.length ?? 0;
  const fail = report?.failed.length ?? 0;

  return (
    <div className="panel">
      <div className="panel-head">
        <h2>{t.cue.tab}</h2>
      </div>
      <p className="scan-note">{t.cue.sub}</p>

      {/* ① 选文件 */}
      <div className="toolbar">
        <div className="tb-left">
          <button className="btn" onClick={() => void pick()} disabled={busy}>
            <IconFolder />
            {cuePath ? t.cue.change : t.cue.pick}
          </button>
          {cuePath && (
            <span className="path" title={cuePath}>
              {cuePath}
            </span>
          )}
        </div>
      </div>

      {err && (
        <p className="scan-error">
          <IconWarnTri /> {err}
        </p>
      )}

      {info && (
        <>
          {/* ② 检视摘要（不触碰音频） */}
          <p className="scan-note">
            {t.cue.inspectAlbum}: <b>{info.album ?? "—"}</b> · {t.cue.inspectPerformer}:{" "}
            <b>{info.performer ?? "—"}</b> · {t.cue.audioFile}:{" "}
            <b>{info.audioFile ?? "—"}</b> · {t.cue.trackCount(n)}
          </p>
          {!info.audioExists && (
            <p className="scan-error">
              <IconWarnTri /> {t.cue.audioMissing}
            </p>
          )}
          {info.needsFfmpeg && (
            <p className="scan-note">
              <IconWarnTri /> {t.cue.needsFfmpeg}
            </p>
          )}

          {/* 曲目预览（与媒体库同一行网格） */}
          <div className="tracks" style={{ marginTop: "var(--sp-2)" }}>
            <div className="vt-head">
              <span>{t.cue.colTrack}</span>
              <span>{t.cue.colTitle}</span>
              <span>{t.cue.colPerformer}</span>
              <span />
              <span />
              <span />
            </div>
            {info.tracks.map((tr) => (
              <div className="vt-row" key={tr.number}>
                <span className="vt-idx">{tr.number}</span>
                <span className="vt-main">
                  <b>{tr.title ?? "—"}</b>
                </span>
                <span className="vt-alb">{tr.performer ?? info.performer ?? "—"}</span>
                <span className="vt-num" />
                <span className="vt-num" />
                <span />
              </div>
            ))}
          </div>

          {/* ③ 输出目录 + 三级闸触发 */}
          <div className="toolbar" style={{ marginTop: "var(--sp-3)" }}>
            <div className="tb-left">
              <button className="btn" onClick={() => void pickOut()} disabled={busy}>
                <IconFolder />
                {t.cue.pickOutDir}
              </button>
              <span className="path" title={outDir}>
                {outDir || "—"}
              </span>
              <button
                className="btn primary"
                onClick={() => setConfirm(true)}
                disabled={busy || !outDir || !info.audioExists || n === 0}
              >
                {t.cue.splitAction}
              </button>
            </div>
          </div>
        </>
      )}

      {busy && <p className="scan-note">{t.cue.splitting}</p>}

      {report && (
        <>
          <p className="scan-note">
            <IconCheckBox /> {t.cue.doneOk(ok, fail)}
          </p>
          {fail > 0 && (
            <p className="scan-error">
              <IconWarnTri /> {t.cue.doneHint}
            </p>
          )}
          <div className="tracks">
            <div className="vt-head">
              <span>{t.cue.colTrack}</span>
              <span>{t.cue.colTitle}</span>
              <span>{t.cue.colFile}</span>
              <span />
              <span />
              <span />
            </div>
            {report.tracks.map((r) => (
              <div className="vt-row" key={r.path}>
                <span className="vt-idx">{r.index}</span>
                <span className="vt-main">
                  <b>{r.title ?? "—"}</b>
                  <span>{r.durationSecs}s</span>
                </span>
                <span className="vt-alb" title={r.path}>
                  {r.path}
                </span>
                <span className="vt-num" />
                <span className="vt-num" />
                <span />
              </div>
            ))}
            {report.failed.map((f) => (
              <div className="vt-row" key={`f${f.track}`}>
                <span className="vt-idx">{f.track}</span>
                <span className="vt-main">
                  <b>{f.reason}</b>
                </span>
                <span className="vt-alb" />
                <span className="vt-num" />
                <span className="vt-num" />
                <span />
              </div>
            ))}
          </div>
        </>
      )}

      {/* 三级闸 ②③：预览 + 勾选确认 */}
      <ConfirmDialog
        open={confirm}
        title={t.cue.splitAction}
        summary={
          <>
            {t.cue.confirmSummary(n)} <b>{outDir}</b>
          </>
        }
        items={info?.tracks.slice(0, 8).map((x) => `${x.number}. ${x.title ?? "—"}`)}
        moreCount={Math.max(0, n - 8)}
        note={<span className="path">{cuePath ?? ""}</span>}
        ackLabel={t.cue.confirmAck}
        confirmLabel={t.cue.confirmGo}
        cancelLabel={t.confirm.cancel}
        busy={busy}
        onConfirm={() => void doSplit()}
        onCancel={() => setConfirm(false)}
      />
    </div>
  );
}
