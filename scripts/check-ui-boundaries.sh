#!/usr/bin/env bash
# UI 错误边界覆盖断言（P4-1）：App.tsx 中所有面板级组件（<XxxPanel /> / <XxxCard />）
# 必须位于 ErrorBoundary 内。
#
# 背景（v3 审计 §3.2 / §7.2）：ServerInfoCard 曾落在所有边界之外——它请求后端
# （/version、/wizard/status），一旦渲染异常即整页白屏，ErrorBoundary 的存在意义被绕过。
# 这是"修复本身引入新边界缺陷"的实例：把约束变成 CI 断言，比靠人记可靠。
#
# 规则：按出现顺序跟踪 ErrorBoundary 的开关深度；遇见 <XxxPanel|XxxCard 时深度必须 > 0。
# （ErrorBoundary 自身以 Boundary 结尾，不在 Panel|Card 命名空间内，不会自匹配。）
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

FILE="musicforge-gui/ui/src/App.tsx"
[ -f "$FILE" ] || { echo "X 未找到 $FILE"; exit 1; }

awk '
  /<ErrorBoundary/ { depth++ }
  /<\/ErrorBoundary>/ { depth-- }
  {
    line = $0
    while (match(line, /<[A-Z][A-Za-z0-9]*(Panel|Card)/)) {
      name = substr(line, RSTART + 1, RLENGTH - 1)
      if (depth <= 0) {
        printf "X 边界越界：L%d <%s> 不在任何 ErrorBoundary 内\n", NR, name
        bad++
      }
      line = substr(line, RSTART + RLENGTH)
    }
  }
  END {
    if (bad) { printf "错误边界覆盖断言失败（%d 处越界）\n", bad; exit 1 }
    print "UI 错误边界覆盖 OK（所有 Panel/Card 组件均在 ErrorBoundary 内）"
  }
' "$FILE"
