// 播放状态（P2）：500ms 轮询 player_status + 操作封装。
//
// 为什么轮询而非事件：状态负载小（单个 JSON）、对引擎无状态假设
// （引擎重启/不可用时自动恢复）、实现与测试都简单。事件推送留待后续优化。
import { useCallback, useEffect, useRef, useState } from "react";
import {
  IS_DESKTOP,
  playerNext,
  playerPlayQueue,
  playerPrev,
  playerSeek,
  playerSetVolume,
  playerStatus,
  playerStop,
  playerToggle,
} from "../api";
import type { PlayerSnapshot, QueueItem, Track } from "../lib/types";

const POLL_MS = 500;

export interface PlayerApi {
  /** 当前状态（引擎不可用/服务端形态时为 null） */
  status: PlayerSnapshot | null;
  /** 是否正在播放 */
  playing: boolean;
  /** 以 `tracks` 为队列、从 `startIndex` 开始播放 */
  playTracks: (tracks: Track[], startIndex: number) => Promise<void>;
  toggle: () => Promise<void>;
  next: () => Promise<void>;
  prev: () => Promise<void>;
  seek: (ms: number) => Promise<void>;
  /** 音量：本地立即生效 + 120ms 节流下发（拖动不刷后端） */
  setVolume: (v: number) => void;
  stop: () => Promise<void>;
}

export function usePlayer(): PlayerApi {
  const [status, setStatus] = useState<PlayerSnapshot | null>(null);
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

  const playTracks = useCallback(async (tracks: Track[], startIndex: number) => {
    const items: QueueItem[] = tracks.map((t) => ({
      trackId: t.id,
      path: t.path,
      title: t.title,
      artist: t.artist,
      durationMs: t.durationMs,
    }));
    await playerPlayQueue(items, startIndex);
  }, []);

  const toggle = useCallback(async () => {
    await playerToggle();
  }, []);

  const next = useCallback(async () => {
    await playerNext();
  }, []);

  const prev = useCallback(async () => {
    await playerPrev();
  }, []);

  const seek = useCallback(async (ms: number) => {
    await playerSeek(ms);
  }, []);

  const stop = useCallback(async () => {
    await playerStop();
  }, []);

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
    playTracks,
    toggle,
    next,
    prev,
    seek,
    setVolume,
    stop,
  };
}
