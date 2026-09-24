# 发布流程（Release process）

> 面向维护者。目标：从「代码已合并」到「用户可自动更新」全流程可复现。

## 0. 一次性配置：updater 签名密钥

updater 的更新包必须签名（minisign）。密钥只生成一次：

1. **生成密钥对**（本地；私钥**永不入库**）：

   ```bash
   cd musicforge-gui
   npx tauri signer generate -w ~/.tauri/musicforge.key -p "<强密码>"
   ```

   ⚠️ **密码不能为空**：tauri CLI 把空密码视为「未设置」→ 每次构建都会交互式提示，
   CI/脚本环境直接卡死（P5 实测教训）。本项目密码存于 `~/.tauri/musicforge.key.password`。

2. **公钥**（`.key.pub` 的整行 base64）写入 `musicforge-gui/src-tauri/tauri.conf.json`
   的 `plugins.updater.pubkey`（本项目已写入）。
   ⚠️ 更换公钥 = 已装用户不再信任旧签名 —— 必须配套发布说明，不能静默更换。

3. **私钥配进 GitHub Secrets**（Settings → Secrets and variables → Actions）：

   | Secret | 值 |
   |:--|:--|
   | `TAURI_SIGNING_PRIVATE_KEY` | `~/.tauri/musicforge.key` 文件的完整内容 |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | `~/.tauri/musicforge.key.password` 的内容（非空） |

4. **本地构建也需密钥**（`createUpdaterArtifacts: true` 后 `tauri build` 强制签名）：

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = [System.IO.File]::ReadAllText("$env:USERPROFILE\.tauri\musicforge.key")
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [System.IO.File]::ReadAllText("$env:USERPROFILE\.tauri\musicforge.key.password")
   ```

   未配置 secret 时，CI 的 updater job 会**自动跳过**（Guard 步骤），不影响 NSIS / CLI 资产发布。

## 1. 改版本号（两处必须一致）

- `musicforge-gui/src-tauri/tauri.conf.json` → `version`
- `musicforge-gui/src-tauri/Cargo.toml` → `version`

## 2. 打 tag 发布

```bash
git tag v0.11.0 && git push origin v0.11.0
```

`Release (multi-platform)` 自动执行：

1. **nsis job**（windows）：CLI + GUI + 自定义 NSIS 安装包；
   **静默安装 → 卸载 → 校验清理**的完整生命周期冒烟。
2. **unix-assets job**：linux-musl 静态 CLI / macOS aarch64 CLI → tar.gz。
3. **release job**：统一资产 + `SHA256SUMS.txt` + `SBOM.cdx.json` → GitHub Release。
4. **updater job**（需签名 secret）：tauri bundler 产**签名更新包**（`.nsis.zip` + `.sig`）
   与 `latest.json`，追加到同一 Release。

## 3. 发布后验收（5 分钟）

- **更新链路**：装上一版 → 设置 → 「更新」→ 检查更新 → 下载安装 → 重启 → 版本生效；
  断网时点检查 → 报错且**不影响任何其他功能**（唯一网络行为的边界）。
- **签名**：`latest.json` 的 `signature` 与 Release 里 `.nsis.zip.sig` 内容一致。
- **文件关联**：安装后双击 `.ncm` → 应用打开（冷启动入列）；已运行时双击 → 聚焦 + 入列（单实例）。
- **卸载**：控制面板卸载后 `%LOCALAPPDATA%\Programs\MusicForge` 无残留；
  `%APPDATA%\MusicForge`（数据目录）按策略保留（用户数据不主动删除）。

## 4. 手动灰度（不发 tag）

Actions → Release (multi-platform) → Run workflow → 填 `version`。

## 5. 崩溃日志

`%APPDATA%\MusicForge\logs\crash.log`（release 为 `panic = "abort"`，panic hook 先于 abort
执行，故仍有留痕）。用户报障时先要这个文件。

## 6. 跨平台 GUI（Linux / macOS）

**正式 Release 已收编**（`release-installer.yml` 的 `linux-gui` / `macos-gui` job）：
发布资产含

| 资产 | 平台 | 签名 |
|:--|:--|:--|
| `musicforge-v{v}-linux-x86_64.deb` | Linux（Debian/Ubuntu） | 未签名 |
| `musicforge-v{v}-linux-x86_64.AppImage` | Linux（通用） | 未签名 |
| `musicforge-v{v}-macos-aarch64.dmg` | macOS（Apple Silicon） | 未签名（**用户需右键→打开** 手动放行） |

- **持续构建验证**：`GUI portability` workflow 在 gui/core 变更时跑同样的构建（artifact 留档），
  防止跨平台构建回归
- updater：跨平台包**不产 updater 产物**（`--config '{"bundle":{"createUpdaterArtifacts":false}}'`），
  免签名密钥；自动更新仍以 Windows 先行（见第 0/3 节）
- Linux 构建依赖：`libwebkit2gtk-4.1-dev`、`libappindicator3-dev`、`librsvg2-dev`、`patchelf`、`libasound2-dev`
- macOS 正式签名（公证）为后续项

## 7. 来源可验证性（minisign）

`SHA256SUMS.txt` 只防**传输损坏**、不防**篡改**：能替换二进制的人可以同时替换校验和。
因此两条发布线都对产物做 **minisign（ed25519）签名**——签名与校验和分离，
校验和本身也被签名覆盖。

**签名范围**（`scripts/sign-artifacts.sh`，幂等、跳过已有 `.minisig`）：

| 发布线 | 签什么 | 何时签 |
|:--|:--|:--|
| `release-fpk.yml` | `fpk-out/*`（fpk + `SHA256SUMS-fpk.txt`） | fpk 构建后 |
| `release-installer.yml` | `assets/*`（全平台安装包 + `SBOM` + **`SHA256SUMS.txt`**） | 展平 + 生成校验和后 |

**用户验证**（拿到 Release 里的 `minisign.pub` 与 `*.minisig`）：

```bash
# 1) 先确认 SHA256SUMS.txt 本身的签名（防校验和被替换）
minisign -Vm SHA256SUMS.txt -p minisign.pub
# 2) 再校验实际下载到的文件
sha256sum -c SHA256SUMS.txt
# 单个产物也可单独验：minisign -Vm musicforge-v1.0.0-x86_64-unknown-linux-musl.tar.gz -p minisign.pub
```

**当前状态（如实标注）**：仓库**尚未提交 `minisign.pub`，也未配置 `MINISIGN_SECRET_KEY`**
→ 两条线的签名步骤都会打印配置指引并**跳过**，产物未签名但**发布不受影响**
（脚本 `exit 0`，绝不因签名基础设施缺失拦停交付）。

**启用步骤**（主理人执行，一次性）：

1. `minisign -G`（口令**留空**——CI 无法交互输入；私钥由 GitHub encrypted secret 保管）
   → 产出 `minisign.key`（私钥，**绝不入库**）与 `minisign.pub`（公钥，可公开）；
2. 把 `minisign.pub` 提交到**仓库根**（发布流程会自动复制进产物并随 Release 分发）；
3. GitHub → Settings → Secrets → 新增 `MINISIGN_SECRET_KEY`，值 = `minisign.key` 全文；
4. 下一个 tag 起，产物自动附 `*.minisig`。

> Windows 安装包的**自动更新签名**是另一套（`TAURI_SIGNING_PRIVATE_KEY`，见第 0/3 节）——
> 它只服务于 updater 通道，与本节的「下载产物来源可验证」互补，不互相替代。
