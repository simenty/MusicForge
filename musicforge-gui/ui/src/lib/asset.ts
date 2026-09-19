// 本地文件 → webview 可加载的 asset URL（Tauri v2 asset 协议）。
// 非桌面形态 / 转换异常 → null（调用方回退占位图）。
import { convertFileSrc } from "@tauri-apps/api/core";
import { IS_DESKTOP } from "../api";

export function assetUrl(p: string | null | undefined): string | null {
  if (!IS_DESKTOP || !p) return null;
  try {
    return convertFileSrc(p);
  } catch {
    return null;
  }
}
