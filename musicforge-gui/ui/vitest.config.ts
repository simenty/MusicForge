import { defineConfig } from "vitest/config";

/**
 * 前端测试（P1-4）：只跑**纯逻辑**（lib/format）与**字典结构**（i18n），
 * 因此需要 jsdom 的组件测试尚未引入——环境用 node，保持依赖面最小。
 *
 * 独立配置文件（不写 vite.config.ts）——避免影响生产构建配置。
 */
export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
    reporters: ["default"],
  },
});
