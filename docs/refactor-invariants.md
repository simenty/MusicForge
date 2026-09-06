# MusicForge 重构不变量（P1a「绞杀者重构测试保护网」）

> 用途：后续对 MusicForge 做绞杀者重构时，**每一条都必须保持成立**。
> 每条不变量都标注了验收方式（自动测试名 / 手工命令），重构后逐条打勾。
> 约定：新增测试只追加、不改既有断言；下表列出的测试名即验收入口。

---

## 一、行为不变量

| # | 不变量 | 验收方式 |
|---|---|---|
| B1 | 同一 `.ncm` fixture 转换后的音频负载**逐字节确定**（多次解码结果 sha256 一致） | `golden_decode_is_deterministic`（musicforge-core/tests/golden.rs） |
| B2 | 对 5 个自建合规 fixture，输出逐字节等于 Python 参考解码器生成的金标 sha256 | `golden_all_fixtures_byte_exact`（golden.rs） |
| B3 | 标签写入**默认不覆盖已有值**：仅当字段缺失/为空白时才写入；`golden_fixtures_write_tags_reports_match_disk` 保证「报告写了 → 磁盘真有」 | `golden_fixtures_write_tags_reports_match_disk`（musicforge-core/tests/qa_adversarial.rs） |
| B4 | 封面策略不变：有封面数据则嵌入**主标签**；仅 ID3v1 的 MP3 不得把封面写进 ID3v1 后谎报成功（ID3v1 不支持图片、字段上限 30 字符） | `id3v1_only_mp3_must_not_claim_embedded_cover`、`cover_padding_regression`（golden.rs） |
| B5 | 命名模板输出不变：`{artist}/{album}/{track:02d} {title}` 渲染为 `艺术家/专辑/零填充track 标题`；值中的 `/` 被清洗，不得伪造目录 | `full_template_with_dirs`、`value_slash_does_not_create_dirs`（musicforge-core/src/template.rs tests）、`chinese_template_snapshot`（本轮新增） |
| B6 | 同名冲突策略不变：同渲染名追加 ` (n)` 后缀，**不覆盖**；Windows 大小写不敏感（小写键判碰撞） | `batch_dedup_same_dir_collision`（musicforge-cli/tests/batch.rs） |
| B7 | 非法字符清洗结果不变：`<>:"/\|?*` 与控制字符 → `_`；尾部 `. ` 去除；Windows 保留设备名（CON/PRN/AUX/NUL/COM1-9/LPT1-9，大小写不敏感）加 `_` 前缀；**段长上限 = min(100 字符, 200 字节)**（2026-09-06 `dbdf8c5` 起：原「100 字符」在 Linux/macOS 上因组件 255 **字节**上限会 ENAMETOOLONG，故加字节预算；回退分支同样受限） | `sanitizes_illegal_chars`、`sanitizes_control_chars_and_trailing_dots`、`reserved_device_names_prefixed`、`long_name_truncated_by_bytes`、`fallback_stem_is_truncated_too`、`qa_yan_round2::t3_fallback_stem_is_truncated` |
| B8 | 有界并发行为不变：`jobs` 钳制到 `1..=64`；荒谬值不影响正确性 | `absurd_jobs_value_is_bounded_not_fatal`、`batch_bulk_files` |
| B9 | 单文件失败**不中断**批处理：坏文件只影响自己，其余正常完成 | `batch_failure_isolation`、`batch_bulk_files` |
| B10 | 失败清单 CSV 字段兼容：表头恒为 `source,code,reason`，字段顺序与含义不变 | `failures_csv_header_is_stable`（本轮新增）、`sidecar_must_not_contain_absolute_source` |
| B11 | `--skip-existing` 语义不变：仅当输出存在且 sidecar（`.musicforge.json`）记录的 `size` 与 `sha256` **双重**一致才跳过；无标记/标记不符一律重转 | `batch_skip_existing_incremental`、`plan_does_not_leave_residual_output_dir` |
| B12 | 硬失败边界不变：CRC 篡改检出且**无产物**；空音频不产出 0 字节文件 | `crc_tamper_detected_and_no_output`、`empty_audio_must_error_not_zero_byte_output` |
| B13 | 结构保留不变：`-r` 递归时源目录树镜像到输出目录 | `batch_recursive_preserves_structure`、`expanded_inputs_mirror_source_tree` |
| B14 | 符号链接/junction 不跟随（防越界遍历 + 自引用 junction 重复计数） | `collect_inputs_skips_symlinks`（unix-only，CI 提供 Windows 覆盖） |

