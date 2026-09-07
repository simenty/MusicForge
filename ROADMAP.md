# MusicForge ROADMAP（定稿基线 v2.4 · 2026-09-05）

> 本文档是 MusicForge 的正式 Roadmap + Architecture Baseline，由九轮输入整合定稿（详见仓库外《平台化方案 v2.4》）。
> **自本版起 scope 冻结**（见 §10）：新能力一律走 `docs/rfc/`，主线只接受 Bug 修复、测试补充与既定 P0–P9 工作。
> 代码基线：`a813cb3`（132 测试函数 / core 直接依赖 6 / CLI 1.04MB / GUI 3.49MB）。
>
> **执行状态（2026-09-07）**：P0 / P0.5 / P1a–e / P2（v0.2.0 已发布）/ **P3（v0.3.0 Scan/Clean）✅ / P4（v0.4.0 Dedupe/Organize/Playlist+genre+similar_cover+GUI 治理面板）✅**，
> 真机实测通过并修复 2 个实测缺陷（约定目录剪枝、organize suffix 幂等）；当前执行位 = **P5a（v0.5.0 纯 Rust 无损）**。
> 实测基线更新：220 测试函数 / CLI ~2.5MB / 依赖 +image（仅 jpeg/png 解码器）。

## 0. 定位

> **MusicForge = 本地优先、可审计、可扩展的开源音乐资产处理平台。**
> 小核心、强测试、离线默认、插件隔离、任务可审计、批量可回滚、桌面/NAS/CLI 三形态统一；高风险格式能力一律进程外可选插件化；账号与云同步为可选自托管层，永不进核心。
>
> **No telemetry. No account. No background upload. No hidden network calls.**

## 1. 三层能力模型（D12）

```text
L1 Core（主仓，零网络）：scan / watcher / metadata / clean / organize / dedupe / playlist / convert / plan / task / report / safety / touch / db
L2 插件仓×2（默认禁用、独立发版独立许可证）：musicforge-plugins（AI/在线）+ musicforge-format-plugins（高风险格式）
L3 云层（独立仓 musicforge-cloud，自托管，v1.x 可选）：账号 / License / 订阅 / 设备绑定 / 配置加密同步
```

**跨层下沉视为架构缺陷，PR 评审直接拒绝。**

## 2. 蓝图能力映射（22 项，v0.9.0 覆盖 19 项）

| # | 能力 | 层 | 版本 | # | 能力 | 层 | 版本 |
|:-:|:--|:-:|:-:|:-:|:--|:-:|:-:|
| 1 | 20+ 格式扫描入库 | L1 | v0.3.0 | 12 | m3u8 歌单导出/导入修复 | L1 | v0.4.0 |
| 2 | 增量监听自动整理 | L1 | v0.9.0 | 13 | 双维去重+可解释评分 | L1 | v0.4.0 |
| 3 | 内嵌标签真写入 | L1 | v0.2.0 | 14 | AI 智能去重建议 | L2 | v0.7.0 |
| 4 | AI 识别/歧义消解 | L2 | v0.7.0 | 15 | 加密格式迁移 | L2 | 不占核心线 |
| 5 | AI 文件名正则 | L2→L1 | v0.7.0 | 16 | mtime 刷新+播放器重扫 | L1+L2 | v0.9.0 |
| 6 | 歌词刮削+双写 | L1+L2 | v0.7.0 | 17 | 桌面三平台 GUI | L1 | v0.8.0 |
| 7 | 封面下载+内嵌双写 | L1+L2 | v0.7.0 | 18 | fnOS/Docker NAS | L1 壳 | v0.9.0 |
| 8 | 重复封面检测 | L1 | v0.4.0 | 19 | Web/PWA 管理 | L1 壳 | v0.9.0 |
| 9 | AI 文生图封面 | L2 | v0.7.x | 20 | 移动/TV 控制台 | L1 壳 | v1.x |
| 10 | AI 歌词核验 | L2 | v0.7.0 | 21 | 账号/License/订阅 | L3 | v1.x |
| 11 | 风格代码分类 | L1+L2 | v0.3.0/v0.7.0 | 22 | 配置云同步/日志脱敏 | L3+L1 | 全程 |

## 3. 决策记录（D1–D20）

