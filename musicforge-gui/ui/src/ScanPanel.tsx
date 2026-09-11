// P3 曲库扫描面板（2026-09-11 界面重构：设计稿布局 —— 左配置卡 / 右统计卡 + 结果清单）
//
// 设计边界（务必守住）：
// - **只读**：扫描不改动任何文件；清洗执行走清洗面板/CLI（默认 dry-run + 回收站），
//   本面板刻意不做「一键清洗」——破坏性操作必须有显式的分级闸门，不藏在查看器里。
// - **独立组件**：与主转换流程零状态耦合；扫描只在用户显式点击时发生。
// - **明示截断**：异常项最多展示 MAX_ROWS 条，超出部分给出口（CLI --json），不谎报「全部」。
// - **导出 CSV 为前端生成**（Blob，不新增后端能力）；内容 = 当前命中规则的项。
import { useState } from "react";
import {
  scanLibrary,
  refreshLibrary,
  selectDirectory,
  type LibraryRefreshReport,
  type ScanItem,
  type ScanReport,
} from "./api";
import { IconScan, IconMusic, IconCheckBox, IconWarnTri } from "./icons";
import { useLang } from "./i18n";

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

/**
 * @param hideCollapse 二级菜单场景（左侧导航已选定本工具）：默认展开且隐藏「收起」，
 *                     避免与左侧导航语义重复。缺省 false = 保持原有折叠行为。
 */
