// P4-2：前端静态检查（此前仅 `tsc --noEmit`——类型能过，但 hooks 依赖遗漏、
// 未用变量、floating promise 等正确性问题不被拦）。
//
// 规则取向：只拦**正确性**问题，不做格式规定（项目无 prettier，避免与手写风格冲突产生噪音）。
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";

export default tseslint.config(
  { ignores: ["dist", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // 未用变量：允许下划线前缀（约定：故意忽略）
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  }
);
