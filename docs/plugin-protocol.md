# MusicForge 插件协议 v1.0（冻结版；原 v0.1-draft 审查修订版）

> **文档状态**：v0.1-draft（P6a1 期间可修订；随 P9 协议一致性套件发布升 v1.0 冻结）
> **进仓位置**：`docs/plugin-protocol.md`
> **关联决策**：D2（子进程+stdio）、D4（两仓制）、D5（安全分级）、D8（编译隔离）、D20（semver 协商）
> **关联修正**：X8（NDJSON 正典）、X38（ekey）、X39（三消息模型+握手）、X40（尾部采样）、X41（出站资源）、X42（错误双层）
> **修订记录**：见附录 A（审查发现 9 项 → 修订对照表）

---

## 1. 概述与设计原则

MusicForge 核心（core/CLI/GUI）保持**零网络**；一切需要网络、AI 或法务敏感的能力由插件提供。插件是**独立子进程**，通过 stdio 上的强类型 NDJSON 与 Host（`musicforge-plugin-host`）通信。

五条不可协商原则：

1. **核心零网络**：协议本身不使用任何网络；插件进程才可能联网（且须声明）
2. **AI 只建议不执行**：插件返回 Suggestion/Candidate，写入与删除必经 用户确认 → Plan → Apply
3. **插件永不删移用户文件**：权限模型无 delete/move/upload 位；声明即拒载
4. **最小数据出域**：禁发 `absolute_path`/`audio_bytes`/`cover_bytes`；指纹本地算只传哈希
5. **降级完整**：插件全挂/全禁时，本地五域（扫描/清洗/去重/归档/转换）100% 可用

---

## 2. 传输层

| 项 | 规定 |
|:--|:--|
| 通道 | **stdin/stdout = 协议；stderr = 日志**（Host 捕获 stderr，脱敏后入日志系统） |
| 编码 | NDJSON：一行一条完整 JSON 消息；强制 UTF-8；消息内禁嵌套裸换行 |
| 大小 | 单条消息 ≤ **16 MB**；超限 Host 拒绝并记 `MF-PLUGIN-BAD-REQUEST` |
| 并发 | 协议本身**串行**；并发由 Host 侧进程池实现（多实例），请求 id 进程内唯一 |
| 背压 | Host 串行派发即天然背压；插件不得主动发送非 event 消息 |

---

## 3. 消息模型（三类）

```jsonc
// 请求（Host → Plugin）
{ "id": "req-42", "method": "ai.identify_track", "params": { } }

// 响应（Plugin → Host）——ok=true 带 result；ok=false 带 error
{ "id": "req-42", "ok": true,  "result": { } }
{ "id": "req-42", "ok": false, "error": { "code": "MF-PLUGIN-TIMEOUT", "message": "...", "source_code": null } }

// 事件（Plugin → Host，无 id，不要求响应）
{ "method": "event.progress", "params": { "job_id": "job-1", "percent": 42, "stage": "decrypt" } }
{ "method": "event.log",      "params": { "level": "warn", "message": "..." } }
```

规则：

- `id` 为进程内唯一字符串（UUID/递增均可）；Host 校验响应 id 与请求匹配，不匹配报 `MF-PLUGIN-ID-MISMATCH`
- `result` 成功时不得为空，否则 `MF-PLUGIN-EMPTY-RESULT`
- 插件只能发送 response（对应当前请求）与 event；**event 不得在没有任何进行中请求时发送**（心跳用 `plugin.health` 由 Host 发起）

---

## 4. 生命周期与握手

```text
spawn
  → plugin.init          ★ 首个且必须首个调用（握手）
  → 工作期（request/response/event 混合）
  → plugin.shutdown      （优雅：处理完当前请求后退出；Host 等 5s 后 kill）
  → 空闲 5 min 自动回收（Host 策略）
```

### 4.1 plugin.init（握手，X39）

```jsonc
// Host → Plugin（首个请求）
{ "id": "init", "method": "plugin.init", "params": {
    "protocol_version": 1,
    "work_dir": "/abs/path/to/.musicforge/work/<plugin-id>",
    "locale": "zh-CN",
    "host_capabilities": { "batch": true, "events": true, "artifacts": true }
} }

// Plugin → Host
{ "id": "init", "ok": true, "result": {
    "api_version": ">=1,<2",           // semver 区间（D20）
    "manifest": { /* 见 §7 plugin.json 同构 */ }
} }
```

