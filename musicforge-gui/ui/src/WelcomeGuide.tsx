// 首启引导（P5）：首次运行显示三步说明（选媒体源 → 扫描 → 播放）。
// 标记存 localStorage——纯前端状态，无需后端参与；写失败（隐私模式等）
// 只影响下次是否再显示，绝不影响功能。
import { useState } from "react";
import { useLang } from "./i18n";

export default function WelcomeGuide({ onGo }: { onGo: () => void }) {
  const { t } = useLang();
  const [show, setShow] = useState(() => {
    try {
      return localStorage.getItem("mf.onboarded") !== "1";
    } catch {
      return false;
    }
  });

  if (!show) return null;

  const close = (go: boolean) => {
    try {
      localStorage.setItem("mf.onboarded", "1");
    } catch {
      /* 隐私模式等：只影响下次是否再显示 */
    }
    setShow(false);
    if (go) onGo();
  };

  return (
    <div className="modal-mask" role="presentation" onClick={() => close(false)}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="welcome-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3 id="welcome-title">{t.welcome.title}</h3>
        </div>
        <div className="modal-body">
          <div className="modal-summary">{t.welcome.intro}</div>
          <ul className="modal-list">
            <li>{t.welcome.step1}</li>
            <li>{t.welcome.step2}</li>
            <li>{t.welcome.step3}</li>
          </ul>
          <div className="modal-note">{t.welcome.privacy}</div>
        </div>
        <div className="modal-foot">
          <button className="btn sm" onClick={() => close(false)}>
            {t.welcome.later}
          </button>
          <button className="btn sm primary" onClick={() => close(true)}>
            {t.welcome.start}
          </button>
        </div>
      </div>
    </div>
  );
}
