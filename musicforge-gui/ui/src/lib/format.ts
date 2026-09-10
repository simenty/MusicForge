/**
 * 纯函数集合（P1-3 拆分：从 App.tsx 抽出，便于 vitest 单测）。
 *
 * 约束：这里不得依赖 React、DOM 或 api 层——只做字符串/数字变换，
 * 因此可以在 Node 环境直接测试（无需 jsdom）。
 */

/** 取路径最后一段（跨平台分隔符） */
export function fileName(p: string): string {
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  return i >= 0 ? p.slice(i + 1) : p;
}

/** 输出路径只显示末两段，避免长路径撑破表格 */
export function relOutput(p: string): string {
  const norm = p.replace(/\\/g, "/");
  const parts = norm.split("/").filter(Boolean);
  return parts.length <= 2 ? norm : "…/" + parts.slice(-2).join("/");
}

/** 耗时展示：秒带一位小数 → 分秒 → 时分 */
export function formatDuration(ms: number): string {
  if (ms <= 0) return "—";
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}.${Math.floor((ms % 1000) / 100)}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}m`;
}

/** 取扩展名（小写、不含点）；无扩展名返回空串 */
export function extOf(p: string): string {
  return (/\.(?:([A-Za-z0-9]+))$/.exec(p)?.[1] ?? "").toLowerCase();
}

/** 完成百分比（total=0 时为 0；上限 100） */
export function percent(done: number, total: number): number {
  return total > 0 ? Math.min(100, Math.round((done / total) * 100)) : 0;
}