- Host 校验 `api_version` 区间与自身协议兼容；不兼容报 `MF-PLUGIN-API-INCOMPATIBLE` 并拒绝加载
- 握手失败（首消息非 init 响应/坏 JSON/超时 10s）→ 记 `MF-PLUGIN-BAD-REQUEST` 或 `MF-PLUGIN-TIMEOUT`，进程销毁
- `work_dir` 为插件唯一可写目录（§9 出站资源约定）

### 4.2 崩溃与降级

- 插件进程崩溃/超时：Host 标记任务项 `MF-PLUGIN-CRASHED` / `MF-PLUGIN-TIMEOUT`，允许重试；主进程与任务队列存活
- 同一插件连续 3 次崩溃 → 本次会话禁用并提示用户
- 插件全禁/全挂 → 本地功能 100% 可用（P6a 硬验收）

---

## 5. 方法集

### 5.1 plugin.\*（必备）

| 方法 | 说明 |
|:--|:--|
| `plugin.init` | 握手（§4.1），首个调用 |
| `plugin.manifest` | 返回插件清单（热查询用；与 init 返回一致） |
| `plugin.health` | 返回 `{"status":"ok"}`；Host 心跳/看门狗用 |
| `plugin.shutdown` | 优雅退出 |

### 5.2 ai.\*（ai-provider 插件）

```jsonc
// ai.identify_track —— 单曲识别（最小请求模型：禁发路径/音频/封面字节）
→ params: {
    "normalized_filename": "王铮亮 feat. 风华音纪 - 借墨 [SQ].wav",
    "title": "借墨", "artists": ["王铮亮"], "album": null,
    "duration_ms": 252000, "format": "wav", "language_hint": "zh"
  }
← result: {
    "title": "借墨", "artists": ["王铮亮", "风华音纪"], "album": "借墨",
    "confidence": 0.93,
    "field_confidence": { "title": 0.99, "artists": 0.95, "album": 0.86 },  // 逐字段置信度（X35）
    "reason": "文件名与内嵌标签一致，feat. 结构识别为合作艺人"
  }

// ai.identify_tracks —— 批量变体（可选能力；manifest 声明 "capabilities": {"batch": true}）
// params.tracks 为数组，≤100 条/次；未声明该能力的插件由 Host 退化为循环单条

// ai.generate_filename_regex —— 输入文件名样例 → 返回解析正则文本（规则交回 core 执行，执行权永不在插件）
→ params: { "samples": ["01. 周杰伦 - 晴天 [FLAC].flac", "..."] }
← result: { "regex": "...", "confidence": 0.88, "reason": "..." }

// ai.review_duplicate_group —— 重复组保留建议（建议带 reason，决定权在用户）
→ params: { "candidates": [ { "role": "keep|remove-candidate", "format": "flac",
             "bitrate": 1411, "sample_rate": 44100, "bit_depth": 16,
             "tag_completeness": 0.9, "has_cover": true, "has_lyrics": true } ] }
← result: { "keep_index": 0, "confidence": 0.91, "reason": "无损+标签完整+含封面歌词" }
```

置信度语义统一：`0.0–1.0` 浮点 + `reason` 必填；三段阈值并入 Plan（X13）：≥0.95 标记「可自动」仍需 Plan 确认 / 0.8–0.95 待确认 / <0.8 仅展示。

### 5.3 lyrics.\* / cover.\*

