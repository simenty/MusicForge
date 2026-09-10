import { useCallback, useEffect, useState } from "react";
import { previewTemplate } from "../api";
import { loadSettings, saveSettings, type Settings } from "../settings";

/**
 * 设置状态：加载 / 增量更新 / 防抖持久化 / 模板实时预览。
 *
 * 从 App.tsx 原样迁出（行为不变）：
 * - 持久化防抖 300ms（连续输入不会频繁写盘）；
 * - 模板预览防抖 300ms，失败时清空而非抛错（预览是辅助信息）。
 */
export function useSettings() {
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [preview, setPreview] = useState<string[]>([]);

  // 设置持久化（防抖 300ms）
  useEffect(() => {
    const id = setTimeout(() => saveSettings(settings), 300);
    return () => clearTimeout(id);
  }, [settings]);

  // 模板实时预览（debounce 300ms）
  useEffect(() => {
    const id = setTimeout(() => {
      if (settings.template.trim()) {
        previewTemplate(settings.template).then(setPreview).catch(() => setPreview([]));
      } else {
        setPreview([]);
      }
    }, 300);
    return () => clearTimeout(id);
  }, [settings.template]);

  const patch = useCallback((p: Partial<Settings>) => {
    setSettings((s) => ({ ...s, ...p }));
  }, []);

  return { settings, patch, preview };
}
