// 前端曲目排序（P6.21）：用于已一次性取全的详情页列表（专辑/艺术家详情）。
// 虚拟化库/分页列表（曲库/喜欢/历史）走**服务端排序**（`listTracks(..., sort)`），
// 这里只服务「整段已在内存」的场景；`play_count` 无前端字段，详情页不提供。
import type { Track, TrackSortField } from "./types";

/** 返回新数组（不改原数组）；`default` 原样返回（保持后端/碟轨序）。 */
export function sortTracks(tracks: Track[], sort: TrackSortField): Track[] {
  if (sort === "default" || sort === "played_at" || sort === "liked_at" || sort === "play_count") {
    return tracks;
  }
  const cmp = (a: Track, b: Track): number => {
    switch (sort) {
      case "title":
        return (a.title ?? "").localeCompare(b.title ?? "", undefined, { sensitivity: "base" });
      case "artist":
        return (a.artist ?? "").localeCompare(b.artist ?? "", undefined, { sensitivity: "base" });
      case "album":
        return (a.album ?? "").localeCompare(b.album ?? "", undefined, { sensitivity: "base" });
      case "duration":
        return (a.durationMs ?? 0) - (b.durationMs ?? 0);
      default:
        return 0;
    }
  };
  return [...tracks].sort(cmp);
}
