/**
 * 服务端信息卡（P2 收尾）：闭合最后两个「后端已实现但前端未接」的端点。
 *
 * - `/api/version`：版本与 API 面标识（升级排查时一眼可见服务端版本）；
 * - `/api/wizard/status`：首启自检（token 就绪 / 数据目录可写 / 关键路径）——
 *   这正是 fnOS 上「启用失败」时最需要的两项自检。
 *
 * 2026-09-11 体验修复：`MF-AUTH-REQUIRED`（缺失/错误 X-Token）不再显示为一行裸文本，
 * 而是升级为**可操作引导卡**——给出两条取 token 的命令 + 填写位置（R22 无默认口令，
 * 用户拿 token 的唯一门槛就在这一步）。
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
  const [errCode, setErrCode] = useState("");
  /** 重新检测计数（用户在顶栏填入 token 后点「重新检测」） */
  const [reload, setReload] = useState(0);

  useEffect(() => {
    if (!IS_SERVER_MODE) return;
    let cancelled = false;
    void (async () => {
      try {
        const [v, w] = await Promise.all([serverVersion(), wizardStatus()]);
        if (!cancelled) {
          setVer(v);
          setWiz(w);
          setError(null);
          setErrCode("");
        }
      } catch (e) {
        if (!cancelled) {
          setErrCode((e as { code?: string })?.code ?? "");
          setError(e instanceof Error ? e.message : String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [reload]);

  if (!IS_SERVER_MODE) return null;

  /** token 未配置/错误（R22）→ 取 token 引导卡 */
  const isTokenError = errCode === "MF-AUTH-REQUIRED" || /X-Token/i.test(error ?? "");
  /** 客户端过旧（M2：缺签名头）→ 强刷引导（升级 fpk 后浏览器缓存旧 SPA 的典型症状） */
  const isStaleClient = errCode === "MF-AUTH-SIG-MISSING";

  return (
    <div className="panel">
      <div className="panel-head">
        <h2>{t.serverInfo.head}</h2>
      </div>

      {isStaleClient ? (
        <div className="state-hero" role="alert" style={{ padding: "20px 16px" }}>
          <span className="state-ico bad" aria-hidden="true">
            <svg
              width="22"
              height="22"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M20 11a8 8 0 10-2.3 5.7" />
              <path d="M20 4v7h-7" />
            </svg>
          </span>
          <h2 style={{ fontSize: "var(--fs-md)" }}>{t.auth.staleTitle}</h2>
          <p>{t.auth.staleBody}</p>
          <div className="state-actions">
            <button className="btn sm primary" onClick={() => window.location.reload()}>
              {t.auth.staleReload}
            </button>
            <button className="btn sm" onClick={() => setReload((n) => n + 1)}>
              {t.auth.retry}
            </button>
          </div>
          <p className="hint">{t.auth.staleHint}</p>
        </div>
      ) : isTokenError ? (
        <div className="state-hero" role="alert" style={{ padding: "20px 16px" }}>
          <span className="state-ico bad" aria-hidden="true">
            <svg
              width="22"
              height="22"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <circle cx="8" cy="15" r="4" />
              <path d="M10.8 12.2L20 3M16 6l2.5 2.5M13 9l2 2" />
            </svg>
          </span>
          <h2 style={{ fontSize: "var(--fs-md)" }}>{t.auth.title}</h2>
          <p>{t.auth.body}</p>
          <div className="auth-cmds">
            <div>
              <div className="cfg-label">{t.auth.cmdFileLabel}</div>
              <code>{t.auth.cmdFile}</code>
            </div>
            <div>
              <div className="cfg-label">{t.auth.cmdLogLabel}</div>
              <code>{t.auth.cmdLog}</code>
            </div>
          </div>
          <p className="hint">{t.auth.where}</p>
          <p className="hint">{t.auth.hint}</p>
          <div className="state-actions">
            <button className="btn sm" onClick={() => setReload((n) => n + 1)}>
              {t.auth.retry}
            </button>
          </div>
        </div>
      ) : (
        <>
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
        </>
      )}
    </div>
  );
}
