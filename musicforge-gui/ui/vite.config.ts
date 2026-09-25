import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 前端：固定端口 + 禁用浏览器自动打开（由 Tauri 窗口承载）
// 测试配置独立在 vitest.config.ts（保持本文件只关心生产构建）。
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    // P6.13：产出 manifest——体积护栏据此区分「首屏静态链」与「懒加载 chunk」
    // （只看总体积会掩盖首屏变胖；只看入口又会漏掉懒加载块的膨胀）。
    manifest: true,
    // P2-22：首屏分块可控——react / tauri / i18n / icons 各自独立 chunk，
    // 避免首屏被单一大 bundle 拖胖；其余模块走默认分块（返回 undefined）。
    rollupOptions: {
      output: {
        manualChunks(id: string) {
          if (
            id.includes("node_modules/react") ||
            id.includes("node_modules/react-dom") ||
            id.includes("node_modules/scheduler")
          )
            return "react-vendor";
          if (id.includes("@tauri-apps")) return "tauri-vendor";
          if (id.includes("/src/i18n")) return "i18n";
          if (id.includes("/src/lib/icons")) return "icons";
          return undefined;
        },
      },
    },
  },
});
