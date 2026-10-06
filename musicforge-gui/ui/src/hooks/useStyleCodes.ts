// 列表内风格码（X15）：批量解析 + 逐路径缓存。
//
// 目的：虚拟列表有 N 行，若每行各调一次 IPC 会把滚动拖垮。故用**批量**命令
// `style_codes` 把 N 次压成 1 次，并缓存「路径 → 短标签」——已解析过的路径不再重复请求
// （翻页/筛选切换时只补新出现的路径）。
//
// 缓存值 `null` = 该路径**无前导码块**（绝大多数曲目）；不进返回集，故调用方
// 用 `chips[path]` 取不到即表示"无码"，无需区分 undefined/null。
import { useEffect, useMemo, useRef, useState } from "react";
import { styleCodes } from "../api";
import type { StyleCodeDto, Track } from "../lib/types";
import { useRequestGuard } from "./useRequestGuard";

/** 路径 → 短标签（如 `2023 · 流行 · 学习`）；无码的曲目不在表中。 */
export type StyleCodeChips = Record<string, string>;

/**
 * chip 只取**年份 + 风格 + 场景**：专辑列很窄，情绪/版本/其他挤进去会淹没曲库信息，
 * 完整字段由「风格码卡片」承担。码名查不到时回退原始码（绝不编造）。
 */
export function shortLabel(d: StyleCodeDto): string {
  const name = (c: string) => d.labels[c] ?? c;
  const parts: string[] = [];
  if (d.year !== null) parts.push(String(d.year));
  if (d.style) parts.push(name(d.style));
  for (const s of d.scenes) parts.push(name(s));
  return parts.join(" · ");
}

/**
 * 批量解析曲目路径的风格码，返回「路径 → 短标签」。
 *
 * - `paths` 请用 `useMemo` 稳定引用（否则每次渲染都会重算待补集合）；
 * - 请求失败（如 codebook 路径非法）时**标记为空并停止重试**，避免每次渲染重发刷屏；
 *   此时列表不显示 chip，卡片入口仍会显式报错（那里才有路径问题的上下文）。
 */
export function useStyleCodes(paths: string[], codebookPath?: string): StyleCodeChips {
  /** 路径 → 短标签；`null` = 无码（含"取过但失败"）。 */
  const cache = useRef<Map<string, string | null>>(new Map());
  const [chips, setChips] = useState<StyleCodeChips>({});
  const guard = useRequestGuard();

  useEffect(() => {
    const missing = paths.filter((p) => !cache.current.has(p));
    if (missing.length === 0) return;
    const token = guard.token();
    void styleCodes(missing, codebookPath)
      .then((res) => {
        if (guard.isStale(token)) return;
        const added: StyleCodeChips = {};
        for (const p of missing) {
          const dto: StyleCodeDto | undefined = res[p];
          const v = dto ? shortLabel(dto) : null;
          cache.current.set(p, v);
          // 空串（有码块但 chip 三要素都为空）也按"无 chip"处理
          if (v) added[p] = v;
        }
        if (Object.keys(added).length > 0) setChips((prev) => ({ ...prev, ...added }));
      })
      .catch(() => {
        if (guard.isStale(token)) return;
        for (const p of missing) cache.current.set(p, null);
      });
  }, [paths, codebookPath, guard]);

  return chips;
}

/**
 * `useStyleCodes` 的便捷包装：直接吃 `Track[]`，内部把路径数组 memo 稳定
 * （调用方不必各自引 `useMemo`，也避免忘记 memo 导致每次渲染 O(N) 重算待补集合）。
 */
export function useTrackStyleChips(
  tracks: Track[],
  codebookPath?: string,
): StyleCodeChips {
  const paths = useMemo(() => tracks.map((t) => t.path), [tracks]);
  return useStyleCodes(paths, codebookPath);
}