```jsonc
// lyrics.search
→ params: { "title": "...", "artists": ["..."], "album": "...", "duration_ms": 252000, "language_hint": "zh" }
← result: { "candidates": [ { "provider": "...", "score": 0.94,
             "synced": true, "lrc": "[00:00.00] ..." } ] }
//   歌词文本直接内嵌 JSON（≤1MB）；禁止返回 URL 让 Host 下载（P2：内容必须经协议回传）

// lyrics.verify —— 核验歌词与歌曲匹配；红线：绝不改歌手/歌名
// 【2026-09-09 裁决（X47）】：本节早先草案中的 `replacement_candidate` 字段**删除**——
// 与红线「绝不改歌手/歌名」直接冲突（红线从字面，无例外）。现实现
// （LyricsVerifyResult{verdict/confidence/candidates}）为契约形状，P6a-R 已按此对齐。
← result: { "verdict": "match", "confidence": 0.97, "candidates": ["来源描述（非替换项）"] }

// cover.search —— 候选经出站资源回传（§9）
← result: { "candidates": [ { "provider": "...", "score": 0.92, "width": 1000, "height": 1000,
             "artifact": "cover-01.jpg" } ] }   // 相对 work_dir 路径

// cover.generate（AI 文生图，来源优先级最末 D22；同样走出站资源）
← result: { "artifact": "generated-cover.png", "width": 1024, "height": 1024, "confidence": 0.8 }
```

### 5.4 format.\*（format-adapter 插件，高风险域）

```jsonc
// format.probe —— X40：首 4KB + 尾 4KB 双采样契约（STag/QTag 等尾标在文件尾部）
→ params: { "file_name": "song.mflac", "extension": ".mflac", "size_bytes": 48213311,
            "header_hex": "66 4C ...", "tail_hex": "... 53 54 61 67" }
← result: { "supported": true, "format_id": "mflac0-stag", "estimated_output": "flac",
            "confidence": 0.98, "requires_acknowledgement": true, "risk_level": "high",
            "requires_ekey": true,                                  // X38
            "ekey_hint": "需从你自己的 QQ 音乐客户端提取，见文档" }      // X38

// format.validate —— 完整性校验（损坏文件明确报错，绝不静默产出损坏音频）

// format.migrate —— X38：options 增可选 ekey；长任务须发 event.progress（X39）
→ params: { "job_id": "job-1", "input_path": "/music/in/song.mflac",
            "output_path": "/music/.musicforge/work/job-1/out.flac",
            "work_dir": "/music/.musicforge/work/job-1",
            "options": { "preserve_source": true, "overwrite": false,
                         "verify_output": true, "ekey": null } }
← result: { "status": "success", "output_format": "flac",
            "artifacts": ["out.flac"],                              // 相对 work_dir（X41）
            "metadata": { "title": "...", "artists": ["..."], "album": "..." },
            "warnings": [] }
← 失败示例（业务错误透传，X42）：
  { "ok": false, "error": { "code": "MF-PLUGIN-FAILED",
    "message": "ekey 校验失败", "source_code": "QMC-EKEY-INVALID" } }

// format.cancel —— P1-1 语义：插件清理自己在 work_dir 的临时文件后响应 cancelled；
//   Host 等 5s 无响应则 kill 进程兜底，残留由 Host 启动清理流程处理
```

---

## 6. 事件（X39）

| 事件 | params | 规则 |
|:--|:--|:--|
| `event.progress` | `{ job_id, percent: 0-100, stage }` | 长任务（预计 >5s）**必须**周期发送；仅在存在进行中请求时可发 |
| `event.log` | `{ level: debug\|info\|warn\|error, message }` | 协议级日志补充；大量日志仍走 stderr |

GUI 任务中心经 ui-protocol（D18）消费 progress；watcher 计划进度与插件任务进度同一 schema 渲染（v2.8 P8 增补）。

---

## 7. 权限模型与 plugin.json

```jsonc
{
  "api_version": ">=1,<2",                     // D20 semver 区间（handshake 协商）
  "id": "ai-openai-compatible",
  "name": "OpenAI Compatible AI Provider",
  "version": "0.1.0",
  "kind": "ai-provider",                        // ai-provider | lyrics-provider | cover-provider | format-adapter | nas-adapter | notification
  "risk_level": "low",                          // low | medium | high（format-adapter 恒为 high）
  "default_enabled": false,
  "user_acknowledgement_required": false,       // format-adapter 恒为 true（D10）
  "capabilities": { "batch": true },            // 可选能力声明（P1-4）
  "supported_inputs": [],                       // format-adapter 必填（扩展名列表）
  "permissions": {
    "network": true,
    "read_audio_metadata": true,
    "read_audio_file": false,                   // true 仅 format-adapter 可申请
    "write_tags": true,
    "delete_source_file": false,                // 恒 false；true 即拒载（§4.10 准入）
    "move_source_file": false,                  // 恒 false；true 即拒载
    "upload_audio": false                       // 恒 false；true 即拒载
  },
  "network_endpoints": ["https://api.example.com"],
  "data_sent":     ["title", "artists", "album", "duration_ms", "normalized_filename"],
  "data_not_sent": ["audio_binary", "cover_binary", "absolute_path", "full_lyrics"]
}
```

