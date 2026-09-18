// 曲目窗口化加载（P1 曲库体验）：分页取数 + 自建虚拟滚动（与 useBatch 同模式，
// 零新依赖）。
//
// 设计：
// - 后端分页契约 200 行/页（core 侧 limit 硬上限 500）——本 hook 按**可视区间**
//   推导需要的页，缓存缺失时拉取：十万级曲库无需一次性传输/渲染；
// - 行数据按页缓存（Map），页到达时触发重渲染；
// - 竞态：inflight 去重（同页并发只发一次）；越界到达的页照常入缓存
//   （下次滚回立即可见）；拉取失败静默——滚动区间变化会自然重试。
import { useCallback, useEffect, useRef, useState } from "react";
import { listTracks } from "../api";
import type { Track } from "../lib/types";

/** 行高（必须与 styles.css 的 `.vt-row` height 一致） */
export const TRACK_ROW_H = 44;
/** 上下各多取的行数（滚动时预取，减少空白） */
const OVERSCAN = 8;

export interface WindowedTracks {
  /** 曲目总数（库统计驱动；未加载的行显示占位） */
  total: number;
  /** 当前窗口的起始行号 */
  start: number;
  /** 当前需要渲染的行号（连续区间） */
  indices: number[];
  /** 取第 i 行数据（未加载 → undefined，渲染占位） */
  rowAt: (i: number) => Track | undefined;
  /** 已缓存页数（诊断/提示用） */
  loadedPages: number;
  /** 是否有页正在拉取 */
  busy: boolean;
  /** 重置（总数变化/刷新时调用） */
  reset: (total: number) => void;
  /** 滚动事件入口（传容器元素） */
  onScroll: (el: HTMLDivElement) => void;
  /** 已缓存行的有序快照（双击播放构造队列用；未加载的行跳过） */
  snapshot: () => Track[];
}

export function useWindowedTracks(pageSize = 200): WindowedTracks {
  const [total, setTotal] = useState(0);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportH, setViewportH] = useState(560);
  const [, setTick] = useState(0);
  const pages = useRef(new Map<number, Track[]>());
  const inflight = useRef(new Set<number>());

  const reset = useCallback((n: number) => {
    pages.current.clear();
    inflight.current.clear();
    setTotal(n);
    setScrollTop(0);
    setTick((v) => v + 1);
  }, []);

  const start = Math.max(0, Math.floor(scrollTop / TRACK_ROW_H) - OVERSCAN);
  const end =
    total > 0
      ? Math.min(total - 1, Math.ceil((scrollTop + viewportH) / TRACK_ROW_H) + OVERSCAN)
      : -1;

  useEffect(() => {
    if (total <= 0 || end < start) return;
    const p0 = Math.floor(start / pageSize);
    const p1 = Math.floor(end / pageSize);
    for (let p = p0; p <= p1; p++) {
      if (pages.current.has(p) || inflight.current.has(p)) continue;
      inflight.current.add(p);
      listTracks(pageSize, p * pageSize)
        .then((rows) => {
          pages.current.set(p, rows);
        })
        .catch(() => {
          /* 静默：区间变化后自然重试 */
        })
        .finally(() => {
          inflight.current.delete(p);
          setTick((v) => v + 1);
        });
    }
  }, [start, end, total, pageSize]);

  const rowAt = useCallback(
    (i: number): Track | undefined => pages.current.get(Math.floor(i / pageSize))?.[i % pageSize],
    [pageSize]
  );

  const indices: number[] = [];
  for (let i = start; i <= end; i++) indices.push(i);

  const onScroll = useCallback((el: HTMLDivElement) => {
    setScrollTop(el.scrollTop);
    if (el.clientHeight > 0) {
      setViewportH((h) => (h === el.clientHeight ? h : el.clientHeight));
    }
  }, []);

  const snapshot = useCallback((): Track[] => {
    const out: Track[] = [];
    for (let i = 0; i < total; i++) {
      const r = rowAt(i);
      if (r) out.push(r);
    }
    return out;
  }, [total, rowAt]);

  return {
    total,
    start,
    indices,
    rowAt,
    loadedPages: pages.current.size,
    busy: inflight.current.size > 0,
    reset,
    onScroll,
    snapshot,
  };
}
