import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 前端：固定端口 + 禁用浏览器自动打开（由 Tauri 窗口承载）
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
  },
});
