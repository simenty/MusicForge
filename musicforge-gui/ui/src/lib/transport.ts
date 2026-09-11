// 传输基础层（P4-3 从 api.ts 拆出）：双形态分发（Tauri IPC / HTTP fetch）+
// 访问 token + 稳定错误码 + 请求原语。
//
// P8.2.5 双形态传输：Tauri 桌面 = invoke IPC；fnOS 服务端 = fetch 同源 API。
// 未接 HTTP 的命令在服务端形态下经 invoke 包装**显式降级**（MF-DESKTOP-ONLY，
// 降级铁律：绝不静默装作可用）。
//
// 拆分原则：**零语义变化**——两处请求原语（httpPost/httpGet）保持原样搬运，
// 其内部解包逻辑的重复留待有测试覆盖时再合并。
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { type UnlistenFn } from "@tauri-apps/api/event";
import { makeNonce, signRequest } from "./hmac";

export type { UnlistenFn };

/** Tauri 桌面环境探测（fnOS server 形态下为 false） */
export const IS_DESKTOP =
  typeof window !== "undefined" &&
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !==
    undefined;

/** 是否服务端形态（面板据此提示能力边界） */
export const IS_SERVER_MODE = !IS_DESKTOP;

const TOKEN_KEY = "mf.server.token";

/** fnOS 服务端形态的访问 token（用户从服务端首启日志获取，存 localStorage） */
export function serverToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? "";
}

export function setServerToken(t: string): void {
  localStorage.setItem(TOKEN_KEY, t);
}

/** HTTP 形态错误：code = MF-* 稳定码（与 CLI/GUI 同码表），message 原文 */
export class HttpApiError extends Error {
  code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

/**
 * M2（RFC-0003 §4.2）：统一构造带签名的请求头。
 *
 * - token 为空时**不签名**——让后端报「缺 token」（可操作）而不是「签名无效」（误导）；
 * - 签名覆盖 method + 完整 path + body 摘要 + 时间戳 + nonce（服务端另有 60s 时间窗与 nonce 去重）；
 * - 与服务端约定：path 为**完整路径**（含 `/api` 前缀；服务端用 OriginalUri 取，见 lib.rs）。
 */
function signedHeaders(method: "GET" | "POST", path: string, body: string): Record<string, string> {
  const tok = serverToken();
  const headers: Record<string, string> = { "x-token": tok };
  if (tok) {
    const ts = String(Math.floor(Date.now() / 1000));
    const nonce = makeNonce();
    headers["x-mf-ts"] = ts;
    headers["x-mf-nonce"] = nonce;
    headers["x-mf-sign"] = signRequest(tok, method, path, body, ts, nonce);
  }
  return headers;
}

export async function httpPost<T>(path: string, body?: unknown): Promise<T> {
  // B19: 30s 超时——网络挂起时显式失败而非永久 pending
  const json = JSON.stringify(body ?? {});
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 30_000);
  let res: Response;
  try {
    res = await fetch(path, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...signedHeaders("POST", path, json),
      },
      body: json,
      signal: ctl.signal,
    });
  } catch (e) {
    clearTimeout(timer);
    throw new HttpApiError(
      "MF-HTTP-FAILED",
      `服务端不可达或超时（${e instanceof Error && e.name === "AbortError" ? "30s 超时" : String(e)}）`
    );
  }
  clearTimeout(timer);
  let v: { ok: boolean; data?: T; code?: string; message?: string };
  try {
    v = (await res.json()) as typeof v;
  } catch {
    throw new HttpApiError("MF-HTTP-FAILED", `服务端响应非 JSON（HTTP ${res.status}）`);
  }
  if (!v.ok) {
    throw new HttpApiError(v.code ?? "MF-HTTP-FAILED", v.message ?? `HTTP ${res.status}`);
  }
  return v.data as T;
}

export async function httpGet<T>(path: string): Promise<T> {
  // 与 httpPost 同构（30s 超时 + {ok,data} 解包），method 为 GET、无 body
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 30_000);
  let res: Response;
  try {
    res = await fetch(path, {
      method: "GET",
      headers: signedHeaders("GET", path, ""),
      signal: ctl.signal,
    });
  } catch (e) {
    clearTimeout(timer);
    throw new HttpApiError(
      "MF-HTTP-FAILED",
      `服务端不可达或超时（${e instanceof Error && e.name === "AbortError" ? "30s 超时" : String(e)}）`
    );
  }
  clearTimeout(timer);
  let v: { ok: boolean; data?: T; code?: string; message?: string };
  try {
    v = (await res.json()) as typeof v;
  } catch {
    throw new HttpApiError("MF-HTTP-FAILED", `服务端响应非 JSON（HTTP ${res.status}）`);
  }
  if (!v.ok) {
    throw new HttpApiError(v.code ?? "MF-HTTP-FAILED", v.message ?? `HTTP ${res.status}`);
  }
  return v.data as T;
}

/**
 * 命令调用的双形态分发：桌面直通 Tauri IPC；服务端形态对未接 HTTP 的命令
 * 显式降级（已接线命令在各自函数内走 httpPost，不经过此包装的降级路径）。
 */
export async function invoke<T>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  if (IS_DESKTOP) return tauriInvoke<T>(cmd, args);
  throw new HttpApiError(
    "MF-DESKTOP-ONLY",
    `功能 ${cmd} 需要桌面版：fnOS 服务端形态当前提供 扫描/格式迁移/整理/清洗 域`
  );
}
