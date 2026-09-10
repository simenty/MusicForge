#!/usr/bin/env bash
# release 产物签名（minisign / ed25519）——P3 方案 A 落地（评估见 docs/signing-evaluation.md）。
#
# 用法:
#   bash scripts/sign-artifacts.sh [目录=dist]
#   MINISIGN_SECRET_KEY=/path/to/minisign.key bash scripts/sign-artifacts.sh dist
#
# 设计原则（与签名评估一致）：
# 1. **不阻断发布**：未配置密钥 / 未装 minisign / 目录不存在 → 明确提示并 exit 0。
#    发布继续（产物未签名，用户可见），绝不让签名基础设施的缺失拦停交付。
# 2. **私钥绝不落仓库**：只从环境变量（CI secret / 本地 env）读取路径；
#    本脚本不生成、不复制、不打印私钥内容。
# 3. **无口令私钥**：CI 无法交互输入口令——请用 `minisign -G` 留空口令生成
#    （私钥经 GitHub encrypted secret 保管，见 docs/signing-evaluation.md §4）。
set -uo pipefail

DIR="${1:-dist}"
KEY="${MINISIGN_SECRET_KEY:-}"
PUB="${MINISIGN_PUBKEY_FILE:-minisign.pub}"

if [ ! -d "$DIR" ]; then
  echo "! 产物目录不存在，跳过签名: $DIR"
  exit 0
fi

if [ -z "$KEY" ] || [ ! -f "$KEY" ]; then
  echo "! 未配置 MINISIGN_SECRET_KEY（私钥文件路径）——跳过产物签名。"
  echo "  · 发布继续（产物未签名）；"
  echo "  · 配置步骤见 docs/signing-evaluation.md §4（密钥生成 + GitHub secret）。"
  exit 0
fi

if ! command -v minisign >/dev/null 2>&1; then
  echo "! 未安装 minisign（ubuntu: apt-get install -y minisign）——跳过签名。"
  exit 0
fi

signed=0
skipped=0
# 遍历目录内全部普通文件（含子目录）；跳过已有的 .minisig 避免自签
while IFS= read -r -d '' f; do
  case "$f" in
    *.minisig) continue ;;
  esac
  if minisign -Sm "$f" -s "$KEY" >/dev/null 2>&1; then
    echo "  ✓ 已签名 ${f}"
    signed=$((signed + 1))
  else
    echo "  X 签名失败 ${f}"
    skipped=$((skipped + 1))
  fi
done < <(find "$DIR" -type f -print0)

echo "签名完成：${signed} 个成功${skipped:+, ${skipped} 个失败}"
if [ -f "$PUB" ]; then
  echo "公钥（发布到 README/SECURITY.md 供验证）：$PUB"
fi
# 签名失败不改变退出码——发布继续（与原则 1 一致）
exit 0
