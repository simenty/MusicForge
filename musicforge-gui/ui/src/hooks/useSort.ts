// 排序偏好持久化（P6.21）：按页面 key 记忆上次选择的排序键，刷新后保持。
//
// P1-14：持久化值必须按「当前列表实际支持的排序键」校验。旧版本残留或手工改写的脏值
// 会被后端**静默**回退（曲库的 played_at / liked_at → ORDER BY t.path），表现为
// 「用户明明选了排序，列表顺序却没变且无任何提示」。现改为：校验失败即回退默认，
// 并通过第三个返回值把失效的键交给 UI 显式提示——绝不静默。
import { useEffect, useState } from "react";
import type { TrackSortField } from "../lib/types";

const KEY = (k: string) => `mf.sort.${k}`;

/**
 * 各列表**实际支持**的排序键——与后端 `list_*_with` 及前端 `sortTracks` 的能力对齐：
 * - `library`  → `list_tracks_with`：未 JOIN play_history/likes，played_at / liked_at 会静默回退 t.path
 * - `favorites`→ `list_liked_with`：未 JOIN play_history，played_at 不可用
 * - `history`  → `list_history_with`：未 JOIN likes，liked_at 不可用
 * - `memory`   → 前端 `sortTracks`：无 played_at / liked_at / play_count 字段
 */
export const SORT_SUPPORTED: Record<
  "library" | "favorites" | "history" | "memory",
  readonly TrackSortField[]
> = {
  library: ["default", "title", "artist", "album", "duration", "play_count"],
  favorites: ["default", "title", "artist", "album", "duration", "play_count"],
  history: ["default", "title", "artist", "album", "duration", "play_count"],
  memory: ["default", "title", "artist", "album", "duration"],
};

/**
 * @returns `[当前排序键, 设置函数, 被回退的失效键]`
 *   - 第三个元素：持久化的排序键不被本列表支持时返回该键，否则 `null`。
 *     调用方**必须**据此显式提示——静默回退会让用户以为自己选的排序生效了（P1-14）。
 */
export function useSort(
  key: string,
  fallback: TrackSortField = "default",
  allowed?: readonly TrackSortField[]
): [TrackSortField, (s: TrackSortField) => void, TrackSortField | null] {
  // 只在挂载时读一次（不响应外部写入，与既有行为一致）
  const [initial] = useState(() => {
    let stored: TrackSortField | null = null;
    try {
      stored = localStorage.getItem(KEY(key)) as TrackSortField | null;
    } catch {
      /* 隐私模式等：忽略 */
    }
    if (stored === null) return { sort: fallback, rejected: null };
    if (allowed && !allowed.includes(stored)) return { sort: fallback, rejected: stored };
    return { sort: stored, rejected: null };
  });
  const [sort, setSort] = useState<TrackSortField>(initial.sort);

  useEffect(() => {
    try {
      localStorage.setItem(KEY(key), sort);
    } catch {
      /* 隐私模式等：忽略 */
    }
  }, [key, sort]);

  return [sort, setSort, initial.rejected];
}