| ID | 结论 |
|:-:|:--|
| D1 | 纯 Rust 无损先行（hound/claxon）+ FFmpeg sidecar 五级探测，永不默认捆绑 |
| D2 | 插件=子进程 + stdio **强类型 NDJSON**（`{id,method,params}`→`{id,ok,result\|error{code,message}}`） |
| D3 | NAS Web = `musicforge-server`（axum）+ 复用 React SPA（api.ts 双宿主） |
| D4 | 插件两仓制：`musicforge-plugins`（AI/在线）+ `musicforge-format-plugins`（高风险格式）；api/host/ui-protocol 入主仓 |
| D5 | 插件安全分级 L0–L3；任何等级无删除/移动/覆盖能力 |
| D6 | Release 100% 附 SHA256SUMS.txt；P9 评估 minisig/Authenticode |
| D7 | v0.5.0 纯 Rust 无损 / v0.6.0 FFmpeg sidecar 分开发布 |
| D8 | 宿主编译隔离：`default=[]`，feature 仅 `plugin-host`/`nas`；core/CLI/GUI 永无网络 feature |
| D9 | 重构治理：invariants 文档 + PR 模板 + P1a–e + 等价性测试 + 突变验证；`NCM-FORMAT-UNKNOWN` 保留，新码 `MF-*` |
| D10 | 格式插件四级风险模型：A=NCM 内置；B=QMC / C=Mgg·Mflac=进程外插件（默认禁用+ack）；D=平台 DRM 只识别不处理 |
| D11 | 技术栈全 Rust 化：Rust/Axum/lofty/Tauri/notify/BLAKE3（BLAKE3 仅去重扫描哈希，manifest 完整性=sha256） |
| D12 | 能力三分层，跨层下沉=架构缺陷 |
| D13 | watcher 三级自动化安全模型：T0 登记（默认）/ T1 新文件自动整理 / T2 全自动白名单（永不删/移源文件） |
| D14 | 移动端=远程控制台，v1.x 后 Tauri Mobile |
| D15 | 云层 `cloud` feature 隔离；遥测=0 不可协商 |
| D16 | 状态层=SQLite（rusqlite bundled）`library.db`，与 manifest.jsonl 双写；db=可再生缓存，真相在文件系统；**仅放本地配置目录，严禁网络挂载** |
| D17 | 增量扫描三级指纹：mtime → BLAKE3 仅变化文件 → 音频指纹远期；二次扫描 10 万文件 <10s 入验收 |
| D18 | UI 事件协议正典 crate `musicforge-ui-protocol`（Tauri emit ↔ Axum WS 共用） |
| D19 | 测试三层：合成 fixture 生成器 + proptest + cargo-fuzz（定时跑） |
| D20 | 插件 `api_version` semver 区间 + handshake 协商，不兼容报 `MF-PLUGIN-API-INCOMPATIBLE` |

## 4. 工程治理（13 条）

| § | 层 | 要点 |
|:-:|:--|:--|
| 4.1 | 依赖门禁（P0.5） | deny.toml + cargo-deny/cargo-audit；嵌入式 SQLite 允许，DB 服务端客户端库禁；core 白名单制 |
| 4.2 | 版本化 | Manifest/Config/plugin.json/UI 事件/db 全部 `schema_version`，向后兼容或迁移 |
| 4.3 | 性能预算 | 本地：10k 扫描<10s、100k<120s、Plan<5s、哈希内存≤64MB；网络挂载放宽 6–10×（观测目标）；CI 只跑 3× 裕量冒烟 |
| 4.4 | 插件分级 | L0–L3，铁律：插件永不获得删除/移动/覆盖能力 |
| 4.5 | 发布完整性 | SHA256SUMS 100%；P9 评估签名 |
| 4.6 | 隐私 | `PRIVACY.md`；无遥测/无崩溃上报/无统计 |
| 4.7 | 崩溃安全 | 临时文件+原子 rename；manifest 逐项落盘；`--resume`；kill→resume 逐字节一致 |
| 4.8 | 重构保护网 | 四层：金标（音频负载 sha256）/CLI 行为/模板快照+GUI 契约/依赖网络门禁 + 突变验证 |
| 4.9 | 离线 CI 四层闸 | 依赖树+deny/audit、源码正则、默认构建 feature 断言、断网行为测试（docker --network none） |
| 4.10 | 格式插件准入 | Host 加载强制：api_version 区间匹配 ∧ kind=format-adapter ∧ network=false ∧ delete/move/upload=false，否则拒载；ack 闸 |
| 4.11 | watcher 三级模型 | T0/T1/T2（见 D13）；任何级别 Manifest+Trash+每日上限 |
| 4.12 | 威胁模型 | `docs/threat-model.md`：恶意插件/恶意音频文件/路径注入/模板注入 |
| 4.13 | db 迁移规则 | `PRAGMA user_version`；升级自动迁移、降级拒绝并备份 |

## 5. 阶段计划（P0–P9，每阶段=可发布版本，总量 11–15 周）

