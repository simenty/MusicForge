/**
 * 服务端信息卡（P2 收尾）：闭合最后两个「后端已实现但前端未接」的端点。
 *
 * - `/api/version`：版本与 API 面标识（升级排查时一眼可见服务端版本）；
 * - `/api/wizard/status`：首启自检（token 就绪 / 数据目录可写 / 关键路径）——
 *   这正是 fnOS 上「启用失败」时最需要的两项自检。
 *
 * 仅服务端形态显示（桌面直通本地文件系统，无此概念）。
 */
import { useEffect, useState } from "react";
import {
  IS_SERVER_MODE,
  serverVersion,
  wizardStatus,
  type ServerVersion,
  type WizardStatus,
} from "./api";
import { useLang } from "./i18n";

export default function ServerInfoCard() {
  const { t } = useLang();
  const [ver, setVer] = useState<ServerVersion | null>(null);
  const [wiz, setWiz] = useState<WizardStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!IS_SERVER_MODE) return;
    let cancelled = false;
    void (async () => {
      try {
        const [v, w] = await Promise.all([serverVersion(), wizardStatus()]);
        if (!cancelled) {
          setVer(v);
          setWiz(w);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  if (!IS_SERVER_MODE) return null;

  return (
    <div className="panel">
      <div className="panel-head">
        <h2>{t.serverInfo.head}</h2>
      </div>

      {error && <div className="scan-error">✕ {error}</div>}

      {ver && (
        <div className="scan-summary">
          <span>{t.serverInfo.version(ver.version)}</span>
          <span className="plugin-note">{t.serverInfo.apiSurface(ver.api_surface)}</span>
        </div>
      )}

      {wiz && (
        <div className="scan-summary">
          <span className={wiz.token_ready ? "s-ok" : "s-failed"}>
            {wiz.token_ready ? "✓" : "✕"} {t.serverInfo.tokenReady}
          </span>
          <span className={wiz.data_dir_writable ? "s-ok" : "s-failed"}>
            {wiz.data_dir_writable ? "✓" : "✕"} {t.serverInfo.dataDirWritable}
          </span>
          <span className="plugin-note mono">{wiz.data_dir}</span>
          {wiz.library_dir && <span className="plugin-note mono">{wiz.library_dir}</span>}
        </div>
      )}

      <div className="scan-note">{t.serverInfo.hint}</div>
    </div>
  );
}
