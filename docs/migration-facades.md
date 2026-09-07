# Facade 迁移指南（D9 时间线：v0.6.0 交付）

> 背景：P1b 绞杀者重构将物理布局迁移为「文件 + 同名目录」，并保留旧路径
> facade（显式 re-export，禁 glob）。本文档是 D9 时间线「v0.6.0 迁移文档」的
> 交付物：**旧路径 → 新路径**的完整映射。
>
> 兼容承诺（不变量 A1）：旧路径在 v1.0 前保持可编译；`NCM-FORMAT-UNKNOWN`
> 稳定码原文永久保留（X7）。

## 1. 模块路径映射

| 旧路径（facade，P1b 前的物理位置） | 新路径（现行物理位置） | 说明 |
|:--|:--|:--|
| `musicforge_core::crypto` | `musicforge_core::formats::ncm::crypto` | RC4/密钥/基址（NCM 域内聚） |
| `musicforge_core::header` | `musicforge_core::formats::ncm::header` | NCM 头解析 + CRC |
| `musicforge_core::decoder` | `musicforge_core::formats::ncm::decoder` | 解封装器 |
| `musicforge_core::format` | `musicforge_core::formats::probe` | 三级格式判定 |
| `musicforge_core::metadata` | `musicforge_core::metadata::model`（结构）/ `metadata::tagger`（写入） | 元数据模型与标签写入 |
| `musicforge_core::tagger` | `musicforge_core::metadata::tagger` | 标签写入（facade 同名转发） |
| `musicforge_core::template` | `musicforge_core::template::engine` | 模板引擎（D25 别名在此层归一） |

**lib.rs 中的 facade 声明**（`pub use`，P1b 落地）：

```rust
pub use formats::ncm::{crypto, decoder, header};
pub use formats::probe as format;
pub use metadata::tagger;
pub use decoder::Decoder;
pub use error::NcmError;
```

## 2. 新代码应该用哪个

| 需求 | 用新路径 |
|:--|:--|
| NCM 解封装 | `musicforge_core::formats::ncm::decoder::Decoder`（或 `FormatRegistry` + `NcmAdapter`，P1d 起 CLI 已走此路） |
| WAV/FLAC 无损转码 | `musicforge_core::lossless`（P5a） |
| CUE 解析/切分 | `musicforge_core::cue`（P5.2；APE/WV/TAK 经 `Ffmpeg` sidecar，P5b） |
| 有损导出 | `musicforge_core::ffmpeg`（P5b.1；`MF-FFMPEG-MISSING` 探测） |
| 标签写入 | `musicforge_core::metadata::tagger`（FillMissingOnly 语义在 tagger 内） |
| 命名模板 | `musicforge_core::template::engine::render_filename`（D25：`$artist`/`%artist%` 别名自动归一） |

## 3. 错误码双命名空间（X7）

- legacy：`NCM-BAD-MAGIC` / `NCM-CRC-MISMATCH` / `NCM-FORMAT-UNKNOWN`（原文永久保留）
- 现行：`MF-*` 全族（`docs/result-codes.md` 注册表）
- 两个命名空间经 `NcmError::code()` / `NcmError::mf_code()` 并存，GUI/报告用 `MF-*`，旧脚本按 `NCM-*` 兼容。

## 4. 时间线与状态

| 时点 | 动作 | 状态 |
|:--|:--|:--|
| v0.3.0 | 旧 facade 标记 `#[deprecated]`（预期） | ⏳ 未打标——QA 保护测试（T1）直接 import facade 路径，打标会触发 clippy `-D warnings` 失败；待 QA 测试迁移到新路径后统一打标（迁移动作本身受 T1 保护，见下） |
| v0.6.0 | 本迁移文档交付 | ✅ |
| v1.0 后 | 评估移除 facade | ⏳ P9 |

**打标的前置条件**（诚实记录）：`tests/qa_yan*.rs`（T1：QA 维护，重构者不得触碰）
等大量测试直接使用 facade 路径——打标会让 `clippy -D warnings` 全红。正确顺序是
「测试先迁新路径 → facade 打标 → 等价性测试守门」，已列入 P9 前置清单。
