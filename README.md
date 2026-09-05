# MusicForge

**本地优先、可靠、可审计、可扩展的开源音乐文件处理平台。**

把散乱的音乐文件变成带封面、带歌词、按歌手/专辑/风格归档的标准曲库——以高可靠的 `.ncm`
本地解封装为起点，逐步支持无损转换、标签与封面修复、曲库扫描清洗、重复治理与 NAS 自动化。

> **No telemetry. No account. No background upload. No hidden network calls.**

> ⚠️ **法律须知**：MusicForge 仅用于处理你已合法获得的文件的个人本地格式转换/备份。
> 本项目与网易云音乐无任何关联。使用前请阅读下方[法律须知](#法律须知)全文。

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](https://rustup.rs)
[![CI](https://img.shields.io/badge/CI-Windows%20%7C%20macOS%20%7C%20Linux-green.svg)](.github/workflows/ci.yml)

## 特性

- **默认完全离线**——零网络代码路径，CI 四层扫描断言（依赖树 / 源码正则 / feature 断言 / 断网行为测试）
- **CRC32 头校验**——损坏文件明确报错，绝不静默产出损坏音频
- **命名模板**——`{artist}/{album}/{track:02d} {title}`，跨平台非法字符清洗
- **结构保留**——递归导入自动保留目录树，同名自动去重
- **可靠批量**——有界并发、单文件失败不中断、失败清单 CSV 导出
- **不覆盖已有值**——标签与封面仅在缺失时写入（字段级写策略 + 来源溯源）
- **体积极小**——CLI 1.04 MB / GUI 3.49 MB（Rust + Tauri 2 + WebView2）

### 格式能力矩阵

| 能力 | .ncm | WAV | FLAC | MP3 | AAC/M4A | QMC/MGG/MFLAC |
|:--|:-:|:-:|:-:|:-:|:-:|:-:|
| 解封装/读取 | ✅ | 📋 v0.5.0 | 📋 v0.5.0 | 📋 v0.6.0 | 📋 v0.6.0 | 🔌 可选格式插件 |
| 无损转换 | ✅ 载荷直出 | 📋 v0.5.0 | 📋 v0.5.0 | —（有损源） | —（有损源） | 🔌 插件 |
| 有损导出 | ✅ 载荷直出 | 📋 v0.6.0 | 📋 v0.6.0 | — | — | 🔌 插件 |

✅ 已支持 · 📋 规划中（版本见 [ROADMAP.md](ROADMAP.md)） · 🔌 由[可选插件](PLUGIN_POLICY.md)提供（默认禁用）

## 平台化路线与治理文档

- [ROADMAP.md](ROADMAP.md) —— 定稿路线图（P0–P9，scope 已冻结）
- [PRIVACY.md](PRIVACY.md) —— 隐私承诺：无遥测、无账号、无后台上传
- [PLUGIN_POLICY.md](PLUGIN_POLICY.md) —— 插件边界：分级、权限清单、准入规则
- [docs/threat-model.md](docs/threat-model.md) / [docs/architecture.md](docs/architecture.md) / [docs/dependency-policy.md](docs/dependency-policy.md)
- [CHANGELOG.md](CHANGELOG.md) / [SECURITY.md](SECURITY.md) / [TRADEMARK.md](TRADEMARK.md)

## 安装

### Windows（推荐：NSIS 安装包）

1. 下载 `MusicForge-0.1.0-setup.exe`（约 1.3 MB）。
2. 双击运行 —— **无需管理员权限**，默认安装到 `%LOCALAPPDATA%\Programs\MusicForge`。
3. 安装向导会先展示[法律须知](#法律须知)，同意后选择组件：
   - 主程序（必需）：`musicforge-gui.exe` + `musicforge.exe`
   - 开始菜单快捷方式
   - 桌面快捷方式（默认不创建）
4. 卸载：开始菜单 → `卸载 MusicForge`，或 Windows「添加或删除程序」→ MusicForge。

安装包**不联网、不下载任何组件、不注册文件关联、不设开机自启、不改 PATH**。
脚本源码见 [`installer/musicforge.nsi`](installer/musicforge.nsi)，可自行审计后编译。

静默安装（企业部署 / 脚本）：

```bat
MusicForge-0.1.0-setup.exe /S
"%LOCALAPPDATA%\Programs\MusicForge\Uninstall.exe" /S
```

### 免安装版

解压 `musicforge-v0.1.0-windows-x64.zip`（约 1.8 MB）到任意目录，直接运行其中的 `musicforge-gui.exe`。

### 从源码构建

```bash
git clone https://github.com/simenty/MusicForge.git
cd MusicForge

cargo build --release -p musicforge-cli            # CLI
cargo build --release -p musicforge-gui            # GUI（需先构建前端，见 CONTRIBUTING.md）
cargo test --workspace                        # 132 个测试函数（金标 + 对抗 + QA 双轮）
```

### macOS / Linux

CLI 可直接从源码构建（`cargo build --release -p musicforge-cli`）。
GUI 安装包尚未提供——Tauri 2 的 macOS（.dmg/.app）与 Linux（AppImage/deb）产物有待补齐。

## 使用

### CLI

```bash
# 转换单个文件
musicforge.exe song.ncm

# 批量 + 递归 + 输出目录 + 命名模板
musicforge.exe -d "网易云缓存目录" -r -o "音乐库" --template "{artist}/{album}/{track:02d} {title}" --skip-existing

# 跳过已存在 + 导出失败清单
musicforge.exe -d "缓存目录" -r --skip-existing --export-failures failures.csv
```

### GUI

拖入 `.ncm` 文件或文件夹 → 开始转换 → 查看逐文件状态 → 导出失败清单。

## 命名模板

| 占位符 | 含义 | 示例 |
|---|---|---|
| `{title}` | 曲目名 | 贝贝 |
| `{artist}` | 艺术家 | 李荣浩 |
| `{album}` | 专辑 | 耳朵 |
| `{track}` | 音轨号 | 1 |
| `{track:02d}` | 零填充音轨号 | 01 |
| `{format}` | 音频格式 | flac |

模板中 `/` 产生子目录。非法字符（`<>:"/\|?*`）自动替换为 `_`。

## 法律须知

<details>
<summary>展开阅读</summary>

1. 本软件仅用于处理你**已合法获得的文件**的个人本地格式转换/备份。
2. 根据《中华人民共和国著作权法》第 49 条，故意避开或者破坏技术措施属于禁止行为；第 50 条列举了五项法定例外，个人格式转换不在其中，且第 50 条设有但书——不得向他人提供避开技术措施的技术。
3. GitHub 有对涉及规避技术保护措施（TPM）项目执行 DMCA 下架的先例（2021-10 网易云音乐投诉 → 下架整个 fork 网络 3,319 个仓库；2022-11 腾讯音乐另案 497 个仓库）。风险为**低概率、高后果**。
4. 本软件不连接网络、不上传数据、不提供下载能力、不包含任何受版权保护的内容。
5. 本项目不商业化、无捐赠、无遥测。
6. **本项目不构成法律意见。是否使用由你自行判断并承担相应责任。**

</details>

## 许可

[MIT](LICENSE)

## 致谢

- 算法参考：[anonymous5l/ncmdump](https://github.com/anonymous5l/ncmdump)（已删库）、[taurusxin/ncmdump](https://github.com/taurusxin/ncmdump)（C++）、[taurusxin/ncmdump-go](https://git.taurusxin.com/taurusxin/ncmdump-go)（Go）
- NCM 格式逆向：社区共识（`vvbbnn00/NcmCrypt` 等多方交叉验证）
- CRC32 覆盖范围：本项目穷举搜索实证（`crc32(data[0:696])`，25 万+ 候选唯一命中）
