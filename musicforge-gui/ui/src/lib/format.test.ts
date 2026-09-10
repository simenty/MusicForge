import { describe, expect, it } from "vitest";
import { extOf, fileName, formatDuration, percent, relOutput } from "./format";

describe("fileName", () => {
  it("取 unix 路径最后一段", () => {
    expect(fileName("/vol1/music/song.ncm")).toBe("song.ncm");
  });
  it("取 windows 路径最后一段", () => {
    expect(fileName("D:\\music\\song.ncm")).toBe("song.ncm");
  });
  it("无分隔符时原样返回", () => {
    expect(fileName("song.ncm")).toBe("song.ncm");
  });
  it("混合分隔符取最右段（跨平台路径兜底）", () => {
    expect(fileName("D:/music\\sub/song.ncm")).toBe("song.ncm");
  });
});

describe("relOutput", () => {
  it("两段以内原样返回", () => {
    expect(relOutput("a/b.flac")).toBe("a/b.flac");
  });
  it("超过两段只保留末两段并加省略号", () => {
    expect(relOutput("/vol1/music/album/artist/song.flac")).toBe("…/artist/song.flac");
  });
  it("windows 分隔符归一化为 /", () => {
    expect(relOutput("D:\\music\\album\\song.flac")).toBe("…/album/song.flac");
  });
});

describe("formatDuration", () => {
  it("0 或负值显示占位符", () => {
    expect(formatDuration(0)).toBe("—");
    expect(formatDuration(-5)).toBe("—");
  });
  it("不足 60s 显示秒（一位小数）", () => {
    expect(formatDuration(1500)).toBe("1.5s");
  });
  it("60s~1h 显示分秒（补零）", () => {
    expect(formatDuration(65_000)).toBe("1m05s");
  });
  it("超过 1h 显示时分（补零）", () => {
    expect(formatDuration(3_725_000)).toBe("1h02m");
  });
});

describe("extOf", () => {
  it("取小写扩展名（不含点）", () => {
    expect(extOf("/a/b/Song.NCM")).toBe("ncm");
    expect(extOf("x.qmcflac")).toBe("qmcflac");
  });
  it("无扩展名返回空串", () => {
    expect(extOf("/a/b/README")).toBe("");
  });
});

describe("percent", () => {
  it("total 为 0 时为 0（避免除零）", () => {
    expect(percent(0, 0)).toBe(0);
  });
  it("按比例取整", () => {
    expect(percent(1, 3)).toBe(33);
    expect(percent(3, 3)).toBe(100);
  });
  it("上限 100（done 超过 total 的异常输入不溢出）", () => {
    expect(percent(5, 4)).toBe(100);
  });
});
