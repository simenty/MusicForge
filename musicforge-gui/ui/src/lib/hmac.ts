// SHA-256 + HMAC-SHA256（纯 JS，零依赖）——RFC-0003 §4.2 请求签名。
//
// 为什么不用 Web Crypto（crypto.subtle）：
// 它仅在**安全上下文**（HTTPS / localhost）可用，而 fnOS 形态正是
// `http://<NAS-IP>:8787` 的内网明文访问 —— 会直接不可用。
// 纯 JS 实现无此限制，且便于与后端共用同一组测试向量（防两端算法漂移）。
//
// 规范（与 Rust 侧一致，两侧测试共用同一向量）：
//   canonical = METHOD \n path \n sha256hex(body) \n ts \n nonce
//   sign      = hex(hmac_sha256(key = token, msg = canonical))
// - METHOD 大写；path 不含 query；无 body 时按空串计算 sha256；
// - ts 为 unix 秒（十进制字符串）；nonce 为 16 字节 hex（32 字符）。

const K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

const rotr = (x: number, n: number): number => ((x >>> n) | (x << (32 - n))) >>> 0;

/** SHA-256（纯 JS）。输入任意字节，输出 32 字节摘要。 */
export function sha256Bytes(data: Uint8Array): Uint8Array {
  const H = new Uint32Array([
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
  ]);
  const len = data.length;
  const bitLen = len * 8;
  const paddedLen = ((len + 9 + 63) >> 6) << 6;
  const msg = new Uint8Array(paddedLen);
  msg.set(data);
  msg[len] = 0x80;
  // 64 位长度（大端）：高位/低位分写（消息长度远小于 2^53，JS 数字安全）
  const hi = Math.floor(bitLen / 0x100000000);
  const lo = bitLen >>> 0;
  msg[paddedLen - 8] = (hi >>> 24) & 0xff;
  msg[paddedLen - 7] = (hi >>> 16) & 0xff;
  msg[paddedLen - 6] = (hi >>> 8) & 0xff;
  msg[paddedLen - 5] = hi & 0xff;
  msg[paddedLen - 4] = (lo >>> 24) & 0xff;
  msg[paddedLen - 3] = (lo >>> 16) & 0xff;
  msg[paddedLen - 2] = (lo >>> 8) & 0xff;
  msg[paddedLen - 1] = lo & 0xff;

  const w = new Uint32Array(64);
  for (let i = 0; i < paddedLen; i += 64) {
    for (let t = 0; t < 16; t++) {
      w[t] =
        ((msg[i + t * 4] << 24) |
          (msg[i + t * 4 + 1] << 16) |
          (msg[i + t * 4 + 2] << 8) |
          msg[i + t * 4 + 3]) >>>
        0;
    }
    for (let t = 16; t < 64; t++) {
      const s0 = (rotr(w[t - 15], 7) ^ rotr(w[t - 15], 18) ^ (w[t - 15] >>> 3)) >>> 0;
      const s1 = (rotr(w[t - 2], 17) ^ rotr(w[t - 2], 19) ^ (w[t - 2] >>> 10)) >>> 0;
      w[t] = (w[t - 16] + s0 + w[t - 7] + s1) >>> 0;
    }
    let a = H[0], b = H[1], c = H[2], d = H[3], e = H[4], f = H[5], g = H[6], h = H[7];
    for (let t = 0; t < 64; t++) {
      const S1 = (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) >>> 0;
      const ch = (e & f) ^ (~e & g);
      const t1 = (h + S1 + ch + K[t] + w[t]) >>> 0;
      const S0 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) >>> 0;
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const t2 = (S0 + maj) >>> 0;
      h = g; g = f; f = e; e = (d + t1) >>> 0;
      d = c; c = b; b = a; a = (t1 + t2) >>> 0;
    }
    H[0] = (H[0] + a) >>> 0; H[1] = (H[1] + b) >>> 0;
    H[2] = (H[2] + c) >>> 0; H[3] = (H[3] + d) >>> 0;
    H[4] = (H[4] + e) >>> 0; H[5] = (H[5] + f) >>> 0;
    H[6] = (H[6] + g) >>> 0; H[7] = (H[7] + h) >>> 0;
  }

  const out = new Uint8Array(32);
  for (let i = 0; i < 8; i++) {
    out[i * 4] = (H[i] >>> 24) & 0xff;
    out[i * 4 + 1] = (H[i] >>> 16) & 0xff;
    out[i * 4 + 2] = (H[i] >>> 8) & 0xff;
    out[i * 4 + 3] = H[i] & 0xff;
  }
  return out;
}

function toHex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}

/** UTF-8 编码（导出：测试需要构造**字节级**输入，如 RFC 4231 的 0xaa×131 key）。 */
export function utf8Bytes(s: string): Uint8Array {
  return new TextEncoder().encode(s);
}

/** SHA-256 → 小写 hex。 */
export function sha256Hex(input: string): string {
  return toHex(sha256Bytes(utf8Bytes(input)));
}

/**
 * HMAC-SHA256（**字节版**）→ 32 字节摘要。
 *
 * 为什么要有字节版：HMAC 的 key/msg 本质是**字节串**而非「字符串」——
 * 例如 RFC 4231 的长 key 用例是 0xaa×131（131 **字节**），
 * 若按字符串构造（`"\xaa".repeat(131)`）经 UTF-8 会变成 262 字节，语义就错了。
 */
export function hmacSha256Bytes(key: Uint8Array, msg: Uint8Array): Uint8Array {
  const block = 64;
  const k0 = key.length > block ? sha256Bytes(key) : key;
  const ipad = new Uint8Array(block);
  const opad = new Uint8Array(block);
  for (let i = 0; i < block; i++) {
    const kb = i < k0.length ? k0[i] : 0;
    ipad[i] = kb ^ 0x36;
    opad[i] = kb ^ 0x5c;
  }
  const inner = new Uint8Array(block + msg.length);
  inner.set(ipad, 0);
  inner.set(msg, block);
  const innerHash = sha256Bytes(inner);
  const outer = new Uint8Array(block + 32);
  outer.set(opad, 0);
  outer.set(innerHash, block);
  return sha256Bytes(outer);
}

/** HMAC-SHA256（字符串版：key 与 msg 均按 UTF-8 编码）→ 小写 hex。 */
export function hmacSha256Hex(key: string, msg: string): string {
  return toHex(hmacSha256Bytes(utf8Bytes(key), utf8Bytes(msg)));
}

/** 规范化串（与 Rust 侧逐字节一致；两侧测试共用向量）。 */
export function buildCanonical(
  method: string,
  path: string,
  body: string,
  ts: string,
  nonce: string
): string {
  return `${method.toUpperCase()}\n${path}\n${sha256Hex(body)}\n${ts}\n${nonce}`;
}

/** 计算请求签名（hex）。 */
export function signRequest(
  token: string,
  method: string,
  path: string,
  body: string,
  ts: string,
  nonce: string
): string {
  return hmacSha256Hex(token, buildCanonical(method, path, body, ts, nonce));
}

/** 生成 16 字节随机 nonce（hex）。crypto.getRandomValues 在非安全上下文亦可用，仍留兜底。 */
export function makeNonce(): string {
  const buf = new Uint8Array(16);
  const c = globalThis.crypto;
  if (c && typeof c.getRandomValues === "function") {
    c.getRandomValues(buf);
  } else {
    for (let i = 0; i < buf.length; i++) buf[i] = Math.floor(Math.random() * 256);
  }
  return toHex(buf);
}
