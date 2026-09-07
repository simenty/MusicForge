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

---

## 五、P3/P4 新增不变量（2026-09-07 追加）

P3（scan/clean）与 P4（dedupe/organize/playlist/genre）引入的库治理面契约。
与 B/T 系列同级：重构时逐条保持成立，验收方式即测试名。

| # | 不变量 | 验收方式 |
|---|---|---|
| N1 | 扫描器**只读**且剪枝任意层级的 `.musicforge/` 约定目录——工具自身状态（回收站/清单/回滚）绝不进入扫描结果、计划或整理范围 | `musicforge_convention_dir_is_invisible_to_scan`（musicforge-core/tests/p3_scan.rs） |
| N2 | clean/dedupe 牺牲**永不直接删除**：全部移入 `<root>/.musicforge/trash/<task>/`（保留相对结构）+ `rollback.jsonl`（from↔to），`clean --restore` 可整体还原 | `apply_moves_sacrifices_to_trash_and_restores`（p4_dedupe.rs）、`clean_apply_moves_to_trash_and_restore_roundtrip`（p3_cli.rs）、`apply_moves_sacrifices_to_trash_and_restores`（p4_cli.rs） |
| N3 | 破坏性命令默认 dry-run；`--apply` 才执行；apply 结果先行于报告（人读与 JSON 反映真实发生的事） | `clean_defaults_to_dry_run_and_never_moves`、`dedupe_defaults_to_dry_run_and_never_moves`、`dedupe_apply_moves_sacrifices_keeps_best_and_restores`（p4_cli.rs，后者钉死「JSON 分支不得先于执行返回」的历史 bug） |
| N4 | D17 哈希缓存语义：size+mtime 双一致才命中；命中零读取；mtime 不可得永远 miss（占位行）；二扫零重算 | `hash_cache_first_hash_then_all_hits`、`hash_cache_rehashes_only_changed_file`、`hash_cache_is_reused_across_runs`（p3_d17_scan_cache.rs、p4_dedupe.rs）、`scan_state_db_builds_then_reuses_hash_cache`（p3_cli.rs） |
| N5 | dedupe 保留评分**可复算**：权重无损+40/采样率+8（组内最高）/位深+8/标签+10/封面+5/校验+20；平分取路径字典序最小且 reason 明示；两次运行分数与保留项一致 | `score_weights_are_pinned`、`keep_best_reason_is_recomputable`、`same_name_prefers_lossless_and_reports_only`（p4_dedupe.rs） |
| N6 | 同名候选（同目录同 stem 不同内容）默认**仅报告**；`--include-same-name` 才纳入执行；跨目录同名绝不并组 | `same_name_prefers_lossless_and_reports_only`（p4_dedupe.rs） |
| N7 | organize **绝不覆盖目标**（任何冲突策略；apply 前复查目标存在性）；冲突 skip 报告 / suffix 从 (2) 起连续编号 / overwrite-never 计失败 `MF-PATH-CONFLICT` | `conflict_strategies_never_overwrite`（p4_organize.rs） |
| N8 | organize **幂等**：同模板二次规划零移动；suffix 落位形态（含既有 `(N)`）判已在位，绝不再后缀化 | `second_run_is_noop_and_rollback_restores`、`suffix_placed_files_are_in_place_on_second_run`（p4_organize.rs） |
| N9 | organize 渲染与 convert **同源同语义**（sanitize/保留设备名/段长双上限/回退分支）；无标签走 Fallbacks::default；扩展名保留原格式 | `plan_renders_from_embedded_tags`、`fallback_render_without_tags`（p4_organize.rs）、template.rs 既有快照测试 |
| N10 | playlist 导出条目路径必须可解析存在；导入修复**审计不丢行**（unresolved 以 `# FAIL` 注释保留）；时长 ±1s 消歧只在唯一命中时修复 | `export_by_artist_writes_groups_with_valid_entries`、`import_repairs_broken_paths_with_duration_disambiguation`、`import_roundtrip_all_ok`（p4_playlist.rs） |
| N11 | genre 写入 FillMissingOnly：已有 genre **绝不覆盖**；`--replace-all` 升级高危需 `--yes`；无 style/scene 的码块不写空 genre | `genre_write_fill_missing_only_roundtrip`（p4_stylecode.rs）、`genre_replace_all_is_high_risk_needs_yes`（p4_cli.rs） |
| N12 | `dedupe --suggest` 在离线版显式报 `MF-PLUGIN-NOT-FOUND`（绝不伪装给过 AI 建议） | `dedupe_suggest_reports_plugin_not_found`（p4_cli.rs） |
| N13 | GUI `dedupe_apply` 逐条 canonical 校验牺牲路径**必须在曲库目录内**，逃逸显式拒绝；action 携带 canonical 路径（防 strip_prefix 失配退化为自移动） | `dedupe_commands_contract`（musicforge-gui/src-tauri/src/main.rs） |
