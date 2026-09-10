#!/usr/bin/env bash
# 发布前门禁（P3）：一条命令跑完全部「发布前必须为真」的检查。
#
# 用法:
#   bash scripts/release-check.sh                      # 基础门禁（本地/CI 均可）
#   bash scripts/release-check.sh --lifecycle <bin>    # 追加生命周期冒烟（需 server 二进制）
#
# 设计原则：
# - **可离线**：不访问网络（除 npm ci 由调用方决定是否先跑）；
# - **可组合**：复用既有护栏脚本（契约 / fpk 结构 / 版本），避免规则双份；
# - **失败即发布阻断**：任何一项失败 → 退出码 1（CI 的 tag job 会因此拦住发布）。
set -uo pipefail

cd "$(dirname "$0")/.."

LIFECYCLE_BIN=""
if [ "${1:-}" = "--lifecycle" ] && [ -n "${2:-}" ]; then
  LIFECYCLE_BIN="$2"
fi

rc=0
pass() { echo "  ✓ $1"; }
fail() { echo "  X $1"; rc=1; }
section() { echo; echo "== $1 =="; }

# ---- 1. 工作区干净（发布必须来自已提交状态）----
section "工作区状态"
if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
  fail "工作区有未提交改动（发布必须来自干净提交）"
  git status --short | sed 's/^/      /'
else
  pass "git 工作区干净"
fi

# ---- 2. 三项护栏（复用，规则不双份）----
section "契约一致性"
bash scripts/check-api-contract.sh || fail "契约护栏未通过"

section "fpk 结构"
bash scripts/check-fpk-structure.sh || fail "fpk 结构护栏未通过"

section "版本一致性"
bash scripts/check-version-consistency.sh || fail "版本一致性护栏未通过"
VERSION="$(grep -m1 '^version' musicforge-core/Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"

# ---- 3. CHANGELOG 必须含当前版本条目 ----
section "CHANGELOG"
if grep -q "## \[${VERSION}\]" CHANGELOG.md 2>/dev/null; then
  pass "CHANGELOG 含 [${VERSION}] 条目"
else
  fail "CHANGELOG.md 缺少 [${VERSION}] 条目"
fi

# ---- 4. tag 语义双向校验 ----
# 本地（发布前）：tag **不应**存在（防重复发布）；
# CI（tag 触发）：tag 必然已存在 → 改为验证「tag 名 == 当前版本」的身份一致性。
section "tag 校验"
if [ "${GITHUB_REF_TYPE:-}" = "tag" ]; then
  if [ "${GITHUB_REF_NAME:-}" = "v${VERSION}" ]; then
    pass "tag ${GITHUB_REF_NAME} 与版本 ${VERSION} 一致（发布身份校验）"
  else
    fail "tag 名与版本不一致：${GITHUB_REF_NAME:-<空>} vs v${VERSION}"
  fi
elif git rev-parse -q --verify "refs/tags/v${VERSION}" >/dev/null 2>&1; then
  fail "tag v${VERSION} 已存在——该版本已发布过，请先升版本号"
else
  pass "tag v${VERSION} 未被占用（可安全发布）"
fi

# ---- 5. 前端测试（可选：无 node 时给出明确提示而非静默跳过）----
section "前端测试"
if command -v npm >/dev/null 2>&1 && [ -d musicforge-gui/ui/node_modules ]; then
  if npm --prefix musicforge-gui/ui test >/dev/null 2>&1; then
    pass "前端测试通过（vitest）"
  else
    fail "前端测试失败（npm --prefix musicforge-gui/ui test 查看详情）"
  fi
else
  echo "  ! 跳过（需要 npm 与 node_modules；CI 会跑）"
fi

# ---- 6. 生命周期冒烟（可选：需编译好的 server 二进制）----
if [ -n "$LIFECYCLE_BIN" ]; then
  section "生命周期冒烟"
  bash scripts/check-lifecycle.sh "$LIFECYCLE_BIN" musicforge-fpk/cmd/main \
    || fail "生命周期冒烟失败"
else
  echo
  echo "  ! 生命周期冒烟未跑（加 --lifecycle <server 二进制> 启用；CI build job 会跑）"
fi

echo
if [ $rc -eq 0 ]; then
  echo "发布前门禁 ✓ 全部通过（版本 ${VERSION}）"
  echo "  后续：git tag v${VERSION} && git push origin v${VERSION}（触发 tag 构建）"
else
  echo "X 发布前门禁未通过——先修复上述项再发布"
fi
exit $rc
