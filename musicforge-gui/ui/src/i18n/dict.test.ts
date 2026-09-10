import { describe, expect, it } from "vitest";
import { zh } from "./zh";
import { en } from "./en";

/**
 * 字典同构测试（P1-4）。
 *
 * TS 的 `en: typeof zh` 只保证**类型形状**一致，无法保证：
 * - 英文侧真的填了值（空串/占位未翻译仍能过编译）；
 * - 插值函数数量的实际一致（类型相同即可）。
 *
 * 因此这里在**运行期**递归比对两棵树：键集合、值种类（string/fn/object）、非空。
 */
type Node = Record<string, unknown>;

function shape(o: unknown): string {
  if (typeof o === "function") return "fn";
  if (Array.isArray(o)) return "array";
  if (o && typeof o === "object") {
    const keys = Object.keys(o as Node).sort();
    return "{" + keys.map((k) => `${k}:${shape((o as Node)[k])}`).join(",") + "}";
  }
  return typeof o;
}

/** 收集所有字符串叶子（含函数调用产物），用于非空校验 */
function strings(o: unknown, path = "", out: Array<[string, string]> = []) {
  if (typeof o === "string") {
    out.push([path, o]);
  } else if (o && typeof o === "object") {
    for (const [k, v] of Object.entries(o as Node)) {
      strings(v, path ? `${path}.${k}` : k, out);
    }
  }
  return out;
}

describe("i18n dictionaries", () => {
  it("zh / en 结构完全一致（键集合与值种类）", () => {
    expect(shape(en)).toBe(shape(zh));
  });

  it("无空字符串文案（漏翻译/占位残留）", () => {
    for (const [dict, name] of [
      [zh, "zh"],
      [en, "en"],
    ] as const) {
      for (const [path, v] of strings(dict)) {
        expect(v.trim(), `${name}:${path} 为空`).not.toBe("");
      }
    }
  });

  it("插值文案为函数（而非预渲染字符串）", () => {
    expect(typeof zh.app.added).toBe("function");
    expect(typeof en.app.added).toBe("function");
    expect(zh.app.added(3)).toContain("3");
    expect(en.app.added(3)).toContain("3");
  });
});
