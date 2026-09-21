// filterTracks 测试（P6.26）：详情页客户端筛选的匹配语义。
import { describe, expect, it } from "vitest";
import { filterTracks } from "./filterTracks";
import type { Track } from "./types";

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
