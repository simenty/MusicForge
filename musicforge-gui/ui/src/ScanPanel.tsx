// P3 曲库扫描面板：只读扫描 + 规则命中 + 异常项清单。
//
// 设计边界（务必守住）：
// - **只读**：扫描不改动任何文件；清洗执行走 CLI `clean`（默认 dry-run + 回收站），
//   本面板刻意不做「一键清洗」——破坏性操作必须有显式的分级闸门，不藏在查看器里。
// - **独立组件**：与主转换流程零状态耦合；扫描只在用户显式点击时发生。
// - **明示截断**：异常项最多展示 MAX_ROWS 条，超出部分给出口（CLI --json），不谎报「全部」。
import { useState } from "react";
import {
  scanLibrary,
  refreshLibrary,
  selectDirectory,
  type LibraryRefreshReport,
  type ScanItem,
  type ScanReport,
} from "./api";
import { useLang } from "./i18n";

const CAT_CLS: Record<ScanItem["category"], string> = {
  audio: "sc-audio",
  lyrics: "sc-lyrics",
  cover: "sc-cover",
  junk: "sc-junk",
  other: "sc-other",
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
  const { t } = useLang();
  const [open, setOpen] = useState(false);
  const [dir, setDir] = useState("");
  const [recursive, setRecursive] = useState(true);
  const [scanning, setScanning] = useState(false);
  const [report, setReport] = useState<ScanReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  // P8 LibraryRefresher：增量重扫（扫描 + D17 哈希缓存刷新/入库）
  const [refreshing, setRefreshing] = useState(false);
  const [refreshed, setRefreshed] = useState<LibraryRefreshReport | null>(null);

  const browse = async () => {
    const d = await selectDirectory(dir.trim() || null, t.scan.pickDirTitle);
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

  /** P8：增量重扫——二次刷新的成本 = 元数据遍历（缓存命中零文件读取） */
  const runRefresh = async () => {
    const target = dir.trim();
    if (!target || refreshing) return;
    setRefreshing(true);
    setError(null);
    try {
      setRefreshed(await refreshLibrary(target));
    } catch (e) {
      setRefreshed(null);
      setError(String(e));
    } finally {
      setRefreshing(false);
    }
  };

  if (!open) {
    return (
      <button className="scan-toggle" onClick={() => setOpen(true)}>
        {t.scan.toggle}
      </button>
    );
  }

  const flagged = report ? report.items.filter((i) => i.rule !== null) : [];
  const shown = flagged.slice(0, MAX_ROWS);

  return (
    <div className="scan-panel">
      <div className="scan-head">
        <b>{t.scan.head}</b>
        <button className="btn sm" onClick={() => setOpen(false)}>
          {t.scan.collapse}
        </button>
      </div>
      <div className="scan-bar">
        <input
          className="val mono"
          value={dir}
          onChange={(e) => setDir(e.target.value)}
          placeholder={t.scan.dirPlaceholder}
          spellCheck={false}
          disabled={scanning}
        />
        <button className="btn sm" onClick={browse} disabled={scanning}>
          {t.scan.browse}
        </button>
        <label className="check">
          <input
            type="checkbox"
            checked={recursive}
            onChange={(e) => setRecursive(e.target.checked)}
            disabled={scanning}
          />
          <span>{t.scan.recursive}</span>
        </label>
        <button className="btn sm primary" onClick={run} disabled={scanning || !dir.trim()}>
          {scanning ? t.scan.scanning : t.scan.scan}
        </button>
        <button
          className="btn sm"
          onClick={runRefresh}
          disabled={refreshing || !dir.trim()}
          title={t.scan.refreshHint}
        >
          {refreshing ? t.scan.refreshing : t.scan.refresh}
        </button>
      </div>

      {refreshed && (
        <div className="scan-summary">
          <span>{t.scan.refreshScanned(refreshed.scannedFiles)}</span>
          <span className="sc-audio">{t.scan.refreshAudio(refreshed.audio)}</span>
          <span title={t.scan.refreshHint}>{t.scan.refreshCacheHits(refreshed.cacheHits)}</span>
          <span>{t.scan.refreshHashed(refreshed.hashed)}</span>
          <span>{t.scan.refreshSkipped(refreshed.skipped)}</span>
        </div>
      )}

      {error && <div className="scan-error">{t.scan.scanFailed(error)}</div>}

      {report && (
        <>
          <div className="scan-summary">
            <span>{t.scan.filesSeen(report.scannedFiles)}</span>
            <span>{t.scan.dirsSeen(report.scannedDirs)}</span>
            <span className="sc-audio">{t.scan.audio(report.summary.audio)}</span>
            <span className="sc-lyrics">{t.scan.lyrics(report.summary.lyrics)}</span>
            <span className="sc-cover">{t.scan.covers(report.summary.covers)}</span>
            <span className="sc-junk">{t.scan.junk(report.summary.junk)}</span>
            <span className="sc-other">{t.scan.other(report.summary.other)}</span>
            <span>{t.scan.emptyDirs(report.summary.emptyDirs)}</span>
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
                <span>{t.scan.colCategory}</span>
                <span>{t.scan.colRule}</span>
                <span>{t.scan.colFile}</span>
                <span className="ta-c">{t.scan.colSize}</span>
              </div>
              <div className="scan-table">
                {shown.map((i) => (
                  <div className="scan-row" key={i.path}>
                    <span className={"sc-cat " + (CAT_CLS[i.category] ?? "")}>
                      {t.scan.cat[i.category]}
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
                  {t.scan.truncated(flagged.length, MAX_ROWS)}
                  <code>musicforge scan {report.dir} --json</code>
                </div>
              )}
            </div>
          ) : (
            <div className="scan-note scan-clean">{t.scan.nothingToClean}</div>
          )}
        </>
      )}
    </div>
  );
}
