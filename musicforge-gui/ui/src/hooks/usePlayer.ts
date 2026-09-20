// 播放状态（P2）：500ms 轮询 player_status + 操作封装。
//
// 为什么轮询而非事件：状态负载小（单个 JSON）、对引擎无状态假设
// （引擎重启/不可用时自动恢复）、实现与测试都简单。事件推送留待后续优化。
import { useCallback, useEffect, useRef, useState } from "react";
import {
  IS_DESKTOP,
  playerJump,
  playerNext,
  playerPlayQueue,
  playerPrev,
  playerSeek,
  playerSetVolume,
  playerStatus,
  playerStop,
  playerToggle,
} from "../api";
import { loadSession, saveSession, type SavedSession } from "../lib/session";
import type { PlayerSnapshot, QueueItem, Track } from "../lib/types";

const POLL_MS = 500;

export interface PlayerApi {
  /** 当前状态（引擎不可用/服务端形态时为 null） */
  status: PlayerSnapshot | null;
  /** 是否正在播放 */
  playing: boolean;
  /** 当前队列（前端持有的副本；引擎侧队列经 playTracks 一次性提交） */
  queue: QueueItem[];
  /** 上次会话（P6.12）：非 null = 引擎空闲但存在可续播的上次队列 */
  restored: SavedSession | null;
  /** 续播上次会话（重建队列 + 回到保存位置；绝不自动出声） */
  resume: () => Promise<void>;
  /** 以 `tracks` 为队列、从 `startIndex` 开始播放 */
  playTracks: (tracks: Track[], startIndex: number) => Promise<void>;
  toggle: () => Promise<void>;
  /** 暂停（仅在播放中生效——睡眠定时等场景专用，绝不反向唤醒） */
  pause: () => Promise<void>;
  next: () => Promise<void>;
  prev: () => Promise<void>;
  /** 跳到队列中的指定位置（队列抽屉点选） */
  jump: (index: number) => Promise<void>;
  seek: (ms: number) => Promise<void>;
  /** 音量：本地立即生效 + 120ms 节流下发（拖动不刷后端） */
  setVolume: (v: number) => void;
  stop: () => Promise<void>;
}

export function usePlayer(): PlayerApi {
  const [status, setStatus] = useState<PlayerSnapshot | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const alive = useRef(true);
  const volTimer = useRef<number | null>(null);

  useEffect(() => {
    if (!IS_DESKTOP) return;
    alive.current = true;
    let timer: number | null = null;
    const tick = async () => {
      try {
        const s = await playerStatus();
        if (alive.current) setStatus(s);
      } catch {
        /* 引擎暂不可用：静默，下轮重试 */
      }
      if (alive.current) timer = window.setTimeout(() => void tick(), POLL_MS);
    };
    void tick();
    return () => {
      alive.current = false;
      if (timer !== null) window.clearTimeout(timer);
      if (volTimer.current !== null) window.clearTimeout(volTimer.current);
    };
  }, []);

  // ---- P6.12 会话持久化：关掉应用再打开，队列与进度还在 ----

  const [restored, setRestored] = useState<SavedSession | null>(null);

  // 启动恢复：读一次上次会话（只恢复队列与展示，不自动播放——出声必须是用户动作）
  useEffect(() => {
    if (!IS_DESKTOP) return;
    const s = loadSession();
    if (s && s.items.length > 0) {
      setQueue(s.items);
      setRestored(s);
    }
  }, []);

  const statusRef = useRef<PlayerSnapshot | null>(null);
  statusRef.current = status;
  const lastSaveRef = useRef(0);

  // 队列变化立即存；`restored` 未消费时跳过（否则会把恢复出来的进度覆盖成 0）
  useEffect(() => {
    if (!IS_DESKTOP || queue.length === 0 || restored !== null) return;
    const st = statusRef.current;
    const pos = st?.positionMs ?? 0;
    saveSession({
      items: queue,
      index: st?.queueIndex ?? 0,
      positionMs: pos,
      ts: Date.now(),
    });
    lastSaveRef.current = Date.now();
  }, [queue, restored]);

  // 播放位置节流保存（轮询 500ms 太密；5s 一次足够新）
  useEffect(() => {
    if (!IS_DESKTOP || !status || queue.length === 0 || restored !== null) return;
    const now = Date.now();
    if (now - lastSaveRef.current < 5_000) return;
    lastSaveRef.current = now;
    saveSession({
      items: queue,
      index: status.queueIndex ?? 0,
      positionMs: status.positionMs,
      ts: now,
    });
  }, [queue, status, restored]);

  const playTracks = useCallback(async (tracks: Track[], startIndex: number) => {
    const items: QueueItem[] = tracks.map((t) => ({
      trackId: t.id,
      path: t.path,
      title: t.title,
      artist: t.artist,
      durationMs: t.durationMs,
    }));
    setRestored(null); // 用户开始新播放：上次会话作废
    setQueue(items);
    await playerPlayQueue(items, startIndex);
  }, []);


  const toggle = useCallback(async () => {
    await playerToggle();
  }, []);

  // playing 的 ref 镜像：pause 需要读「当前」播放态但不应因 status 重建回调
  const playingRef = useRef(false);
  playingRef.current = status?.state === "playing";

  const pause = useCallback(async () => {
    if (playingRef.current) await playerToggle();
  }, []);

  const next = useCallback(async () => {
    await playerNext();
  }, []);

  const prev = useCallback(async () => {
    await playerPrev();
  }, []);

  const jump = useCallback(async (index: number) => {
    await playerJump(index);
  }, []);

  const seek = useCallback(async (ms: number) => {
    await playerSeek(ms);
  }, []);

  const stop = useCallback(async () => {
    await playerStop();
  }, []);

  /** 续播上次会话（P6.12）：重建引擎队列并回到保存位置；位置太靠前则不 seek。 */
  const resume = useCallback(async () => {
    const s = restored;
    if (!s || s.items.length === 0) return;
    setRestored(null);
    setQueue(s.items);
    await playerPlayQueue(s.items, s.index);
    if (s.positionMs > 1_500) await playerSeek(s.positionMs);
  }, [restored]);

  const setVolume = useCallback((v: number) => {
    const clamped = Math.min(1, Math.max(0, v));
    // 本地乐观更新（拖动即时反馈）；下发节流
    setStatus((s) => (s ? { ...s, volume: clamped } : s));
    if (volTimer.current !== null) window.clearTimeout(volTimer.current);
    volTimer.current = window.setTimeout(() => {
      void playerSetVolume(clamped).catch(() => {});
    }, 120);
  }, []);

  return {
    status,
    playing: status?.state === "playing",
    queue,
    restored,
    resume,
    playTracks,
    toggle,
    pause,
    next,
    prev,
    jump,
    seek,
    setVolume,
    stop,
  };
}
