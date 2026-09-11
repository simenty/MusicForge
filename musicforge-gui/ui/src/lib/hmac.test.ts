// RFC-0003 §4.2 请求签名 —— 正确性与**跨语言一致性**测试。
//
// 跨语言向量与 Rust 侧（musicforge-server/src/lib.rs 的 M2 测试）**共用同一组**
// （由 node:crypto 生成）：任一侧的 canonical/HMAC 实现发生漂移，两侧测试会同时变红。
import { describe, expect, it } from "vitest";
import {
  buildCanonical,
  hmacSha256Bytes,
  hmacSha256Hex,
  makeNonce,
  sha256Hex,
  signRequest,
  utf8Bytes,
} from "./hmac";

const VEC_TOKEN = "test-token";
const VEC_TS = "1760000000";
const VEC_NONCE = "00112233445566778899aabbccddeeff";
const VEC_SIG_POST = "0a759dd225ade9ae7d3fdda28cfa090f0854d05347aba5886be5d6f2c55db3f4";
const VEC_SIG_GET = "fe03de00ac0eaecc8faf137667db7097d61b8243e572e25ed26f5ea58df5edc4";

describe("hmac（RFC-0003 §4.2 请求签名）", () => {
  it("SHA-256 标准向量（FIPS 180-2：#abc）", () => {
    expect(sha256Hex("abc")).toBe(
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
  });

  it("SHA-256 空串向量", () => {
    expect(sha256Hex("")).toBe(
      "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
  });

  it("HMAC-SHA256 标准向量（RFC 4231 Test Case 1）", () => {
    // key = 0x0b × 20，data = "Hi There"
    expect(hmacSha256Hex("\x0b".repeat(20), "Hi There")).toBe(
      "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
  });

  it("HMAC-SHA256 长 key（> block size，先哈希；RFC 4231 Test Case 6）", () => {
    // key = 0xaa × **131 字节**（注意：必须用字节版——"\xaa".repeat(131) 经 UTF-8 会变 262 字节）
    const key = new Uint8Array(131).fill(0xaa);
    const msg = utf8Bytes("Test Using Larger Than Block-Size Key - Hash Key First");
    expect(Array.from(hmacSha256Bytes(key, msg), (b) => b.toString(16).padStart(2, "0")).join("")).toBe(
      "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
  });

  it("跨语言向量：body 摘要 + POST/GET 签名（与 Rust 侧逐字节一致）", () => {
    expect(sha256Hex('{"a":1}')).toBe(
      "015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
    );
    expect(signRequest(VEC_TOKEN, "POST", "/api/batch", '{"a":1}', VEC_TS, VEC_NONCE)).toBe(
      VEC_SIG_POST
    );
    expect(signRequest(VEC_TOKEN, "GET", "/api/version", "", VEC_TS, VEC_NONCE)).toBe(VEC_SIG_GET);
  });

  it("canonical 形状：METHOD\\npath\\nsha256(body)\\nts\\nnonce（method 统一大写）", () => {
    expect(buildCanonical("post", "/api/x", "", "1", "ab")).toBe(
      `POST\n/api/x\n${sha256Hex("")}\n1\nab`
    );
  });

  it("nonce：32 位 hex 且两次不重复", () => {
    const a = makeNonce();
    const b = makeNonce();
    expect(a).toMatch(/^[0-9a-f]{32}$/);
    expect(b).toMatch(/^[0-9a-f]{32}$/);
    expect(a).not.toBe(b);
  });

  it("篡改可检出：body / path / token 任一变化 → 签名变化", () => {
    const base = signRequest(VEC_TOKEN, "POST", "/api/batch", '{"a":1}', VEC_TS, VEC_NONCE);
    expect(signRequest(VEC_TOKEN, "POST", "/api/batch", '{"a":2}', VEC_TS, VEC_NONCE)).not.toBe(base);
    expect(signRequest(VEC_TOKEN, "POST", "/api/other", '{"a":1}', VEC_TS, VEC_NONCE)).not.toBe(base);
    expect(signRequest("other", "POST", "/api/batch", '{"a":1}', VEC_TS, VEC_NONCE)).not.toBe(base);
  });
});
