// 上次会话持久化（P6.12）：队列 / 当前位置 / 播放进度——关掉应用再打开可续播。
//
// 只存 localStorage（本机、无网络）；读取时逐字段校验——localStorage 可能被手工
// 修改或跨版本残留，脏数据一律判废返回 null（宁可没有会话，不可崩播放器）。
import type { QueueItem } from "./types";

const KEY = "mf.session";

export interface SavedSession {
  items: QueueItem[];
  /** 当前曲目在 items 中的下标 */
  index: number;
  /** 该曲目的播放进度（ms） */
  positionMs: number;
  /** 保存时刻（epoch ms） */
  ts: number;
}

/** 保存会话（配额满 / 隐私模式等异常一律静默——持久化失败不能影响播放）。 */
export function saveSession(s: SavedSession): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* 静默 */
  }
}

/** 读取并校验会话；任何字段不合法 → null（绝不抛异常）。 */
export function loadSession(): SavedSession | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<SavedSession>;
    if (!Array.isArray(v.items) || v.items.length === 0) return null;
    const items = v.items.filter(isQueueItem);
    if (items.length === 0) return null;
    const idx = v.index;
    const index =
      Number.isInteger(idx) && (idx as number) >= 0 && (idx as number) < items.length
        ? (idx as number)
        : 0;
    const positionMs =
      typeof v.positionMs === "number" && Number.isFinite(v.positionMs) && v.positionMs >= 0
        ? v.positionMs
        : 0;
    const ts = typeof v.ts === "number" && Number.isFinite(v.ts) ? v.ts : 0;
    return { items, index, positionMs, ts };
  } catch {
    return null;
  }
}

/** 清除会话（测试与显式结束场景）。 */
export function clearSession(): void {
  try {
    localStorage.removeItem(KEY);
  } catch {
    /* 静默 */
  }
}

function isQueueItem(v: unknown): v is QueueItem {
  if (typeof v !== "object" || v === null) return false;
  const it = v as Record<string, unknown>;
  return (
    typeof it.trackId === "number" &&
    typeof it.path === "string" &&
    (it.title === null || typeof it.title === "string") &&
    (it.artist === null || typeof it.artist === "string") &&
    (it.durationMs === null || typeof it.durationMs === "number")
  );
}
