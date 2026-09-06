// P3 曲库扫描面板：只读扫描 + 规则命中 + 异常项清单。
//
// 设计边界（务必守住）：
// - **只读**：扫描不改动任何文件；清洗执行走 CLI `clean`（默认 dry-run + 回收站），
//   本面板刻意不做「一键清洗」——破坏性操作必须有显式的分级闸门，不藏在查看器里。
// - **独立组件**：与主转换流程零状态耦合；扫描只在用户显式点击时发生。
// - **明示截断**：异常项最多展示 MAX_ROWS 条，超出部分给出口（CLI --json），不谎报「全部」。
import { useState } from "react";
import { scanLibrary, selectDirectory, type ScanItem, type ScanReport } from "./api";

const CAT_META: Record<ScanItem["category"], { text: string; cls: string }> = {
  audio: { text: "音频", cls: "sc-audio" },
  lyrics: { text: "歌词", cls: "sc-lyrics" },
  cover: { text: "封面", cls: "sc-cover" },
  junk: { text: "垃圾", cls: "sc-junk" },
  other: { text: "其他", cls: "sc-other" },
};

/** 异常项展示上限（完整清单走 CLI `scan <目录> --json`） */
const MAX_ROWS = 500;

function fmtSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

/** 长路径只显示末两段（与主列表 relOutput 同策略；完整路径看 hover 与 CLI --json） */
function shortPath(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 2 ? norm : "…/" + parts.slice(-2).join("/");
}

export default function ScanPanel() {
  const [open, setOpen] = useState(false);
  const [dir, setDir] = useState("");
  const [recursive, setRecursive] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [report, setReport] = useState<ScanReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, "选择要扫描的曲库目录");
    if (d) setDir(d);
  };

  const run = async () => {
    const target = dir.trim();
    if (!target || scanning) return;
    setScanning(true);
    setError(null);
    try {
      setReport(await scanLibrary(target, recursive));
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
        ▍曲库扫描（垃圾 / 孤立文件 / 命名异常）
      </button>
    );
  }

  const flagged = report ? report.items.filter((i) => i.rule !== null) : [];
  const shown = flagged.slice(0, MAX_ROWS);

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>曲库扫描（只读，不改动任何文件）</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          收起
        </button>
      </div>
      <div className="scan-bar">
        <input
          className="val mono"
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder="输入或选择要扫描的曲库目录"
          spellCheck={false}
          disabled={scanning}
        />
        <button className="btn sm" onClick={browse} disabled={scanning}>
          浏览
        </button>
        <label className="check">
          <input
            type="checkbox"
            checked={recursive}
            onChange={(e) => setRecursive(e.target.checked)}
            disabled={scanning}
          />
          <span>递归子目录</span>
        </label>
        <button className="btn sm primary" onClick={run} disabled={scanning || !dir.trim()}>
          {scanning ? "扫描中…" : "扫描"}
        </button>
      </div>

      {error && <div className="scan-error">✕ 扫描失败：{error}</div>}

      {report && (
        <>
          <div className="scan-summary">
            <span>文件 {report.scannedFiles}</span>
            <span>目录 {report.scannedDirs}</span>
            <span className="sc-audio">音频 {report.summary.audio}</span>
            <span className="sc-lyrics">歌词 {report.summary.lyrics}</span>
            <span className="sc-cover">封面 {report.summary.covers}</span>
            <span className="sc-junk">垃圾 {report.summary.junk}</span>
            <span className="sc-other">其他 {report.summary.other}</span>
            <span>空目录 {report.summary.emptyDirs}</span>
          </div>

          {report.ruleHits.length > 0 && (
            <div className="scan-rules">
              {report.ruleHits.map((h) => (
                <div key={h.id} className="scan-rule" title={h.description}>
                  <code>{h.id}</code>
                  <span className="scan-rule-n">×{h.count}</span>
                  <span className="scan-rule-desc">{h.description}</span>
                </div>
              ))}
            </div>
          )}

          {flagged.length > 0 ? (
            <div className="scan-table-wrap">
              <div className="scan-thead">
                <span>类别</span>
                <span>规则</span>
                <span>文件</span>
                <span className="ta-c">大小</span>
              </div>
              <div className="scan-table">
                {shown.map((i) => (
                  <div className="scan-row" key={i.path}>
                    <span className={"sc-cat " + (CAT_META[i.category]?.cls ?? "")}>
                      {CAT_META[i.category]?.text ?? i.category}
                    </span>
                    <code className="sc-rule-id" title={i.rule ?? ""}>
                      {i.rule}
                    </code>
                    <span className="sc-path" title={i.path}>
                      {shortPath(i.path)}
                    </span>
                    <span className="sc-size mono ta-c">{fmtSize(i.size)}</span>
                  </div>
                ))}
              </div>
              {flagged.length > shown.length && (
                <div className="scan-note">
                  共 {flagged.length} 条异常，仅显示前 {MAX_ROWS} 条 · 完整清单可用 CLI：
                  <code>musicforge scan {report.dir} --json</code>
                </div>
              )}
            </div>
          ) : (
            <div className="scan-note scan-clean">✓ 未发现可清洗项（垃圾 / 孤立文件 / 命名异常）</div>
          )}
        </>
      )}
    </div>
  );
}
