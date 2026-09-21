// 前端列表筛选（P6.26 曲目 / P6.27 专辑·艺术家）：均用于**已一次性取全**的列表。
//
// 为什么这里可以纯前端过滤：详情页的曲目由 `albumTracks` / `artistTracks`
// 一次性取全（量级是一张专辑 / 一位艺术家，而非十万级曲库），整段已在内存中，
// 过滤不需要往返。曲库/喜欢/历史则相反——它们分批轮子分页，过滤必须下推到
// SQL（见 db.rs `track_filter_pred`），否则只对同一句话沃的一部分数据过滤，结果集是错的。
//
// 匹配字段与**服务端保持一致**（标题 / 艺术家 / 专辑 / 路径），
// 避免「同一个关键词在曲库页和详情页命中范围不同」这种不一致。
import type { Album, Artist, Track } from "./types";

/** 归一化的包含匹配（`String.prototype.includes` 语义，大小写不敏感）。 */
function hits(fields: (string | number | null | undefined)[], q: string): boolean {
  return fields.some((f) => f != null && String(f).toLowerCase().includes(q));
}

/** 返回新数组（不改原数组）；空/空白查询原样返回。大小写不敏感。 */
export function filterTracks(tracks: Track[], query: string): Track[] {
  const q = query.trim().toLowerCase();
  if (!q) return tracks;
  return tracks.filter((t) =>
    hits([t.title, t.artist, t.album, t.path], q)
  );
}

/** 专辑筛选：匹配 **专辑名 / 艺术家 / 年份**（服务端无对应 SQL，此为唯一的过滤面）。 */
export function filterAlbums(albums: Album[], query: string): Album[] {
  const q = query.trim().toLowerCase();
  if (!q) return albums;
  return albums.filter((a) => hits([a.title, a.artist, a.year], q));
}

/** 艺术家筛选：匹配 **艺术家名**。 */
export function filterArtists(artists: Artist[], query: string): Artist[] {
  const q = query.trim().toLowerCase();
  if (!q) return artists;
  return artists.filter((ar) => hits([ar.name], q));
}
