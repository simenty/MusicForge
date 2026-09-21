// 多选状态（P6.19）：按曲目 id 维护选中集合 + 选择模式开关。
// 选择模式开启时，曲目行显示勾选框；批量操作（加入队列 / 下一首播放）复用既有 player 能力。
import { useCallback, useState } from "react";

export interface SelectionApi {
  /** 是否处于选择模式（开启后行内显示勾选框） */
  selMode: boolean;
  /** 切换选择模式：关闭时清空已选 */
  toggleSelMode: () => void;
  /** 选中集合（曲目 id） */
  sel: Set<string>;
  /** 某 id 是否被选中 */
  has: (id: string) => boolean;
  /** 切换某 id 的选中态 */
  toggle: (id: string) => void;
  /** 全选给定 id 列表 */
  selectAll: (ids: string[]) => void;
  /** 清空选中（保留选择模式） */
  clear: () => void;
  /** 已选数量 */
  count: number;
}

export function useSelection(): SelectionApi {
  const [selMode, setSelMode] = useState(false);
  const [sel, setSel] = useState<Set<string>>(new Set());

  const toggleSelMode = useCallback(() => {
    setSelMode((prev) => {
      if (prev) setSel(new Set());
      return !prev;
    });
  }, []);

  const has = useCallback((id: string) => sel.has(id), [sel]);

  const toggle = useCallback((id: string) => {
    setSel((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const selectAll = useCallback((ids: string[]) => {
    setSel(new Set(ids));
  }, []);

  const clear = useCallback(() => {
    setSel(new Set());
  }, []);

  return { selMode, toggleSelMode, sel, has, toggle, selectAll, clear, count: sel.size };
}