export default function ScanPanel({ hideCollapse = false }: { hideCollapse?: boolean } = {}) {
  const { t } = useLang();
  const [open, setOpen] = useState(true);
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

  /** 导出命中项为 CSV（前端生成 Blob；服务端形态 = 浏览器下载，桌面形态走 webview 下载） */
  const exportCsv = () => {
    if (!report) return;
    const flagged = report.items.filter((i) => i.rule !== null);
    const esc = (s: string) => `"${s.replace(/"/g, '""')}"`;
    const lines = [
      ["category", "rule", "path", "size"].join(","),
      ...flagged.map((i) =>
        [i.category, i.rule ?? "", i.path, String(i.size)].map(esc).join(",")
      ),
    ];
    const blob = new Blob(["\ufeff" + lines.join("\r\n")], {
      type: "text/csv;charset=utf-8",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `musicforge-scan-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
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
    <div className="work2">
      {/* ---------- 左：扫描配置 ---------- */}
      <aside className="panel work2-side">
        <div className="panel-head">
          <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.scan.cardTitle}</h2>
          {!hideCollapse && (
            <button className="btn sm" style={{ marginLeft: "auto" }} onClick={() => setOpen(false)}>
              {t.scan.collapse}
            </button>
          )}
        </div>
        <p className="cfg-intro">{t.scan.panelIntro}</p>

        <div className="cfg-block">
          <label className="cfg-label" htmlFor="scan-dir">
            {t.scan.dirLabel}
          </label>
          <input
            id="scan-dir"
            className="cfg-input mono"
            value={dir}
            onChange={(e) => setDir(e.target.value)}
            placeholder={t.scan.dirPlaceholder}
            spellCheck={false}
            disabled={scanning}
          />
          <div className="cfg-row">
            <button className="btn sm" onClick={browse} disabled={scanning}>
              {t.scan.browse}
            </button>
            <label className="check" style={{ marginLeft: "auto" }}>
              <input
                type="checkbox"
                checked={recursive}
                onChange={(e) => setRecursive(e.target.checked)}
                disabled={scanning}
              />
              <span>{t.scan.recursive}</span>
            </label>
          </div>
          <button
            className="btn primary wide"
            onClick={run}
            disabled={scanning || !dir.trim()}
          >
            <IconScan />
            {scanning ? t.scan.scanning : t.scan.scan}
          </button>
          <button
            className="btn sm wide"
            onClick={runRefresh}
            disabled={refreshing || !dir.trim()}
            title={t.scan.refreshHint}
          >
            {refreshing ? t.scan.refreshing : t.scan.refresh}
          </button>
        </div>

        <div className="flow-card">
          <b>{t.scan.readonlyTitle}</b>
          <p>{t.scan.readonlyBody}</p>
          <ul>
            <li>{t.scan.flowDedupe}</li>
            <li>{t.scan.flowOrganize}</li>
            <li>{t.scan.flowClean}</li>
          </ul>
        </div>
      </aside>

      {/* ---------- 右：统计卡 + 结果清单 ---------- */}
      <section className="work2-main">
        {error && <div className="scan-error">{t.scan.scanFailed(error)}</div>}

        {refreshed && (
          <div className="panel" style={{ gap: "var(--sp-2)" }}>
            <div className="scan-summary">
              <span>{t.scan.refreshScanned(refreshed.scannedFiles)}</span>
              <span className="sc-audio">{t.scan.refreshAudio(refreshed.audio)}</span>
              <span title={t.scan.refreshHint}>{t.scan.refreshCacheHits(refreshed.cacheHits)}</span>
              <span>{t.scan.refreshHashed(refreshed.hashed)}</span>
              <span>{t.scan.refreshSkipped(refreshed.skipped)}</span>
            </div>
          </div>
        )}

        {!report ? (
          <div className="panel">
            <p className="cfg-intro">{t.scan.emptyGuide}</p>
          </div>
        ) : (
          <>
            <div className="stat-grid">
              <div className="panel stat-card">
                <span className="stat-ico i-total">
                  <IconScan size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.scan.statTotal}</div>
                  <div className="stat-num">{report.scannedFiles}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-audio">
                  <IconMusic size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.scan.statAudio}</div>
                  <div className="stat-num n-audio">{report.summary.audio}</div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-lyric">
                  <IconCheckBox size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.scan.statLyricsCover}</div>
                  <div className="stat-num n-lyric">
                    {report.summary.lyrics + report.summary.covers}
                  </div>
                </div>
              </div>
              <div className="panel stat-card">
                <span className="stat-ico i-junk">
                  <IconWarnTri size={18} />
                </span>
                <div className="stat-meta">
                  <div className="stat-lbl">{t.scan.statJunkOther}</div>
                  <div className="stat-num n-junk">
                    {report.summary.junk + report.summary.other}
                  </div>
                </div>
              </div>
            </div>

            <div className="panel">
              <div className="panel-head">
                <h2 style={{ margin: 0, fontSize: "var(--fs-lg)" }}>{t.scan.resultTitle}</h2>
                <span className="chip">{t.scan.resultCount(flagged.length)}</span>
                <button
                  className="btn sm"
                  style={{ marginLeft: "auto" }}
                  onClick={exportCsv}
                  disabled={!flagged.length}
                >
                  {t.scan.exportCsv}
                </button>
              </div>

              <div className="cfg-intro" style={{ fontSize: "var(--fs-xs)" }}>
                {t.scan.filesSeen(report.scannedFiles)} · {t.scan.dirsSeen(report.scannedDirs)} ·{" "}
                {t.scan.emptyDirs(report.summary.emptyDirs)}
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
                <>
                  <div className="dt-wrap">
                    <table className="dt">
                      <thead>
                        <tr>
                          <th>{t.scan.colCategory}</th>
                          <th>{t.scan.colRule}</th>
                          <th>{t.scan.colFile}</th>
                          <th style={{ textAlign: "right" }}>{t.scan.colSize}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {shown.map((i: ScanItem) => (
                          <tr key={i.path}>
                            <td>
                              <span className={"chip cat-" + i.category}>
                                {t.scan.cat[i.category]}
                              </span>
                            </td>
                            <td>
                              <code className="chip">{i.rule}</code>
                            </td>
                            <td>
                              <div style={{ fontSize: "var(--fs-md)" }}>{shortPath(i.path)}</div>
                              <div className="path" title={i.path}>
                                {i.path}
                              </div>
                            </td>
                            <td className="num">{fmtSize(i.size)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  {flagged.length > shown.length && (
                    <div className="scan-note">
                      {t.scan.truncated(flagged.length, MAX_ROWS)}
                      <code>musicforge scan {report.dir} --json</code>
                    </div>
                  )}
                </>
              ) : (
                <div className="scan-note scan-clean">{t.scan.nothingToClean}</div>
              )}
            </div>
          </>
        )}
      </section>
    </div>
  );
}
