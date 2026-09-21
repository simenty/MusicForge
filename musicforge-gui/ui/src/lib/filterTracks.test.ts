// filterTracks 测试（P6.26）：详情页客户端筛选的匹配语义。
import { describe, expect, it } from "vitest";
import { filterAlbums, filterArtists, filterTracks } from "./filterTracks";
import type { Album, Artist, Track } from "./types";

function mk(id: number, title: string, artist: string, album: string, path: string): Track {
  return {
    id,
    sourceId: 1,
    path,
    size: 1024,
    title,
    artist,
    album,
    trackNo: id,
    durationMs: 1000,
    format: "flac",
    sampleRate: 44100,
    bitDepth: 16,
    channels: 2,
    isLossless: true,
  };
}

describe("filterTracks (P6.26)", () => {
  const rows = [
    mk(1, "夜空中最亮的星", "逃跑计划", "世界", "/m/a.flac"),
    mk(2, "晴天", "周杰伦", "叶惠美", "/m/b.flac"),
    mk(3, "晴天(Live)", "周杰伦", "演唱会", "/m/live/c.mp3"),
  ];

  it("空查询 / 纯空白原样返回（同一引用语义：内容一致）", () => {
    expect(filterTracks(rows, "")).toEqual(rows);
    expect(filterTracks(rows, "   ")).toEqual(rows);
  });

  it("按 标题 / 艺术家 / 专辑 / 路径 四个字段匹配", () => {
    expect(filterTracks(rows, "晴天").map((t) => t.id)).toEqual([2, 3]);
    expect(filterTracks(rows, "周杰伦").map((t) => t.id)).toEqual([2, 3]);
    expect(filterTracks(rows, "叶惠美").map((t) => t.id)).toEqual([2]);
    expect(filterTracks(rows, "/m/live/").map((t) => t.id)).toEqual([3]);
  });

  it("大小写不敏感", () => {
    expect(filterTracks(rows, "live").map((t) => t.id)).toEqual([3]);
  });

  it("不修改原数组", () => {
    const before = rows.map((t) => t.id);
    filterTracks(rows, "晴天");
    expect(rows.map((t) => t.id)).toEqual(before);
  });

  it("无命中返回空数组", () => {
    expect(filterTracks(rows, "不存在的关键词")).toEqual([]);
  });
});

describe("filterAlbums (P6.27)", () => {
  const albums: Album[] = [
    { id: 1, title: "叶惠美", artist: "周杰伦", year: 2003, trackCount: 11 },
    { id: 2, title: "世界", artist: "逃跑计划", year: 2011, trackCount: 10 },
    { id: 3, title: "范特西", artist: "周杰伦", year: 2001, trackCount: 10 },
  ];

  it("空/空白查询原样返回", () => {
    expect(filterAlbums(albums, "")).toEqual(albums);
    expect(filterAlbums(albums, "  ")).toEqual(albums);
  });

  it("按专辑名匹配", () => {
    expect(filterAlbums(albums, "惠美").map((a) => a.id)).toEqual([1]);
  });

  it("按艺术家匹配（一位艺术家多张专辑）", () => {
    expect(filterAlbums(albums, "周杰伦").map((a) => a.id)).toEqual([1, 3]);
  });

  it("按年份匹配（数字转字符串比较）", () => {
    expect(filterAlbums(albums, "2011").map((a) => a.id)).toEqual([2]);
  });

  it("大小写不敏感；无命中返回空", () => {
    expect(filterAlbums(albums, "FANTASY".toLowerCase()).length).toBe(0);
    expect(filterAlbums(albums, "不存在")).toEqual([]);
  });
});

describe("filterArtists (P6.27)", () => {
  const artists: Artist[] = [
    { id: 1, name: "周杰伦", trackCount: 12 },
    { id: 2, name: "逃跑计划", trackCount: 3 },
    { id: 3, name: "Jay Chou", trackCount: 5 },
  ];

  it("空查询原样返回", () => {
    expect(filterArtists(artists, "")).toEqual(artists);
  });

  it("按艺术家名匹配，大小写不敏感", () => {
    expect(filterArtists(artists, "周杰伦").map((a) => a.id)).toEqual([1]);
    expect(filterArtists(artists, "jay").map((a) => a.id)).toEqual([3]);
  });

  it("无命中返回空数组", () => {
    expect(filterArtists(artists, "不存在")).toEqual([]);
  });
});
