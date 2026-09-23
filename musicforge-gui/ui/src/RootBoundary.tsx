/**
 * **根错误边界**（单独成文件：main.tsx 里定义组件会触发
 * `react-refresh/only-export-components`，且不利于热更新）。
 *
 * 为什么需要它：App 内的分区 `ErrorBoundary` 只覆盖了 6 处，而 PlayerBar、
 * SearchPalette（lazy —— chunk 加载失败必抛）、WelcomeGuide、左侧栏都在边界
 * 之外。任一处抛错就是**整页白屏**，用户只能重启应用，且无从判断是哪个模块。
 *
 * i18n：class 组件不可用 hook，故在本函数组件内取字典再以 props 传入
 * （与 App.tsx 里分区边界的既有键一致）。
 */
import type { ReactNode } from "react";
import ErrorBoundary from "./ErrorBoundary";
import { useLang } from "./i18n";

export default function RootBoundary({ children }: { children: ReactNode }) {
  const { t } = useLang();
  return (
    <ErrorBoundary title={t.app.errorTitle} hint={t.app.errorHint} retry={t.app.errorRetry}>
      {children}
    </ErrorBoundary>
  );
}
