# 稳定错误码（Result Codes）

MusicForge 的每个失败都携带一个**稳定错误码**：UI、日志、失败清单 CSV 与将来的插件
都按码分类，而不解析自然语言文案（文案会随措辞演进，码不会）。

## 双命名空间

| 命名空间 | 状态 | 用途 |
|:--|:--|:--|
| `NCM-*` | **保留（legacy）** | v0.1.x 既有码。既有脚本与失败清单 CSV 依赖它，**永不删除** |
| `MF-*` | **现行** | 跨格式/插件统一命名空间。新代码（GUI / 报告 / 插件协议）一律用 `MF-*` |

两者由 `NcmError::code()` 与 `NcmError::mf_code()` 同时提供，映射由
`musicforge-core/tests/p1e_error_codes.rs` 钉死。

## 映射表

| `NcmError` 变体 | legacy `NCM-*` | 现行 `MF-*` | 含义 |
|:--|:--|:--|:--|
| `BadMagic` | `NCM-BAD-MAGIC` | `MF-FORMAT-UNSUPPORTED` | 不是本格式（或已损坏），魔数不符 |
| `Truncated` / `LengthOutOfRange` / `BadKeyPrefix` / `BadMetaPrefix` / `BadMusicPrefix` / `EmptyKey` | `NCM-TRUNCATED` / `NCM-STRUCT-INVALID` | `MF-FORMAT-CORRUPT` | 容器结构异常 |
| `CrcMismatch` | `NCM-CRC-MISMATCH` | `MF-FORMAT-CORRUPT` | 头部 CRC32 校验失败，文件损坏（绝不静默产出） |
| `Base64` / `MetadataJson` | `NCM-METADATA-INVALID` | `MF-METADATA-INVALID` | 元数据解码/解析失败 |
| `EmptyAudio` | `NCM-EMPTY-AUDIO` | `MF-FORMAT-EMPTY-AUDIO` | 无音频负载可解 |
| `UnknownFormat` | `NCM-FORMAT-UNKNOWN` | `MF-FORMAT-UNKNOWN` | 三级判定皆不可判，拒绝猜测（硬约束 9） |
| `OutputIntegrity` | `OUT-INTEGRITY` | `MF-OUTPUT-VERIFY-FAILED` | 落盘字节数与预期不符 |
| `Io` | `IO-ERROR` | `MF-IO-FAILED` | 底层 I/O 失败 |
| `TagRead` | `TAG-READ` | `MF-TAG-READ-FAILED` | 输出文件标签读取失败 |
| `TagWrite` | `TAG-WRITE` | `MF-TAG-WRITE-FAILED` | 标签写入失败 |

## 规划中的族（随阶段引入）

| 族 | 示例 | 引入阶段 |
|:--|:--|:--|
| `MF-OUTPUT-*` | `MF-OUTPUT-EXISTS` | P2（安全任务层） |
| `MF-PATH-*` | `MF-PATH-CONFLICT` | P2 |
| `MF-ROLLBACK-*` | `MF-ROLLBACK-NOOP` | P2 |
| `MF-FFMPEG-*` | `MF-FFMPEG-MISSING` | P5b（sidecar 探测） |
| `MF-PLUGIN-*` | `MF-PLUGIN-NOT-FOUND` / `MF-PLUGIN-DISABLED` / `MF-PLUGIN-ACK-REQUIRED` / `MF-PLUGIN-PERMISSION-DENIED` / `MF-PLUGIN-TIMEOUT` / `MF-PLUGIN-CRASHED` / `MF-PLUGIN-API-INCOMPATIBLE` | P6a/P6b |
| `MF-FORMAT-*` | `MF-FORMAT-DRM-UNSUPPORTED` | P6b（平台 DRM 只识别不处理，D10-D 级） |
| `MF-TASK-*` | — | P2（任务/报告） |

## 约定

1. 新增错误码必须先写进本表，再写代码（避免"文档追不上实现"）。
2. 改映射等于破坏下游解析：只能**新增**，不得复用或改写既有码的含义。
3. 失败清单 CSV 的 `code` 列承载稳定码；`reason` 列是给人看的文案。
