// X15：风格码写回 genre（把文件名 `[Y23-S01-E01-C01-C02-V00]` 解析出的风格/场景
// 写成音频文件的 genre 标签）。
//
// 能力全部来自 `musicforge_core::stylecode`（plan_genre_writes / apply_genre_writes），
// 与 CLI `musicforge genre` **同一规划/执行语义**（status 四态同形），故两条入口的产物可互换。
//
// 安全纪律（与 CLI 默认档一致）：
// - `replaceAll` 默认**关**（FillMissingOnly）：已有非空 genre 的文件跳过，**绝不覆盖用户数据**；
// - 写入是**改写文件元数据**，故强制二次确认（ConfirmDialog），不用 window.confirm。
import { useState } from "react";
import { IS_DESKTOP, genreApply, genrePlan, selectDirectory } from "./api";
import type { GenrePlan } from "./lib/types";
import { useLang } from "./i18n";
import ConfirmDialog from "./ConfirmDialog";

/** 预览条数上限：十万曲库全量渲染会拖垮面板 */
const PREVIEW_MAX = 20;

export default function GenrePanel({ codebookPath }: { codebookPath?: string }) {
  const { t } = useLang();
  const [dir, setDir] = useState("");
  const [replaceAll, setReplaceAll] = useState(false);
  const [plan, setPlan] = useState<GenrePlan | null>(null);
  const [busy, setBusy] = useState<"plan" | "apply" | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pendingApply, setPendingApply] = useState(false);

  const statusLabel = (s: string) =>
    s === "will-write"
      ? t.genre.stWillWrite
      : s === "has-genre"
        ? t.genre.stHasGenre
        : s === "no-code"
          ? t.genre.stNoCode
          : t.genre.stNoLabel;

  const runPlan = async () => {
    const d = dir.trim();
    if (!d || busy) return;
    setBusy("plan");
    setError(null);
    setResult(null);
    try {
      setPlan(await genrePlan(d, codebookPath, replaceAll));
    } catch (e) {
      setPlan(null);
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const runApply = async () => {
    const d = dir.trim();
    if (!d || busy) return;
    setBusy("apply");
    setError(null);
    setResult(null);
    try {
      // 重新规划后落盘（与 CLI 同序：规划 → 执行）——避免拿着过期规划写入。
      const r = await genreApply(d, codebookPath, replaceAll);
      setResult(t.genre.applied(r.written, r.failed));
      setPlan(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
      setPendingApply(false);
    }
  };

  const willWrite = plan?.items.filter((i) => i.status === "will-write") ?? [];

  return (
    <div className="panel" style={{ gap: "var(--sp-3)" }}>
      <div className="panel-head">
        <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.genre.cardTitle}</h2>
        <span className="plugin-note" style={{ marginLeft: "auto" }}>
          {t.genre.note}
        </span>
      </div>

      {!IS_DESKTOP && <div className="scan-note">{t.styleCode.open}</div>}

      <div className="cfg-block">
        <div className="cfg-row">
          <label className="lbl" htmlFor="genre-dir">
            {t.genre.dirLabel}
          </label>
          <input
            id="genre-dir"
            className="cfg-input mono"
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            placeholder={t.genre.dirPlaceholder}
            spellCheck={false}
            disabled={busy !== null}
          />
          <button
            type="button"
            className="btn sm"
            disabled={busy !== null}
            onClick={async () => {
              const p = await selectDirectory(dir.trim() || null, t.genre.pickDirTitle);
              if (p) setDir(p);
            }}
          >
            {t.genre.browse}
          </button>
        </div>
        <div className="cfg-row">
          <label className="check">
            <input
              type="checkbox"
              checked={replaceAll}
              onChange={(e) => setReplaceAll(e.target.checked)}
              disabled={busy !== null}
            />
            <span>{t.genre.replaceAll}</span>
          </label>
          <span className="hint">{t.genre.replaceAllHint}</span>
        </div>
        {!codebookPath && <div className="scan-note">{t.genre.noCodebook}</div>}
      </div>

      <div className="cfg-row">
        <button
          type="button"
          className="btn"
          disabled={!dir.trim() || busy !== null}
          onClick={() => void runPlan()}
        >
          {busy === "plan" ? t.genre.planning : t.genre.planBtn}
        </button>
        <button
          type="button"
          className="btn primary"
          disabled={!dir.trim() || willWrite.length === 0 || busy !== null}
          onClick={() => setPendingApply(true)}
        >
          {busy === "apply" ? t.genre.applying : t.genre.applyBtn}
        </button>
      </div>

      {plan && (
        <>
          <div className="scan-note">
            {t.genre.summary(plan.will, plan.hasGenre, plan.noCode, plan.noLabel)}
          </div>
          {willWrite.length === 0 ? (
            <div className="scan-note">{t.genre.noWrite}</div>
          ) : (
            <div className="pl-pick">
              <div className="qd-head">
                <span>{t.genre.previewHead}</span>
              </div>
              {willWrite.slice(0, PREVIEW_MAX).map((it) => (
                <div className="preview-line" key={it.path}>
                  <span className="preview-label">{statusLabel(it.status)}</span>
                  {it.path} → {it.genre}
                </div>
              ))}
              {willWrite.length > PREVIEW_MAX && (
                <div className="scan-note">
                  {t.genre.previewMore(willWrite.length - PREVIEW_MAX)}
                </div>
              )}
            </div>
          )}
        </>
      )}

      {error && (
        <div className="scan-error" role="alert">
          {error}
        </div>
      )}
      {result && (
        <div className="scan-note" role="status">
          {result}
        </div>
      )}

      <ConfirmDialog
        open={pendingApply}
        title={t.genre.confirmTitle}
        summary={t.genre.confirmBody(willWrite.length)}
        items={willWrite.slice(0, PREVIEW_MAX).map((i) => i.path)}
        moreCount={Math.max(0, willWrite.length - PREVIEW_MAX)}
        note={t.genre.replaceAllHint}
        ackLabel={t.genre.ack}
        confirmLabel={t.genre.applyBtn}
        cancelLabel={t.confirm.cancel}
        busy={busy === "apply"}
        onConfirm={() => void runApply()}
        onCancel={() => setPendingApply(false)}
      />
    </div>
  );
}