**安全分级**（D5）：L0 纯算法 / L1 元数据 / L2 内容 / L3 系统级默认禁止。铁律：任何等级插件永不获得删除/移动/覆盖文件能力。

**网络位的诚实边界**（P1-5，写入 PLUGIN_POLICY）：进程隔离下 `network:false` 是「约定 + 审计 + 打包审查」级约束，而非沙箱级强制；真正的网络强制依赖 P9 沙箱阶段 3/4（seccomp/容器）。不向用户虚假承诺。

---

## 8. 错误模型（X42 双层）

| 层 | 命名空间 | 示例 | 处理 |
|:--|:--|:--|:--|
| 传输/协议层 | `MF-PLUGIN-*` | BAD-REQUEST / BAD-PARAMS / METHOD-NOT-FOUND / ID-MISMATCH / STDIN-READ-FAILED / EMPTY-RESULT / TIMEOUT / CRASHED / API-INCOMPATIBLE / NOT-FOUND / DISABLED / ACK-REQUIRED / PERMISSION-DENIED | Host 生成；入 `docs/result-codes.md` 注册表 |
| 插件业务层 | 插件自有（建议 `PREFIX-*`） | `QMC-EKEY-INVALID`、`AI-RATE-LIMITED` | 插件返回 → Host 包装为 `MF-PLUGIN-FAILED` + `source_code` 透传 → UI 展示原码与引导 |

---

## 9. 出站资源约定（X41）

二进制产物（图片、音频中间产物）**永不进管道**：

1. 插件只能写入握手时指定的 `work_dir`
2. 响应 `artifacts` 字段返回**相对路径**数组
3. Host resolve 后校验仍在 work_dir 内（拒绝 `../`、绝对路径、符号链接逃逸）再读取
4. Host 取走后按 Plan 落位（标签内嵌/输出目录/quarantine），work_dir 随任务结束清理

对抗断言（P6a1）：返回 `../x` / `/etc/x` / 符号链接逃逸全部被拒。

---

## 10. 安全规则汇总

