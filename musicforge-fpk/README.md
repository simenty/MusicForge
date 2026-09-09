# musicforge-fpk — fnOS native FPK 打包（P8/D27）

MusicForge 的飞牛 fnOS **native FPK** 打包骨架（不用 Docker FPK——Docker 镜像
继续服务群晖/绿联/Unraid 多平台复用）。

## 结构

```text
musicforge-fpk/
├── manifest                # 应用清单（X34：fnpack create 落地后校准字段）
├── icon.png                # 桌面图标（当前 1×1 占位，真图标后补）
├── cmd/main                # 生命周期：start/stop/status（R22：随机 token + 回环绑定）
├── config/config.default.json
├── docs/help.md            # 内置帮助（三目录授权引导 + 数据/升级语义）
└── app/                    # 打包时填充（不入库）：musicforge-server + ui/（SPA）
```

## 打包（CI 自动 / 本地 Linux）

`app/` 由 `release-fpk.yml` 从 musl 交叉构建产物填充（amd64 与 arm64 各一包）：

```bash
cp target/x86_64-unknown-linux-musl/release/musicforge-server fpk/app/
cp -r musicforge-gui/ui/dist fpk/app/ui
./fnpack build fpk        # → musicforge_0.9.0_amd64.fpk
```

## 验收对照（P8 §7）

- [x] 生命周期脚本 R22 合规（随机 token / 回环绑定 / status 返回码 0/3）
- [ ] fpk 真机全生命周期（需 fnOS 真机）
- [x] 未授权目录稳定码 `MF-DIR-NOT-AUTHORIZED`（代码侧已注册，walker 检测接线中）
- [ ] amd64/arm64 双架构包（CI 打包线，fnpack URL 真机阶段定稿）
- [ ] 首启 wizard 三目录授权 + token 展示（server API 迭代项）