## 二、API 不变量

| # | 不变量 | 验收方式 |
|---|---|---|
| A1 | `musicforge_core::{crypto,decoder,header,metadata,tagger,template,format}` 的既有公开路径**必须保持可编译**（绞杀者重构若引入 facade，旧路径须经 facade 转发，签名不变） | `cargo test --workspace` 全绿（所有既有测试 import 这些路径；任何破坏性改动即编译失败） |
| A2 | CLI 8 个参数不变：位置参数 `files`、`-d/--directory`、`-r/--recursive`（requires directory）、`-o/--output`、`--skip-existing`、`-j/--jobs`（默认 4）、`--export-failures`、`--template`（默认 `{artist} - {title}`） | `musicforge --help` 输出比对；`cli_contract_bare_invocation_is_convert`（本文件 §三） |
| A3 | 退出码语义不变：有失败 → `1`；全部成功/跳过 → `0`；无任何输入 → `2` | `exit_code_all_success_is_zero`、`exit_code_any_failure_is_nonzero`（本轮新增，musicforge-cli/tests/batch.rs） |
| A4 | `musicforge_cli::{run, run_with_progress, run_with_progress_expanded, BatchConfig, BatchSummary, CancelToken, Status, FileResult}` 公开签名不变 | `cargo test --workspace`（musicforge-cli 既有测试全部直接使用这些 API） |

## 三、CLI 不变量

| # | 不变量 | 验收方式 |
|---|---|---|
| C1 | `musicforge song.ncm`（裸调用，单文件）= convert：输出到源文件同目录，文件名由默认模板渲染 | 手工：`musicforge <fixture>.ncm` 后核对产物；重构后 `--help` 比对 |
| C2 | `musicforge -d <dir> -r -o <out> --template <t> --skip-existing` 组合行为不变（结构保留 + 模板 + 增量跳过） | `batch_recursive_preserves_structure` + `batch_skip_existing_incremental` |
| C3 | `--export-failures <path>` 行为不变：仅含 `Status::Failed` 行；父目录不存在自动创建 | `failures_csv_header_is_stable`（本轮新增） |
| C4 | 输入路径不存在 / 无 `.ncm` 匹配 → stderr 警告，不崩、不改退出码语义 | 手工：`musicforge not_exist.ncm; echo $?` → 0（零结果警告在 stderr） |

## 四、测试不变量

| # | 不变量 |
|---|---|
| T1 | 既有测试**不改代码、不改 fixture、不改期望、不降低断言强度**（`qa_yan_round2.rs`、`qa_yan2_round2.rs` 由 QA 维护，重构者不得触碰） |
| T2 | 新测试只**追加**：文件末尾或既有 `tests` 模块尾部追加，不重排、不删除 |
| T3 | 行为测试优先：断言输出/哈希/路径/退出码/序列化字段名，**禁止**断言私有内部结构（私有函数仅允许在同文件 `#[cfg(test)]` 内测试，且断言的是其**可观察输出**） |
| T4 | 突变验证过的新测试才允许声称「保护网覆盖」；突变实验的临时改动必须立即还原，`git diff` 不得残留 |

---

## 附：已知保护网缺口（重构时需人工盯防）

以下契约**当前没有自动测试钉住**，重构时须人工比对，或待补测试：

1. **Tauri 事件载荷字段名**（`batch-file`：`source/status/output/reason/tagsWritten`；`batch-done`：`ok/skipped/cancelled/failed/durationMs/isCancelled/results`）——载荷在 `musicforge-gui/src-tauri/src/main.rs` 内用 `serde_json::json!` **内联构造**，无独立结构体可测。抽成 struct 才能测（属生产代码改动，超出 P1a 范围）。
2. **需要 `AppHandle` 的 5 个命令**（`start_batch`/`cancel_batch`/`select_ncm_files`/`select_directory`/`save_failures`）无法在纯 `cargo test` 下调用——返回类型以**编译期类型钉子**（`pin_*` 包装函数）钉住，字段级 schema 以 `InputPair`/`BatchArgs`/`FailureRow` 的 serde 测试覆盖。
3. **CRC 校验只由「整文件 CRC 一致性」间接覆盖**：`golden_all_fixtures_byte_exact` 会在 CRC 覆盖范围变化时失败，但没有直接断言「CRC 覆盖 [0, crc_pos)」这一具体语义。
