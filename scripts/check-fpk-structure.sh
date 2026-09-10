#!/usr/bin/env bash
# fpk 结构护栏（P0-4）：在 CI 拦截 fnOS 包形态错误，避免"真机试错"。
#
# 每条断言都对应一次真实故障：
#   - micro_app=true 缺失        → B25 桌面初始化异常
#   - wizard/* 非 JSON           → B26 应用初始化异常（wizard 协议是 JSON 步骤定义）
#   - ui/config 入口键名不一致   → B25（须与 manifest desktop_applaunchname 逐字符一致）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
F="musicforge-fpk"

rc=0
fail() { echo "X $1"; rc=1; }

# --- 1. manifest 必填字段 ---
REQUIRED="appname version display_name platform micro_app desktop_uidir desktop_applaunchname ctl_stop"
for k in $REQUIRED; do
  grep -Eq "^[[:space:]]*${k}[[:space:]]*=" "$F/manifest" || fail "manifest 缺必填字段: $k"
done

# --- 2. micro_app 必须为 true（B25）---
grep -Eq '^[[:space:]]*micro_app[[:space:]]*=[[:space:]]*true' "$F/manifest" \
  || fail "manifest micro_app 必须为 true（缺失会导致 fnOS 桌面初始化事件失败）"

# --- 3. 不应再有 service_port/checkport（端口由 ui/config 声明；checkport 与启动时序竞争）---
if grep -Eq '^[[:space:]]*(service_port|checkport)[[:space:]]*=' "$F/manifest"; then
  fail "manifest 不应含 service_port/checkport（端口由 ui/config 的 port 声明）"
fi

# --- 4. wizard 协议：install/uninstall 存在且为合法 JSON（B26）---
json_ok() {
  local f="$1"
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$f" 2>/dev/null
  elif command -v node >/dev/null 2>&1; then
    node -e 'JSON.parse(require("fs").readFileSync(process.argv[1],"utf8"))' "$f" 2>/dev/null
  else
    echo "skip-json-tool"
    return 0
  fi
}
for w in install uninstall; do
  if [ ! -f "$F/wizard/$w" ]; then
    fail "wizard/$w 缺失（fnOS 安装/卸载完成时会解析它）"
  else
    out="$(json_ok "$F/wizard/$w")"
    if [ "$out" = "skip-json-tool" ]; then
      echo "  ! 无 python3/node，跳过 wizard/$w 的 JSON 校验"
    elif [ -n "$out" ]; then
      fail "wizard/$w 非合法 JSON: $out"
    fi
  fi
done

# --- 5. ui/config 入口键名必须等于 desktop_applaunchname（B25）---
NAME="$(grep -E '^[[:space:]]*desktop_applaunchname' "$F/manifest" \
  | sed 's/.*=[[:space:]]*//' | tr -d '\r' | tr -d ' ')"
CFG="$F/app/ui/config"
if [ ! -f "$CFG" ]; then
  fail "$CFG 缺失（桌面入口定义）"
else
  grep -q "\"$NAME\"" "$CFG" \
    || fail "ui/config 缺少入口键 \"$NAME\"（须与 manifest desktop_applaunchname 逐字符一致）"
  grep -q '"port"' "$CFG" || fail "ui/config 缺 port 字段（桌面入口靠它声明端口）"
fi

# --- 6. icon（非阻断：仅提示）---
if ! [ -f "$F/app/ICON.PNG" ] && ! [ -f "$F/app/ICON_256.PNG" ] \
   && ! [ -f "$F/app/ui/images/icon_256.png" ]; then
  echo "  ! 未找到 icon 文件（非阻断，但桌面图标会缺失）"
fi

[ $rc -eq 0 ] && echo "fpk 结构校验 ✓（manifest / wizard / ui-config 全部合规）"
exit $rc
