#!/usr/bin/env bash
# 契约一致性护栏（P0-1）：server 路由 vs 前端实际调用。
#
# 拦截两类漂移：
#   1) 硬失败：前端调用了 server 不存在的端点（改后端忘了改前端 / 拼错路径）
#   2) 警告  ：server 已实现但前端未接入（后端能力没露出，用户只能回退 CLI）
#
# 2026-09-10 首次运行即暴露 7 个未接端点：
#   /organize/plan /organize/apply /clean/plan /clean/apply /trash/restore /version /wizard/status
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# --- server 端点：.route("/xxx", ...) ---
# shellcheck disable=SC2046
grep -ho '\.route("[^"]*"' $(find musicforge-server/src -name '*.rs') \
  | sed 's/\.route("//; s/"$//' | sort -u > "$tmp/server"

# --- 前端调用：只取字符串字面量 "/api/xxx" ---
# 这样可自然排除 `import ... from "@tauri-apps/api/core"` 之类的 IPC 导入路径。
grep -ho '"/api/[a-z0-9_/]*"' musicforge-gui/ui/src/api.ts \
  | tr -d '"' | sed 's|^/api||' | sort -u > "$tmp/ui"

missing="$(comm -23 "$tmp/ui" "$tmp/server")"
unused="$(comm -13 "$tmp/ui" "$tmp/server")"

rc=0
if [ -n "$missing" ]; then
  echo "X 前端调用了 server 不存在的端点（必须修复）："
  echo "$missing" | sed 's/^/    /'
  rc=1
fi

if [ -n "$unused" ]; then
  echo "! server 已实现但前端未接入（后端能力未露出，用户只能走 CLI）："
  echo "$unused" | sed 's/^/    /'
fi

if [ $rc -eq 0 ]; then
  echo "契约一致性 ✓（前端 $(wc -l < "$tmp/ui" | tr -d ' ') 端点 / server $(wc -l < "$tmp/server" | tr -d ' ') 端点）"
fi
exit $rc
