# P8 施工文档 — 飞牛（fnOS）native FPK 打包与集成

> 状态：施工级方案（v2.7.1 增补，D27）。来源：fnOS 社区镜像（ckcoding/fnnas-docs）、
> fnpack 工具链实践、第三方 fpk 仓库结构实证（fn-fpk-builder-skill / Mihomo-fpk /
> fpk-compose-builder）、官方帮助中心应用授权机制文档。
> **P8 开工 checklist 第一件事**：`fnpack create` 生成官方骨架，比对本文 manifest 字段全集
> （当前以社区实证结构为准，方案修正 X34）。

## 1. 首要决策（D27）：native FPK，不做 Docker FPK

| 形态 | 机制 | 结论 |
|---|---|---|
| **native FPK** ✅ | 二进制 + 生命周期脚本直接打包 | MusicForge：Rust musl 静态二进制（server + CLI + SPA）天然契合，无 Docker 守护依赖、秒级启动、整包 <10MB |
| Docker FPK ✗ | compose + x-fnpack 包装镜像 | 不做——Docker 镜像继续服务群晖/绿联/Unraid（一套镜像多平台复用） |

## 2. 项目结构（fnpack 骨架 + MusicForge 定制）

```text
musicforge-fpk/
├── manifest                # 应用清单（key=value 格式，无空格）——字段全集待 fnpack create 校准（X34）
├── icon.png                # 桌面图标（256×256）
├── cmd/
│   └── main                # 生命周期脚本：start/stop/status（status 返回码 0=运行中，3=未运行）
├── app/
│   ├── musicforge-server   # axum 服务（musl 静态二进制）
│   ├── musicforge          # CLI（与 server 同一二进制矩阵，复用 P7 musl 产物）
│   └── ui/                 # React SPA 构建产物
├── config/
│   └── config.default.json # 默认配置（asset_mode、端口、数据目录）
├── wizard/                 # 首启向导页面（三目录授权引导 + asset_mode 选择 + token 展示）
└── docs/                   # 内置帮助（授权引导图文）
```

## 3. 生命周期脚本（cmd/main，已含 R22 合规）

```bash
#!/bin/bash
# 用法: main {start|stop|status}；status 返回码: 0=运行中, 3=未运行
APP_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA_DIR="${APP_DATA_DIR:-/vol1/@appcenter/musicforge/data}"
PORT="${MUSICFORGE_PORT:-8787}"
PID_FILE="$DATA_DIR/server.pid"
LOG_FILE="$DATA_DIR/logs/server.log"

case "$1" in
  start)
    mkdir -p "$DATA_DIR/logs"
    # R22：首启生成随机 token 并打印日志，无默认口令
    [ ! -f "$DATA_DIR/.token" ] && head -c 24 /dev/urandom | base64 > "$DATA_DIR/.token"
    MUSICFORGE_DATA_DIR="$DATA_DIR" \
    MUSICFORGE_BIND="127.0.0.1:${PORT}" \
    MUSICFORGE_TOKEN_FILE="$DATA_DIR/.token" \
    nohup "$APP_ROOT/app/musicforge-server" >> "$LOG_FILE" 2>&1 &
    echo $! > "$PID_FILE" ;;
  stop)
    [ -f "$PID_FILE" ] && kill "$(cat "$PID_FILE")" && rm -f "$PID_FILE" ;;
  status)
    [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null && exit 0
    exit 3 ;;
esac
```

要点：二进制全 musl 静态链接（不依赖系统库）；数据目录 = 应用自有空间——**library.db 落这里，
天然满足 X16 位置铁律**（本地存储，非网络挂载）。默认绑定 `127.0.0.1`（R22：对外暴露需
显式确认；fnOS 网关集成方式在 P8 真机阶段定）。

## 4. 目录授权对接（fnOS 最关键集成点）

fnOS 权限模型：应用访问用户文件必须由管理员在「系统设置 → 应用 → MusicForge → +文件夹」
显式授权（只读/读写）。

| 目录角色 | 授权要求 | MusicForge 行为 |
|---|---|---|
| 音乐库（扫描源） | **只读** 即可 | scan / clean dry-run 全部可用 |
| 输出目录 | **读写** | convert / organize 输出 |
| 回收站 | **读写** | Trash 机制（§4.7 崩溃安全依赖） |
| 应用数据目录 | 无需授权 | db / manifest / 日志 / token |

配套设计：
- **首启向导（wizard）**：图文引导完成三个目录授权 → 选择 `asset_mode`（fnOS 默认待 R20 实测）→ 展示随机 token。
- **运行期检测**：扫描遇未授权目录 → 新稳定码 **`MF-DIR-NOT-AUTHORIZED`** + 授权引导文案（walker 上报未授权清单）。
- **权限降级语义**：只读授权目录上执行破坏类操作 → 拒绝并报稳定码（与 P8 `:ro` 断言同语义）。

## 5. 安装与分发渠道（四条）

| 渠道 | 方式 | 阶段 |
|---|---|---|
| 应用商店手动安装 | 应用中心 → 设置 → 手动安装 → 上传 .fpk（官方已开放离线安装） | **v0.9.0 首发** |
| SSH 安装 | `appcenter-cli install-fpk musicforge.fpk` | v0.9.0 同步（高级用户/脚本化） |
| FnDepot 第三方商店 | 提交 fpk（需 amd64 + arm64 双包） | v0.9.0 后评估 |
| 官方商店上架 | 审核流程 | v1.0 后 |

## 6. CI/CD 与多架构（并入 D26 渠道矩阵；R23 规避）

```yaml
# .github/workflows/release-fpk.yml（片段）
# R23：fnpack Windows 版工具链粗糙（下载无 .exe 后缀等）→ 固定 Linux runner 打包
- run: |
    curl -L -o fnpack https://developer.fnnas.com/.../fnpack-linux-amd64 && chmod +x fnpack
- run: |
    cp target/x86_64-unknown-linux-musl/release/musicforge{,-server} fpk-amd64/app/
    ./fnpack build fpk-amd64        # musicforge_x.y.z_amd64.fpk
    cp target/aarch64-unknown-linux-musl/release/musicforge{,-server} fpk-arm64/app/
    ./fnpack build fpk-arm64        # musicforge_x.y.z_arm64.fpk
- run: sha256sum *.fpk >> SHA256SUMS.txt   # D6 完整性约定覆盖 fpk
```

**升级/卸载语义**：覆盖安装保留数据目录（library.db 走 §4.13 `PRAGMA user_version` 迁移）；
卸载默认保留 `/data`，由用户手动清理（文档注明）。

## 7. P8 验收增补（v2.7.1）

- [ ] fpk 真机全生命周期：安装 → 启动 → `status`=0 → 停止 → `status`=3 → 卸载
- [ ] 未授权目录扫描报 `MF-DIR-NOT-AUTHORIZED`（含授权引导文案）
- [ ] amd64/arm64 双架构包均入 Release 且过 sha256 校验
- [ ] 只读授权目录上执行破坏类操作被拒绝（稳定码）
- [ ] 首启 wizard 走通三目录授权 + token 展示