| 规则 | 强制点 |
|:--|:--|
| 插件路径白名单（`~/.local/share/musicforge/plugins/`、`%LOCALAPPDATA%\MusicForge\plugins\`、NAS appdata/plugins） | Host 加载前 |
| manifest 准入清单（§4.10：api_version 区间 ∧ kind 合法 ∧ network/delete/move/upload 位校验） | Host 加载时，任一不满足**拒载** |
| ack 闸（`user_acknowledgement_required=true` 的插件未经 `plugins acknowledge` 不得 migrate） | Host 转发前 |
| 禁发字段过滤（absolute_path/audio_bytes/cover_bytes） | Host 序列化前静态校验；多余字段**拒绝**（fail-closed）并记违规日志 |
| 所有路径参数 Host 校验在授权目录/work_dir 内 | Host 转发前 |
| 消息 ≤16MB / UTF-8 / 单行 | Host 解析时 |
| 超时矩阵：init 10s / identify 10–30s / migrate 可配（默认 10min）/ shutdown 5s | Host |
| 插件连续崩溃 3 次 → 会话内禁用 | Host |

---

## 11. Rust 类型参考（musicforge-plugin-api 节选）

```rust
#[derive(Debug, Deserialize)]
pub struct PluginRequest { pub id: String, pub method: String,
    #[serde(default)] pub params: serde_json::Value }

#[derive(Debug, Serialize)]
pub struct PluginResponse<T> { pub id: String, pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")] pub error: Option<PluginError> }

#[derive(Debug, Serialize, Deserialize)]
pub struct PluginError { pub code: String, pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub source_code: Option<String> }  // X42

#[derive(Debug, Deserialize)]
pub struct ProbeParams { pub file_name: String, pub extension: String,
    pub size_bytes: u64, pub header_hex: String, pub tail_hex: String }                 // X40

#[derive(Debug, Deserialize)]
pub struct MigrateOptions { pub preserve_source: bool, pub overwrite: bool,
    pub verify_output: bool, #[serde(default)] pub ekey: Option<String> }               // X38

#[derive(Debug, Serialize)]
pub struct Suggestion { pub confidence: f32, pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_confidence: Option<std::collections::HashMap<String, f32>>,               // X35
    #[serde(flatten)] pub fields: serde_json::Value }

#[derive(Debug, Serialize)]
pub struct ProgressEvent { pub job_id: String, pub percent: u8, pub stage: String }     // X39
```

---

## 12. 测试要求

| 层 | 内容 |
|:--|:--|
| mock 插件 | `mf-format-demo` + echo AI 插件，随 plugin-host `examples/` 交付；覆盖握手/manifest/probe/migrate/progress/cancel/shutdown 全方法 |
| 契约测试 | 协议 schema golden 测试（各方法请求/响应样例快照） |
| 对抗测试 | 坏 JSON、超 16MB、id 不匹配、未 init 直接请求、乱发 event、只写 stderr 不下线、永不响应、返回逃逸路径、声明 delete 权限（拒载断言） |
| fuzz | 协议解析器（cargo-fuzz，夜间跑不阻塞 PR，X17） |
| 一致性套件 | P9：第三方插件通过套件即视为协议兼容；套件随插件 SDK 发布 |

---

## 13. 版本治理

- 本文档 = **v1.0-frozen**（2026-09-10 P9 先行冻结；原 v0.1-draft）
- **P9 冻结声明**：协议一致性套件已发布——`musicforge-plugin-api/tests/protocol_consistency.rs`
  （15 测试钉死：协议常量 / 16 方法集 / 事件名 / 5 稳定错误码 / 三类信封 serde 键集 /
  plugin.json 9 字段键集 / kind 值域 / 三禁位语义）。**任何契约变更必须先改本套件
  （红 → 显式修订 → 套件同步）**，杜绝静默漂移。
- 冻结后变更：新增可选字段=向后兼容（不 bump）；破坏性格式变更=bump major 并经 RFC
  （major bump = `codes::HOST_API_MAJOR` 同步 + 双插件仓协同发版 R26）

---

## 附录 A：修订记录（审查发现 → 修订对照）

| # | 级别 | 审查发现 | 修订 | 章节 |
|:-:|:--|:--|:--|:-:|
| P0-1 | P0 | 无事件/进度通道，大文件 migrate 时 UI 假死 | 新增 event 消息类（progress/log） | §3/§6 |
| P0-2 | P0 | 二进制产物无出站机制 | 出站资源约定（work_dir 相对路径 + Host 校验） | §9 |
| P0-3 | P0 | probe 仅 header_hex，尾标变体（STag/QTag）误判 | tail_hex 首 4KB+尾 4KB 采样契约 | §5.4 |
| P0-4 | P0 | 无握手，协议演进无协商点 | plugin.init 双向握手（首个调用） | §4.1 |
| P1-1 | P1 | cancel 语义未定义 | 清理责任 + 5s 超时 kill + Host 兜底 | §5.4 |
| P1-2 | P1 | stderr 被丢弃 | stdout=协议 / stderr=日志通道 | §2 |
| P1-3 | P1 | 业务错误码与传输错误码混同 | 双层模型 + source_code 透传 | §8 |
| P1-4 | P1 | 无批量方法 | ai.identify_tracks 可选能力 + Host 退化 | §5.2 |
| P1-5 | P1 | 网络位权限过度承诺 | PLUGIN_POLICY 诚实化（约定+审计级，沙箱级待 P9） | §7 |
| P2-x | P2 | id 规则/16MB 上限/进程池/禁 URL/空闲回收 | 明确化 | §2/§3/§10 |

## 附录 B：与 MCP 的关系

MCP 的 stdio transport（客户端启动子进程、标准流通信）作为设计参考；本协议**不实现 JSON-RPC 2.0 信封**（X8：精简 NDJSON + `ok` 布尔 + 稳定错误码为契约核心），不兼容 MCP 工具生态亦无需兼容。
