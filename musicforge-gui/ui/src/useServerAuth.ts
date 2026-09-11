// 鉴权总开关状态探测（2026-09-12 产品决策）：
// `MUSICFORGE_AUTH=off` 时后端完全跳过 token 校验——界面应据此
// **隐藏 token 输入框**并显示「鉴权已关闭」徽标（而非让用户对着无用的输入框困惑）。
//
// 兼容性：旧版后端不带 `auth_enabled` 字段 → 返回 null（保守：保持 token 框可见）。
import { useEffect, useState } from "react";
import { IS_SERVER_MODE, wizardStatus } from "./api";

export function useServerAuth(): { authEnabled: boolean | null } {
  const [authEnabled, setAuthEnabled] = useState<boolean | null>(null);

  useEffect(() => {
    if (!IS_SERVER_MODE) return;
    let cancelled = false;
    void (async () => {
      try {
        const w = await wizardStatus();
        if (!cancelled) {
          setAuthEnabled(typeof w.auth_enabled === "boolean" ? w.auth_enabled : null);
        }
      } catch {
        // 请求失败（后端不可达 / 旧版本）→ 保持 null：不隐藏 token 框（保守策略）
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  return { authEnabled };
}
