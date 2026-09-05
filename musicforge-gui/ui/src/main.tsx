import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// 必须显式引入：Vite 只打包被 import 的 CSS。
// （2026-09-02 修复：此前 styles.css 从未被引入，dist 里根本没有 CSS 产物，
//   应用一直是「无样式」运行的，而「进程活着」类验证查不出来。）
import "./styles.css";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
