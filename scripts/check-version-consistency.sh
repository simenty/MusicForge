#!/usr/bin/env bash
# 版本一致性护栏（P0-3）：core / fpk / tauri 三处版本号必须一致。
#
# 背景：v1 审计发现"版本-内容漂移"（master 已含 v0.9.0 能力但版本号陈旧）。
# 当前三处已统一为 0.9.0；本脚本把它变成 CI 断言，防止未来再次漂移。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

CORE="$(grep -m1 '^version' musicforge-core/Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
FPK="$(grep -E '^[[:space:]]*version[[:space:]]*=' musicforge-fpk/manifest \
  | sed 's/.*=[[:space:]]*//' | tr -d '\r' | tr -d ' ')"
TAURI="$(grep -m1 '"version"' musicforge-gui/src-tauri/tauri.conf.json \
  | sed 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')"

echo "core=$CORE  fpk=$FPK  tauri=$TAURI"

rc=0
[ -n "$CORE" ] || { echo "X 未能解析 core 版本"; rc=1; }
[ "$CORE" = "$FPK" ] || { echo "X 版本漂移：core($CORE) != fpk($FPK)"; rc=1; }
[ "$CORE" = "$TAURI" ] || { echo "X 版本漂移：core($CORE) != tauri($TAURI)"; rc=1; }

# CHANGELOG 顶层条目应包含当前版本（提示级：避免"发了版本没写 CHANGELOG"）
if ! grep -q "## \[${CORE}\]" CHANGELOG.md 2>/dev/null; then
  echo "  ! CHANGELOG.md 未见 [${CORE}] 条目（建议补）"
fi

[ $rc -eq 0 ] && echo "版本一致性 ✓ ($CORE)"
exit $rc
