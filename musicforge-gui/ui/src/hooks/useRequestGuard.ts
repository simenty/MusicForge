/**
 * 异步结果守卫（稳定审计修复：stale response 收敛）。
 *
 * **问题**：Tauri IPC 不保证响应顺序，且组件可能在请求飞行期间被卸载。
 * 此前全前端 ~12 处写法都是「await 完无条件 setState」——晚到的旧响应会
 * 覆盖先到的新响应（换排序/筛选、连点详情页时必现），或对已卸载组件
 * setState（React 警告 + 潜在泄漏）。
 *
 * **用法**（三段式）：
 * ```ts
 * const g = useRequestGuard();
 * // ① 参数变化 → 作废旧代（通常在清缓存的 effect 里）
 * g.bump();
 * // ② 发起请求前取令牌
 * const token = g.token();
 * someApi().then((v) => {
 *   // ③ 落库前校验：仍是最新代际且组件仍挂载
 *   if (g.isStale(token)) return;
 *   setValue(v);
 * });
 * ```
 */
import { useEffect, useMemo, useRef } from "react";

export interface RequestGuard {
  /** 当前代际令牌（发起请求前取，不推进代际）。 */
  token: () => number;
  /** 作废旧代（查询参数/排序/筛选变化时调用）。 */
  bump: () => void;
  /** 该令牌的结果是否应丢弃（已被更新的代际取代，或组件已卸载）。 */
  isStale: (token: number) => boolean;
  /** 组件是否仍挂载（供 finally 里判断能否 setState）。 */
  isMounted: () => boolean;
}

export function useRequestGuard(): RequestGuard {
  const seq = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    // StrictMode 下 effect 会二次执行（首次执行 cleanup 把 mounted 置 false），
    // 故进入时必须重新置 true。
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  return useMemo(
    () => ({
      token: () => seq.current,
      bump: () => {
        seq.current += 1;
      },
      isStale: (t: number) => !mounted.current || t !== seq.current,
      isMounted: () => mounted.current,
    }),
    []
  );
}
