// LRC 解析（纯函数——独立文件以符合 react-refresh 的"组件文件只导出组件"规则）。

export interface LrcLine {
  /** 毫秒时间戳；-1 = 纯文本（无时间戳） */
  t: number;
  s: string;
}

/** 解析 LRC（支持一行多时间戳；无时间戳 → 逐行纯文本，t=-1）。 */
export function parseLrc(text: string): LrcLine[] {
  const out: LrcLine[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const m = raw.match(/^((?:\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?\])+)\s*(.*)$/);
    if (!m) continue;
    const body = m[2].trim();
    const stamps = m[1].match(/\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]/g) ?? [];
    for (const st of stamps) {
      const g = st.match(/\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]/);
      if (!g) continue;
      // 小数位对齐：`.5` → 500ms（padEnd 到 3 位再截断）
      const frac = g[3] ? Number(g[3].padEnd(3, "0").slice(0, 3)) : 0;
      out.push({ t: Number(g[1]) * 60_000 + Number(g[2]) * 1000 + frac, s: body });
    }
  }
  if (out.length === 0) {
    const lines = text
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    return lines.map((s) => ({ t: -1, s }));
  }
  return out.sort((a, b) => a.t - b.t);
}

/** 当前行索引：最后一个 `t ≤ pos` 的行（纯文本 → -1）。 */
export function activeLine(lines: LrcLine[], posMs: number): number {
  if (lines.length === 0 || lines[0].t < 0) return -1;
  let idx = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].t <= posMs) idx = i;
    else break;
  }
  return idx;
}
