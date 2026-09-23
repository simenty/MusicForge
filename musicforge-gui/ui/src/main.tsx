import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// 必须显式引入：Vite 只打包被 import 的 CSS。
// （2026-09-02 修复：此前 styles.css 从未被引入，dist 里根本没有 CSS 产物，
//   应用一直是「无样式」运行的，而「进程活着」类验证查不出来。）
import "./styles.css";
import { I18nProvider } from "./i18n";
// 根错误边界：兜住 App 内所有分区边界之外的渲染异常（否则整页白屏）
import RootBoundary from "./RootBoundary";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <I18nProvider>
      <RootBoundary>
        <App />
      </RootBoundary>
    </I18nProvider>
  </React.StrictMode>
);
