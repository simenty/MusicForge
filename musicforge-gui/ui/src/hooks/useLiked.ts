// 喜欢状态（P2）：一次拉取全部 liked id + 乐观切换。
//
// 为什么一次拉全量 id：likes 是用户显式行为（量级远小于曲库），
// 一次 IPC 换取每行的 O(1) 判定——避免「逐行查询 liked 状态」的 N+1。
import { useCallback, useEffect, useRef, useState } from "react";
import { IS_DESKTOP, likedIds, trackToggleLike, unlikeTracks } from "../api";

export interface LikedApi {
  isLiked: (id: number) => boolean;
  toggle: (id: number) => Promise<void>;
  /** P6.23：批量取消喜欢（一次 IPC）；成功后乐观更新本地集合，收藏页即隐藏对应行 */
  unlikeMany: (ids: number[]) => Promise<void>;
  count: number;
  /** 初始 liked 集合是否已拉取（收藏页过滤依赖它——未加载时不能按空集过滤） */
  loaded: boolean;
}

export function useLiked(): LikedApi {
  const [ids, setIds] = useState<Set<number>>(() => new Set());
  const [loaded, setLoaded] = useState(false);
  // ref 镜像：toggle 需要读「当前」状态，但不应因 ids 变化重建回调
  const idsRef = useRef(ids);
  idsRef.current = ids;

  useEffect(() => {
    if (!IS_DESKTOP) {
      setLoaded(true);
      return;
    }
    likedIds()
      .then((l) => setIds(new Set(l)))
      .catch(() => {
        /* 拉取失败：行状态显示为未喜欢，切换时仍会写库 */
      })
      .finally(() => setLoaded(true));
  }, []);

  const toggle = useCallback(async (id: number) => {
    const wasLiked = idsRef.current.has(id);
    const setLiked = (liked: boolean) =>
      setIds((prev) => {
        const next = new Set(prev);
        if (liked) next.add(id);
        else next.delete(id);
        return next;
      });

    setLiked(!wasLiked); // 乐观更新（拖动/连续点击无延迟感）
    try {
      const r = await trackToggleLike(id);
      setLiked(r.liked); // 以服务端为准校正
    } catch {
      setLiked(wasLiked); // 失败回滚
    }
  }, []);

  const isLiked = useCallback((id: number) => ids.has(id), [ids]);

  const unlikeMany = useCallback(async (idsToRemove: number[]) => {
    if (idsToRemove.length === 0) return;
    try {
      await unlikeTracks(idsToRemove);
      setIds((prev) => {
        const next = new Set(prev);
        for (const id of idsToRemove) next.delete(id);
        return next;
      });
    } catch {
      /* 失败：保留本地集合原状（下次操作可重试） */
    }
  }, []);

  return { isLiked, toggle, unlikeMany, count: ids.size, loaded };
}
