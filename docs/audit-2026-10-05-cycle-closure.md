# MusicForge 审计周期技术收口报告

- 日期：2026-10-05
- 维度：全周期收口（依赖安全 / 性能 PF-1·2·5 / 前端 i18n+A11Y / 播放器队列 PQ-1~6 / 并发清理 / lint 债务 / I18N-7 全链路）
- 审计范围：GUI 后端（`audio.rs` + `commands/*`）、`musicforge-core`、`musicforge-server`、前端 UI（`ui/src`）、CI 门禁（`perf.yml`）
- 结论：**I18N-7 全链路收口完成——数据层/服务端不再嵌中文显示文案；性能回归已落地带余量硬门禁（PF-5）；播放器队列以引擎权威源双向修复（PQ-2）；20 个 lint 警告清零且无回归。全周期审计项全部闭合。**

## 1. 总原则

贯穿整个周期的统一判据：

- **I18N-7（服务端/数据层文本语言中立）**：错误串、展示标签等用户可见文本不硬编码中文；前端 `api.ts`/`transport.ts` 已本地化，后端/core/server 错误与 `stylecode` 标签照 `dedupe::Locale` 模式 locale 化，UI 字符串由前端按语言渲染。
- **性能门禁只对灾难性回归报警**：`perf_baseline` 阈值留足机器间方差余量（预算 ×2 / 内存 ≤85MB），避免 CI 误红。
- **lint 修复依赖稳定成员而非不稳定容器**：避免破坏 `TrackRow` memo（P2-16）或引入 `useEffect` 死循环。

## 2. 已完成项

| # | 项 | commit | 关键改动 | 验证 |
|---|---|---|---|---|
| 1 | **PF-1** 扫描性能 | `43c4335` | `items` 合并改移动（消除克隆副本）+ 哈希回写**分块刷盘**（4096 行/批） | 内存峰值 **46.9MB**（§4.3 预算 64MB，已达标） |
| 2 | **I18N-7·GUI** 后端错误串中立化 | `8920b6c` | `audio.rs` + `commands/*` ~67 处用户可见错误串转英文，保留 `{e}`/`{name}`/`{path}` 等占位符；`main.rs` 三处测试断言同步 | `cargo test`(GUI) 32 passed |
| 3 | **PF-5** 性能回归门禁 | `93b2958` | `perf_baseline` 从「只打印」改**带预算断言**；新增 `perf.yml`（Windows 口径 / 100k，限 push 到 main/master + 手动 + 每夜 cron） | 本地 100k 全 PASS（扫 1.5s / Plan 6.8ms / 热 249ms / 内存 46.9MB） |
| 4 | **PQ-2** 引擎权威队列 | `14c19f6` | 新增 `Cmd::GetQueue` + `PlayerHandle::queue()`（500ms 超时）；`QueueItem` 加 `Serialize`；前端 `resyncQueue` 改**双向**（副本空→从引擎取回权威队列） | 前后端 133 passed（含新增 PQ-2 用例：副本丢失时取回 3 首并采纳，且断言未调 `playQueue`） |
| 5 | **lint** 20 个 exhaustive-deps | `d01ae98` | 解构 `selApi`/`liked`/`w` 的**稳定成员**再依赖；修 AlbumsPage **TDZ**（守卫声明上移）；LibraryPage 保留必要依赖加说明性 disable | eslint 0 problems（20→0）、`tsc` 0、`vitest` 133 passed |
| 6 | **I18N-7·core** 错误串（两批） | `2ddee44` + `b574eff` | core 错误串 + `suggestion()` + 跨行 `format!` 转英文；同步断言 | `cargo test --workspace` 全绿 |
| 7 | **I18N-7·server** HTTP API 文案 | `f953d09` | `api.rs` + `lib.rs` 共 29 处文案转英文（保留 `MF-*` 业务码） | `cargo test -p musicforge-server` 42 passed（含集成 `api_tests` 4） |
| 8 | **I18N-7·stylecode** 标签 locale 化 | `e93195f` | 复用 `dedupe::Locale`，新增 `display_with_locale`；`display_cn` 降为 `Zh` 便捷别名（签名不变） | `i18n_runtime` + `p4_stylecode` 10 passed |

