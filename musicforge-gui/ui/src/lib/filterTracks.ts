// 前端曲目筛选（P6.26）：用于已一次性取全的详情页列表（专辑/艺术家详情）。
//
// 为什么这里可以纯前端过滤：详情页的曲目由 `albumTracks` / `artistTracks`
// 一次性取全（量级是一张专辑 / 一位艺术家，而非十万级曲库），整段已在内存中，
// 过滤不需要往返。曲库/喜欢/历史则相反——它们分批轮子分页，过滤必须下推到
// SQL（见 db.rs `track_filter_pred`），否则只对同一句话沃的一部分数据过滤，结果集是错的。
//
// 匹配字段与**服务端保持一致**（标题 / 艺术家 / 专辑 / 路径），
// 避免「同一个关键词在曲库页和详情页命中范围不同」这种不一致。
import type { Track } from "./types";

/** 返回新数组（不改原数组）；空/空白查询原样返回。大小写不敏感。 */
export function filterTracks(tracks: Track[], query: string): Track[] {
  const q = query.trim().toLowerCase();
  if (!q) return tracks;
  return tracks.filter((t) =>
    (t.title ?? "").toLowerCase().includes(q) ||
    (t.artist ?? "").toLowerCase().includes(q) ||
    (t.album ?? "").toLowerCase().includes(q) ||
    (t.path ?? "").toLowerCase().includes(q)
  );
}
