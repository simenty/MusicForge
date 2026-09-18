// 设置页「更新」区块（P5）：检查 → 下载安装（签名校验）→ 重启生效。
//
// 这是应用的**唯一网络行为**（dependency-policy.md 显式例外）：HTTPS 拉取
// GitHub Releases 的 latest.json；安装包经 minisign 公钥校验，失败即报错，
// 绝不静默降级。网络不可用时不影响任何其他功能。
import { useState } from "react";
import { IS_DESKTOP, checkUpdate, installUpdate, restartApp } from "./api";
import type { UpdateInfo } from "./api";
import { useLang } from "./i18n";
import { IconCheckBox, IconDownload, IconWarnTri } from "./icons";

type Phase = "idle" | "checking" | "latest" | "available" | "installing" | "installed" | "error";

export default function UpdateSection() {
  const { t } = useLang();
  const [phase, setPhase] = useState<Phase>("idle");
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [err, setErr] = useState("");

  if (!IS_DESKTOP) return null;

  const doCheck = async () => {
    setPhase("checking");
    setErr("");
    try {
      const r = await checkUpdate();
      setInfo(r);
      setPhase(r.available ? "available" : "latest");
    } catch (e) {
      setErr(String(e));
      setPhase("error");
    }
  };

  const doInstall = async () => {
    setPhase("installing");
    try {
      await installUpdate();
      setPhase("installed");
    } catch (e) {
      setErr(String(e));
      setPhase("error");
    }
  };

  const busy = phase === "checking" || phase === "installing";

  return (
    <div className="cfg-block">
      <div className="cfg-intro">
        <b>{t.update.title}</b>
        <p>{t.update.desc}</p>
      </div>
      <div className="toolbar">
        <div className="tb-left">
          {(phase === "idle" || phase === "latest" || phase === "error") && (
            <button className="btn" onClick={() => void doCheck()} disabled={busy}>
              {t.update.check}
            </button>
          )}
          {phase === "checking" && <span className="scan-note">{t.update.checking}</span>}
          {phase === "available" && info?.version && (
            <>
              <span className="scan-note">
                <IconDownload /> {t.update.available(info.version)}
              </span>
              <button className="btn primary" onClick={() => void doInstall()}>
                {t.update.install}
              </button>
            </>
          )}
          {phase === "installing" && <span className="scan-note">{t.update.installing}</span>}
          {phase === "installed" && (
            <>
              <span className="scan-note">
                <IconCheckBox /> {t.update.installed}
              </span>
              <button className="btn primary" onClick={() => void restartApp()}>
                {t.update.restart}
              </button>
            </>
          )}
          {phase === "latest" && (
            <span className="scan-note">
              <IconCheckBox /> {t.update.upToDate}
            </span>
          )}
          {phase === "error" && (
            <span className="scan-error">
              <IconWarnTri /> {t.update.failed(err)}
            </span>
          )}
        </div>
      </div>
      <p className="scan-note">{t.update.offlineHint}</p>
    </div>
  );
}
