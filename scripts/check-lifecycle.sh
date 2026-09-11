#!/usr/bin/env bash
# 生命周期冒烟（P2-2）：覆盖 start / status / stop **以及升级路径**。
#
# 起因（B27）：旧 start 的「旧进程存活 → 跳过启动」在升级语义下是错的，
# 且「pid 文件丢失但进程仍占端口」会让新实例秒退 → fnOS 报「启用失败」。
# 这类缺陷**只在升级时暴露**，全新安装测不出来——故固化为冒烟脚本。
#
# 用法: check-lifecycle.sh <server 二进制> [main 脚本]
#   server 二进制：debug/release 均可（CI 用 release 产物）
#   main 脚本    ：默认 musicforge-fpk/cmd/main
set -uo pipefail

BIN="${1:-}"
MAIN="${2:-musicforge-fpk/cmd/main}"

if [ -z "$BIN" ] || [ ! -x "$BIN" ]; then
  echo "X 用法: $0 <musicforge-server 二进制> [main 脚本]" >&2
  exit 2
fi
if [ ! -f "$MAIN" ]; then
  echo "X 找不到生命周期脚本: $MAIN" >&2
  exit 2
fi

# 环境适配：清理逻辑依赖能读到进程名（Linux: /proc/<pid>/comm；macOS: ps -o comm=）。
# Windows/Git-bash 的 /proc 存在但无 comm 语义 → 无法安全判定，跳过而非误报失败
# （真机 fnOS 与 CI 的 ubuntu runner 均具备该能力，断言照常执行）。
probe_name() {
  if [ -r "/proc/$$/comm" ]; then
    cat "/proc/$$/comm" 2>/dev/null
    return
  fi
  command -v ps >/dev/null 2>&1 && ps -p $$ -o comm= 2>/dev/null | tail -1 | tr -d ' '
}
if [ -z "$(probe_name)" ]; then
  echo "! 当前环境读不到进程名（非 Linux/BSD 语义，如 Windows/Git-bash）："
  echo "  清理逻辑按安全优先选择「不杀」，端口占用无法复现 → 跳过本冒烟。"
  echo "  真机 fnOS 与 CI 的 ubuntu runner 具备该能力，断言照常执行。"
  exit 0
fi

TMP="$(mktemp -d)"
trap 'bash "$MAIN" stop >/dev/null 2>&1; rm -rf "$TMP"' EXIT

# 模拟 fnOS 注入的环境（扁平化解包布局）
export TRIM_APPDEST="$TMP/appdest"
export TRIM_PKGVAR="$TMP/pkglvar"
export TRIM_SERVICE_PORT="${TRIM_SERVICE_PORT:-18789}"
mkdir -p "$TRIM_APPDEST"
cp "$BIN" "$TRIM_APPDEST/musicforge-server"
chmod +x "$TRIM_APPDEST/musicforge-server"

rc=0
fail() { echo "X $1" >&2; rc=1; }
step() { echo "  → $1"; }

# ---- 1) 首次 start ----
step "start（全新安装）"
if ! bash "$MAIN" start; then fail "首次 start 失败"; exit 1; fi

# ---- 2) status 应为运行中（退出码 0）----
step "status 应为运行中"
bash "$MAIN" status || fail "启动后 status 应返回 0（运行中）"

# ---- 3) 重复 start 应幂等成功（不得报错）----
step "重复 start（幂等）"
bash "$MAIN" start || fail "重复 start 应成功（幂等）"

# ---- 4) B27 场景：pid 文件丢失但进程仍在（模拟升级残留）----
step "B27 回归：删除 pid 文件后 start（旧进程仍占端口）"
rm -f "$TRIM_PKGVAR/server.pid"
if ! bash "$MAIN" start; then
  fail "B27 回归：pid 文件丢失 + 旧进程占用时 start 失败（应清理旧实例后重启）"
fi
bash "$MAIN" status || fail "B27 回归后服务应处于运行中"

# ---- 4.5) 日志轮转（v3 审计 §5.3 / P4-1）：预置 >5MB 日志，start 应轮出 .1 ----
step "日志轮转：预置 6MB 日志后 start"
LOG_FILE="$TRIM_PKGVAR/logs/server.log"
mkdir -p "$(dirname "$LOG_FILE")"
head -c 6291456 /dev/zero > "$LOG_FILE" 2>/dev/null
if ! bash "$MAIN" start; then fail "日志轮转场景 start 失败"; fi
if [ -f "${LOG_FILE}.1" ]; then
  step "  轮转产物 OK（server.log.1，$(( $(wc -c < "${LOG_FILE}.1") / 1024 ))KB）"
else
  fail "日志未轮转：预置 6MB 后未见 ${LOG_FILE}.1（检查 start 的轮转分支）"
fi

# ---- 5) stop → status 应为未运行（退出码 3）----
step "stop → status 应为未运行（3）"
bash "$MAIN" stop >/dev/null
bash "$MAIN" status
st=$?
[ "$st" -eq 3 ] || fail "stop 后 status 应返回 3（未运行），实际 $st"

# ---- 6) 停止后再次 start（重启路径）----
step "停止后再次 start"
bash "$MAIN" start || fail "停止后重启失败"
bash "$MAIN" status || fail "重启后 status 应为运行中"

# ---- 7) 清理：stop ----
step "stop（收尾）"
bash "$MAIN" stop >/dev/null

if [ $rc -eq 0 ]; then
  echo "生命周期冒烟 ✓（start / status / 重复 start / B27 升级残留 / stop / 重启 全通过）"
fi
exit $rc