## 3. 关键设计决策

### 3.1 PQ-2：队列取回走请求/响应通道
引擎独占持有 `Engine.queue`，快照只给 `queueLen` 不给队列内容，故前端副本失同步后无法以引擎为准恢复。PQ-3 的 `resyncQueue` 只能「以前端副本重建引擎」，**覆盖不到前端副本本身丢失**的情形。`Cmd::GetQueue { resp: Sender<Vec<QueueItem>> }` 经响应通道取回；`PlayerHandle::queue()` 带 **500ms 超时**——引擎线程退出则显式失败，绝不无限阻塞轮询线程。

### 3.2 exhaustive-deps 正解：依赖稳定成员
实测三个 hook 的返回对象**每次渲染新建（不稳定）**：`useSelection`/`useLiked`/`useWindowedTracks`。若按 eslint 建议补容器对象，会让 `useCallback` 每帧重建、破坏 `TrackRow` memo（P2-16 核心优化）；更危险的是把 `w` 写进 `useEffect` 依赖会触发 **reset → setState → 重渲染死循环**。正解是**解构其中实际用到的稳定成员**（`useCallback([])`）再依赖：稳定成员零性能变化，随数据变化的成员（`isLiked`/`snapshot`）列入反而更正确。`useRequestGuard` 是 `useMemo([],[])`（稳定），可安全入依赖（对齐 StatsPage/PlaylistsPage 既有写法）。

### 3.3 AlbumsPage TDZ
`listGuard` 原声明在依赖它的 `useEffect` **之后**，直接补 `[listGuard]` 会撞 TDZ（`Cannot access before initialization`）。已上移守卫声明（对齐 PlaylistsPage 既有正确写法）。

### 3.4 PF-5 门禁阈值
只对灾难性回归报警，避免 CI 误红：内存 ≤85MB（观测 ~47-59MB，2× 峰值 ~95MB 必触发；§4.3 预算 64MB）；时间预算 ×2 余量（扫描 100k<240s / Plan<10s / 哈希热<2s）。常规 `cargo test` 不设 `MF_PERF_N` 仍早退，不拖慢套件。

### 3.5 Locale 模式
`dedupe::Locale{Zh, En}` 复用到 `stylecode`：数据层零中文标签，渲染时按 UI 语言展开；`display_cn` 保留为 `Zh` 别名以维持旧测试/调用兼容。

## 4. i18n 全链路状态

| 层 | 状态 |
|---|---|
| GUI 后端（`audio.rs` + `commands/*`） | ✅ `8920b6c` |
| `musicforge-core`（错误串 + `suggestion()` + `stylecode` 标签） | ✅ `2ddee44` + `b574eff` + `e93195f` |
| `musicforge-server`（HTTP API 文案） | ✅ `f953d09` |
| 前端 `api.ts` / `transport.ts` 本地化 | ✅ 既有 |
| 数据层 `Locale` 模式（dedupe + stylecode） | ✅ |

## 5. 刻意未动（非遗漏，属独立专项）

- **core 中 `genre` 码值 / codebook**：用户数据，本就语言中立（映射表由用户提供，查不到回退原始码绝不编造）。
- **GUI 风格码卡片渲染**：前端当前未渲染该卡片（功能未接），非 i18n 泄漏；数据层已 locale 就绪，将来按 UI 语言传 `Locale` 即可。
- **PF-1 根本项**（`Vec<ScanItem>` 改轻量记录/路径 arena）：§4.3 内存预算已达标（46.9MB<64MB），属超额优化，非必需，留待后续评估。

## 6. 验证汇总

- 后端：`cargo test`(GUI) 32 passed；`cargo test -p musicforge-server` 42 passed（含集成 `api_tests` 4）。
- 前端：`tsc --noEmit` 0 错误；`eslint src` 0 problems；`vitest` 全量 133 passed。
- 性能：本地 `MF_PERF_N=100000` 全 PASS，门禁无误报；`perf.yml` 已入库。
- `cargo test --workspace` 全绿（core 两批 + 其余）。

> 全部提交已推送至 `master`。工作区干净。
