// P4.5 重复组视图：组内对比 + 建议保留高亮 + 人工改选 + 回收站执行。
//
// 设计边界（蓝图 §P4「GUI 重复组视图」+ 项目破坏性操作分级闸门）：
// - 扫描只读；「建议保留」来自可复算评分（core 解释器），前端高亮；
// - **人工改选** = 组内 radio（用户决定保留谁），改选后前端实时重算牺牲清单；
// - 执行走 `dedupe_apply`：服务端逐条强校验路径在曲库目录内（防逃逸），
//   牺牲项全部进回收站（rollback.jsonl），**绝不直接删除**；
// - 同名候选默认仅报告（同名≠同歌；CLI `--include-same-name` 才执行）；
// - 执行前 window.confirm 二次确认（破坏性操作的最后一道人工闸）。
import { useState } from "react";
import {
  dedupeApply,
  dedupeScan,
  selectDirectory,
  type DedupeReport,
  type DupGroup,
} from "./api";

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

export default function DedupePanel() {
  const [open, setOpen] = useState(false);
  const [dir, setDir] = useState("");
  const [scanning, setScanning] = useState(false);
  const [applying, setApplying] = useState(false);
  const [report, setReport] = useState<DedupeReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  /** 用户改选：sha256 -> 保留路径（缺省 = 建议保留） */
  const [keeps, setKeeps] = useState<Map<string, string>>(new Map());

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, "选择要去重的曲库目录");
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
        ▍重复文件去重（组内对比 · 建议保留 · 可改选）
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

  const execute = async () => {
    if (!report || applying || finalSacrifices.length === 0) return;
    const ok = window.confirm(
      `把 ${finalSacrifices.length} 个重复文件移入回收站（可整体还原）？\n\n` +
        finalSacrifices.map(shortPath).join("\n")
    );
    if (!ok) return;
    setApplying(true);
    setError(null);
    try {
      const r = await dedupeApply(report.dir, finalSacrifices);
      setResult(
        `已移入回收站 ${r.moved} 项（保留项原位未动）· 回滚清单: ${r.rollback ?? "—"}`
      );
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
    <div className="scan-panel">
      <div className="scan-head">
        <b>重复文件去重（内容完全相同的文件，牺牲项进回收站）</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          收起
        </button>
      </div>
      <div className="scan-bar">
        <input
          className="val mono"
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder="输入或选择要去重的曲库目录"
          spellCheck={false}
          disabled={scanning || applying}
        />
        <button className="btn sm" onClick={browse} disabled={scanning || applying}>
          浏览
        </button>
        <button
          className="btn sm primary"
          onClick={run}
          disabled={scanning || applying || !dir.trim()}
        >
          {scanning ? "扫描中…" : "扫描"}
        </button>
      </div>

      {error && <div className="scan-error">✕ {error}</div>}
      {result && <div className="scan-note scan-clean">✓ {result}</div>}

      {report && (
        <>
          <div className="scan-summary">
            <span>文件 {report.filesSeen}</span>
            <span>重复组 {report.groups.length}</span>
            <span>同名候选 {report.sameName.length} 组（仅报告）</span>
            <span className="sc-junk">
              将牺牲 {finalSacrifices.length} 项 · 可回收 {fmtSize(savedBytes)}
            </span>
          </div>

          {report.groups.map((g, gi) => {
            const cur = keepOf(g);
            return (
              <div key={g.sha256} className="dup-group">
                <div className="dup-group-head">
                  组 {gi + 1}/{report.groups.length} · sha256 {g.sha256.slice(0, 8)}… ·{" "}
                  {fmtSize(g.size)}
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
                      <span className={"sc-cat " + (isKeep ? "sc-audio" : "sc-junk")}>
                        {isKeep ? "保留" : "牺牲"}
                      </span>
                      <span className="sc-path" title={f.path}>
                        {shortPath(f.path)}
                      </span>
                      <span className="sc-size mono">{f.score} 分</span>
                    </label>
                  );
                })}
                {g.all
                  .filter((f) => f.path !== cur)
                  .map((f) => {
                    const sac = g.sacrifices.find((s) => s.path === f.path);
                    const reason =
                      sac?.reason ??
                      "改选为牺牲（原建议保留）——按当前选择移入回收站";
                    return (
                      <div key={f.path + "-r"} className="dup-reason">
                        {reason}
                      </div>
                    );
                  })}
              </div>
            );
          })}

          {report.sameName.length > 0 && (
            <div className="dup-samename">
              <b>同名候选（同名≠同歌，默认仅报告）</b>
              {report.sameName.map((g) => (
                <div key={g.stem} className="dup-row dup-row-plain">
                  <span className="sc-cat sc-other">候选</span>
                  <span className="sc-path" title={g.keep.path}>
                    "{g.stem}": 建议保留 {shortPath(g.keep.path)}（{g.keep.score} 分），
                    另有 {g.candidates.length} 个候选
                  </span>
                </div>
              ))}
              <div className="scan-note">候选执行请走 CLI：dedupe --include-same-name</div>
            </div>
          )}

          {report.groups.length > 0 && (
            <div className="dup-actions">
              <button
                className="btn sm primary"
                onClick={execute}
                disabled={applying || finalSacrifices.length === 0}
              >
                {applying
                  ? "执行中…"
                  : `🗑 把 ${finalSacrifices.length} 个牺牲项移入回收站`}
              </button>
              <span className="scan-note">执行前会二次确认 · 回收站可整体还原</span>
            </div>
          )}
          {report.groups.length === 0 && (
            <div className="scan-note scan-clean">✓ 未发现内容重复的文件</div>
          )}
        </>
      )}
    </div>
  );
}
