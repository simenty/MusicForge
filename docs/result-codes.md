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
| `Db` | `MF-DB-FAILED` | `MF-DB-FAILED` | 状态库异常（可再生缓存，删除即重建；勿放网络挂载，D16） |
| `Lossless` | `LOSSLESS-ERROR` | `MF-LOSSLESS-FAILED` | 无损转码失败 |
| `FfmpegMissing` | `FFMPEG-MISSING` | `MF-FFMPEG-MISSING` | 需要 ffmpeg sidecar 但未探测到（P5b） |
| `UpgradeBlocked` | `UPGRADE-BLOCKED` | `MF-LOSSY-TO-LOSSLESS` | 有损→无损升级被拦（MP3→FLAC 等，绝不静默升格） |
| `OutputExists` | `OUTPUT-EXISTS` | `MF-OUTPUT-EXISTS` | 输出已存在（任何策略绝不覆盖目标） |
| `Config` | `MF-CONFIG-INVALID` | `MF-CONFIG-INVALID` | 配置文件损坏 / schema 版本过高→显式拒绝打开 |
| `PluginAckRequired` | `MF-PLUGIN-ACK-REQUIRED` | `MF-PLUGIN-ACK-REQUIRED` | 高风险插件未经 ACK 确认（P6b） |
| `PluginNotFound` | `MF-PLUGIN-NOT-FOUND` | `MF-PLUGIN-NOT-FOUND` | 插件未安装 / 默认构建无 host（绝不静默装作执行过） |
| `PluginDisabled` | `MF-PLUGIN-DISABLED` | `MF-PLUGIN-DISABLED` | 插件已禁用 |

## 契约边界（P9 审计澄清，重要）

**只有上面「映射表」里的码由 `NcmError::mf_code()` 产出**，可被 UI / 日志 / 失败清单
CSV 按码分类。

而下面「规划中的族」里的 `MF-OP-*` / `MF-TASK-*` / `MF-DUP-*` / `MF-PATH-CONFLICT` /
`MF-DIR-NOT-AUTHORIZED` / `MF-ORGANIZE` **不是** `mf_code()` 的返回值：它们只是
**规则标记**——出现在回滚清单的 `rule` 字段、扫描报告的 `unauthorized_dirs` 或 CLI
退出文案里。**按码分类时这些族恒为空**，此前统一标注「✅ 已启用」易被误读成
「可由 `mf_code()` 产出」，故在此明确。

（`MF-DB-FAILED` / `MF-PLUGIN-NOT-FOUND` / `MF-CONFIG-INVALID` 是真正的 `NcmError`
码，已补进上面映射表。）

`p1e_error_codes.rs` 目前只钉死 **14 / 19** 个变体（`Db`、`Lossless`、`FfmpegMissing`、
`UpgradeBlocked`、`OutputExists` 五族尚无断言）——映射表的完整性**不能**只靠该测试保证。

## 规划中的族（随阶段引入）

| 族 | 示例 | 引入阶段 |
|:--|:--|:--|
| `MF-OUTPUT-*` | `MF-OUTPUT-EXISTS` | P2（安全任务层） |
| `MF-PATH-*` | `MF-PATH-CONFLICT` | P2 |
| `MF-ROLLBACK-*` | `MF-ROLLBACK-NOOP` | P2 |
| `MF-FFMPEG-*` | `MF-FFMPEG-MISSING` | P5b（sidecar 探测） |
| `MF-PLUGIN-*` | `MF-PLUGIN-NOT-FOUND` / `MF-PLUGIN-DISABLED` / `MF-PLUGIN-ACK-REQUIRED` / `MF-PLUGIN-PERMISSION-DENIED` / `MF-PLUGIN-TIMEOUT` / `MF-PLUGIN-CRASHED` / `MF-PLUGIN-API-INCOMPATIBLE` | P6a/P6b |
| `MF-FORMAT-*` | `MF-FORMAT-DRM-UNSUPPORTED` | P6b（平台 DRM 只识别不处理，D10-D 级） |
| `MF-DB-*` | `MF-DB-FAILED`（状态库异常：可再生缓存，删除即重建；勿放网络挂载） | P2（D16 状态层）✅ 已启用 |
| `MF-OP-*` | `MF-OP-CONFLICT`（`--dry-run` 与 `--apply` 同时给出）／`MF-OP-NEEDS-YES`（高危操作缺 `--yes`） | P2（安全分级 `safety::resolve`）✅ 已启用 |
| `MF-TASK-*` | — | P2（任务/报告） |
| `MF-DUP-*` | `MF-DUP-EXACT`（exact 内容重复牺牲项，回滚清单 `rule` 字段）／`MF-DUP-SAME-NAME`（同名候选牺牲项，`--include-same-name` 才纳入执行） | P4（去重）✅ 已启用 |
| `MF-PATH-CONFLICT` | organize 冲突策略 `overwrite-never`：目标已存在，该项计失败（任何策略绝不覆盖目标） | P4（整理）✅ 已启用 |
| `MF-DIR-NOT-AUTHORIZED` | 目录未获授权（fnOS/只读挂载场景）：扫描上报未授权清单（`ScanReport.unauthorized_dirs`）+ 授权引导文案；只读授权目录上执行破坏类操作拒绝 | P8（fnOS fpk，X31/X49 代码落地）✅ 扫描上报已启用 |
| `MF-ORGANIZE` | organize 移动项在回滚清单 `rule` 字段的标记（from=新位置 → to=原位置，`clean --restore` 可整体还原） | P4（整理）✅ 已启用 |
| `MF-PLUGIN-NOT-FOUND` | `dedupe --suggest` 在离线版显式报出（AI 保留建议 = v0.7.0 `review_duplicate_group` 插件；绝不静默装作给过建议） | P4 触发 / P6a 修复 ✅ 已启用 |
| `MF-CONFIG-INVALID` | 桌面配置文件（`config.json`）损坏 / schema 高于当前版本 → 显式拒绝打开（对齐 db「降级不猜」语义；配置可再生，删除即以默认重建） | P6a（X36 设置持久化）✅ 已启用 |

## 约定

1. 新增错误码必须先写进本表，再写代码（避免"文档追不上实现"）。
2. 改映射等于破坏下游解析：只能**新增**，不得复用或改写既有码的含义。
3. 失败清单 CSV 的 `code` 列承载稳定码；`reason` 列是给人看的文案。
