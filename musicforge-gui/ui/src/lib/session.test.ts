// 上次会话持久化（P6.12）：往返 / 脏数据判废 / 字段回退。
import { beforeEach, describe, expect, it } from "vitest";
import { clearSession, loadSession, saveSession } from "./session";
import type { QueueItem } from "./types";

const item = (id: number): QueueItem => ({
  trackId: id,
  path: `C:\\m\\${id}.flac`,
  title: `t${id}`,
  artist: "A",
  durationMs: 1000,
});

describe("会话持久化", () => {
  beforeEach(() => localStorage.clear());

  it("往返：保存后原样读回", () => {
    saveSession({ items: [item(1), item(2)], index: 1, positionMs: 61_000, ts: 5 });
    const s = loadSession();
    expect(s).not.toBeNull();
    expect(s!.items.map((x) => x.trackId)).toEqual([1, 2]);
    expect(s!.index).toBe(1);
    expect(s!.positionMs).toBe(61_000);
    expect(s!.ts).toBe(5);
  });

  it("脏数据判废：坏 JSON / 空队列 / 全无效条目 → null", () => {
    localStorage.setItem("mf.session", "{oops");
    expect(loadSession()).toBeNull();

    saveSession({ items: [], index: 0, positionMs: 0, ts: 0 });
    expect(loadSession()).toBeNull();

    localStorage.setItem(
      "mf.session",
      JSON.stringify({ items: [{ trackId: "x" }], index: 0, positionMs: 0, ts: 0 })
    );
    expect(loadSession()).toBeNull();
  });

  it("部分脏：坏条目剔除，索引/进度回退到合法值", () => {
    localStorage.setItem(
      "mf.session",
      JSON.stringify({
        items: [item(1), { trackId: 2 }, item(3)],
        index: 9,
        positionMs: -5,
        ts: "x",
      })
    );
    const s = loadSession()!;
    // 坏条目被剔除；越界索引 / 负数进度回退到合法值
    expect(s.items.map((x) => x.trackId)).toEqual([1, 3]);
    expect(s.index).toBe(0);
    expect(s.positionMs).toBe(0);
    expect(s.ts).toBe(0);
  });

  it("清除后读取为 null", () => {
    saveSession({ items: [item(1)], index: 0, positionMs: 0, ts: 0 });
    clearSession();
    expect(loadSession()).toBeNull();
  });
});
