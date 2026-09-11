import { defineConfig } from "vitest/config";

/**
 * 前端测试（P1-4 引入，P4-2 扩展）：
 * - 纯逻辑（lib/format）与字典结构（i18n）→ 任何环境可跑；
 * - 组件测试（*.test.tsx，P4-2 引入：面板交互 + 二次确认闸的安全语义）→ 需要 jsdom。
 *
 * 环境统一取 jsdom（两类测试同环境可跑，node 下 jsdom 的额外开销可忽略）；
 * setupFiles 提供 jest-dom 匹配器（toBeDisabled 等）与用例后 DOM 卸载。
 *
 * 独立配置文件（不写 vite.config.ts）——避免影响生产构建配置。
 */
export default defineConfig({
  test: {
    include: ["src/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    reporters: ["default"],
  },
});
