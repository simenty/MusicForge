// 排序偏好持久化（P6.21）：按页面 key 记忆上次选择的排序键，刷新后保持。
import { useEffect, useState } from "react";
import type { TrackSortField } from "../lib/types";

const KEY = (k: string) => `mf.sort.${k}`;

/** 返回 [当前排序键, 设置函数]；偏好存于 localStorage，读不到时回退 fallback。 */
export function useSort(
  key: string,
  fallback: TrackSortField = "default"
): [TrackSortField, (s: TrackSortField) => void] {
  const [sort, setSort] = useState<TrackSortField>(() => {
    try {
      const v = localStorage.getItem(KEY(key)) as TrackSortField | null;
      return v ?? fallback;
    } catch {
      return fallback;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem(KEY(key), sort);
    } catch {
      /* 隐私模式等：忽略 */
    }
  }, [key, sort]);
  return [sort, setSort];
}