| 阶段 | 版本 | 核心内容 | 关键验收 |
|:-:|:-:|:--|:--|
| P0 | v0.1.1 | Release v0.1.0+SHA256SUMS、治理文档七份、CI 四层闸、模板 | release 3 资产可下载校验；CI 全绿 |
| P0.5 | v0.1.2 | deny.toml 依赖/许可证门禁 + 负向突变（加 ureq 被拒）+ 断网行为测试 | cargo-deny 拒绝网络依赖突变 |
| P1a–e | v0.1.x | 绞杀者五步：测试保护→移动+facade→FormatAdapter+等价性→CLI 切 Registry→错误码统一 | **132 测试零改动全绿**；CLI diff=0；负载逐字节=基线 |
| P2 | v0.2.0 | Plan/Manifest/Dry-run 分级/Trash/Rollback/Resume + **db/ 状态层** + CLI 子命令化 + GUI 任务中心 + result-codes 注册表 + 写策略枚举+provenance | kill→resume 逐字节一致；dry-run 零写入；破坏类默认 dry-run |
| P3 | v0.3.0 | scan/clean（12 类垃圾规则+规则卡）、加密格式四级判别、D17 三级指纹、风格代码解析器 | 二次扫描 10 万文件 <10s；污染 fixture 计数断言 |
| P4 | v0.4.0 | dedupe（BLAKE3+aHash 封面+保留评分解释器）+ organize + playlist | 牺牲项全进回收站；评分可复算；`--suggest` 报 MF-PLUGIN-NOT-FOUND |
| P5a | v0.5.0 | 纯 Rust 无损（WAV↔FLAC）+ 合成样本生成器 | 无 ffmpeg 三预设可用；回环逐样本一致 |
| P5b | v0.6.0 | FFmpeg sidecar 有损导出 + 升级转换拦截 | 缺 ffmpeg 报 MF-FFMPEG-MISSING；MP3→FLAC 被拦 |
| P6a+b | v0.7.0 | 插件 Host + AI（3+1 插件）+ 格式插件框架 + demo + D18/D20 ★AI 曲库管家完整形态 | 插件全挂本地五域 100% 可用；请求体无禁发字段；默认构建无 host 符号 |
| P7 | v0.8.0 | macOS/Linux 交付（CLI 先行）+ GUI i18n 中英 | musl 裸容器跑通；cfg 只在壳层 |
| P8 | v0.9.0 | server/Docker/fnOS + watcher 三级自动化 + LibraryRefresher 重扫 + 歌单导入 | `:ro` 破坏类拒绝；镜像 <40MB 零插件；防抖合并 |
| P9 | v1.0.0 | 收敛：SDK 冻结、安全审计、签名评估、**商标/律师函（T1）收敛**、沙箱阶段 3 | 对抗测试全拒；审计无高危；法务书面收敛 |

### P1 五步明细（✅ 已完成，保留作过程记录）

| 子步 | 内容 | 验收 |
|:-:|:--|:--|
| P1a | 测试保护网（不变量文档+四层测试+突变验证），**不移动代码** | ≥5 突变各自击落 ≥1 测试；132+新增全绿 |
| P1b | `git mv` 六文件 + facade 显式 re-export（禁 glob） | 132 零改动全绿；CLI/GUI diff=0 |
| P1c | FormatAdapter+Registry（只定义不切路径）+ 新旧路径等价性测试 | 负载 sha256+元数据逐字段一致 |
| P1d | CLI 内部切 Registry | fixture 全集输出=基线；退出码表一致 |
| P1e | 错误码统一（保留 legacy 码） | 旧文本不变；新码进失败路径 |

## 6. 版本路线

```text
v0.1.x  NCM 稳定（P0/P0.5/P1a–e）          v0.2.0  安全任务层 + db + 双写策略
v0.3.0  Scan/Clean                         v0.4.0  Dedupe/Organize/Playlist
v0.5.0  纯 Rust 无损                        v0.6.0  FFmpeg sidecar
v0.7.0  插件 Host + AI ★曲库管家完整形态     v0.8.0  全平台 + i18n
v0.9.0  NAS                                v1.0.0  收敛
v1.x    musicforge-cloud + 移动控制台
→ 两个插件仓独立发版，不绑定核心版本
```

## 7. 工程指标（节选）

CLI <2MB / GUI <5MB（增幅 <10%/PR）；依赖树网络栈与 GPL 恒 0（硬闸）；测试每阶段净增 20–40；格式路径覆盖率 ≥90%；高风险默认 Dry-run；kill→resume 逐字节一致；二次扫描 10 万文件 <10s；蓝图覆盖率 v0.9.0 ≥19/22。基线：CLI 1.04MB ✅ / GUI 3.49MB ✅ / 依赖 6 ✅。

## 8. 风险登记（R1–R19 节选）

破坏性操作事故（R4：dry-run 默认+回收站+突变测试）、主仓 DMCA 下架（R14：格式插件独立仓+零真实样本）、watcher 误整理（R16：三级模型+每日上限）、**方案无限迭代（R19：本文档冻结，度量切换为测试数与发布物）**。全表见《平台化方案 v2.4》§8。

## 9. 非目标（cut line）

不做播放器；不做云服务端；v1 插件市场不做自动更新；不做标签「自动纠错」（AI 只建议）；移动端只做远程控制台。

## 10. Scope 冻结声明

本 ROADMAP 为定稿基线。1）新能力 → `docs/rfc/`，v1.0 前不并入主线；2）主线只接受 Bug 修复、测试补充与既定 P0–P9 工作；3）进展以「测试函数净增数 / 发布版本 / CI 绿灯」衡量。
