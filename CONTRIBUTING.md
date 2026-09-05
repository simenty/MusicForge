# 贡献指南

## 开发环境

- Rust 1.98+（`rustup` minimal profile 即可）
- Node 22+（仅 GUI 前端构建）
- 无其他依赖（音频编解码库由 cargo 管理）

## 快速开始

```bash
git clone https://github.com/{owner}/MusicForge.git
cd MusicForge
cargo test --workspace
cargo build --release -p musicforge-cli
```

## 代码规范

### 硬约束（方案书 §7，11 条）

核心库 `musicforge-core` 必须遵守：

1. **零 panic**——所有失败返回 `NcmError`，release 构建 `panic = "abort"`
2. **RC4 全局偏移**——`j = (offset + 1) & 0xff`，offset 为全局字节计数
3. **read_exact**——所有定长读取
4. **长度上界校验**——`0 < n <= 剩余字节数`
5. **返回值全检查**——Read/Seek/Write/Close 不丢弃
6. **零网络**——无任何 net/use 依赖（CI 扫描断言）
7. FLAAC 严格解析（lofty ParsingMode）
8. 格式三级融合
9. CRC32 头校验
10. 并发在应用层（有界 worker pool）
11. Metadata None → 跳过打标签

### Windows 三坑（CI 已覆盖，本地也要注意）

| 坑 | 症状 | 规避 |
|---|---|---|
| 路径分隔符 | `\` vs `/` | 统一用 `PathBuf::join` |
| CRLF | 行尾测试失败 | `.gitattributes` 强制 LF |
| 动态 import | `ERR_UNSUPPORTED_ESM_URL_SCHEME` | 绝对路径必须过 `pathToFileURL` |

### 测试规范

- 新功能必须带测试（单元或集成）
- golden 测试：输出逐字节 sha256 比对
- fixture 用 `scratch/gen_musicforge_fixtures.py` 生成（**只含自构造内容，不含版权材料**）
- 提交前全量 `cargo test --workspace` 必须 0 fail

### 提交规范

- 遵循 [Conventional Commits](https://www.conventionalcommits.org/)
- 每次提交前 `cargo clippy --workspace -- -D warnings` 零 warning
- **禁止**在 commit message / issue / PR 中出现风险评估、时机讨论等「明知+故意」类表述（合规留痕规范）

## 打包

### 免安装包（zip）

```bash
cargo build --release -p musicforge-cli -p musicforge-gui
# 产物：target/release/musicforge.exe、target/release/musicforge-gui.exe
# 连同 README.md、LICENSE、使用说明.txt 一起打包为
# dist/MusicForge-<version>-windows-x64.zip
```

### Windows 安装包（NSIS）

`installer/musicforge.nsi` 为手写脚本（**不使用 Tauri CLI 的模板**），便于完整审计：
零联网、零文件关联、零开机自启、零 PATH 修改，`RequestExecutionLevel user`（不申请管理员权限）。

编译：

```bat
cd installer
"C:\Program Files (x86)\NSIS\Bin\makensis.exe" /INPUTCHARSET UTF8 musicforge.nsi
:: 产物 installer\MusicForge-<version>-setup.exe
```

脚本与 `license-setup.txt` 均为 **UTF-8 with BOM**（`Unicode true` 下的稳定做法）。

校验闭环（改脚本后必跑）：

```bat
MusicForge-0.1.0-setup.exe /S
"%LOCALAPPDATA%\Programs\MusicForge\Uninstall.exe" /S
```

需确认：安装目录清空、开始菜单目录移除、`HKCU\...\Uninstall\MusicForge` 与 `HKCU\Software\MusicForge` 均删除。

#### 踩坑：静默模式下 MessageBox 会返回 Cancel

NSIS 在 `/S` 静默模式下不显示消息框，且对**含「取消」按钮**的消息框直接按 Cancel 处理。
因此 `un.onInit` / `.onInit` 里的确认框若不加保护，`/S` 时会静默 `Abort`
——表现为 **exit code 0 但一个文件都没删/没装**，极易漏检。

正确写法：

```nsi
IfSilent do_it                    ; 静默 → 跳过确认
MessageBox MB_OKCANCEL "..." IDOK do_it
Abort                             ; 用户取消
do_it:
```

本项目卸载确认统一交给 `MUI_UNPAGE_CONFIRM` 页面，`un.onInit` 不再二次弹窗。
