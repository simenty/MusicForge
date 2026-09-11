// P4-2：vitest 全局 setup——jest-dom 匹配器（toBeDisabled/toBeInTheDocument 等）
// + 每个用例后卸载 DOM（testing-library 官方推荐，避免用例间残留）。
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// Node ≥22 的实验性 WebStorage（globalThis.localStorage）会在 jsdom 环境中遮蔽
// jsdom 的实现：未提供 --localstorage-file 时 setItem 为 undefined，导致被测代码
// （i18n.tsx 的 mf.lang 持久化）抛 "localStorage.setItem is not a function"。
// → 仅当全局不可用时注入内存实现；CI 的 Node 20 走 jsdom 原生实现，此分支自动跳过。
if (typeof globalThis.localStorage?.setItem !== "function") {
  const store = new Map<string, string>();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    enumerable: true,
    value: {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, String(v)),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => Array.from(store.keys())[i] ?? null,
      get length() {
        return store.size;
      },
    },
  });
}

afterEach(() => {
  cleanup();
});
