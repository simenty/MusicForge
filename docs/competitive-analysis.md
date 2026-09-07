# Competitive Analysis（竞争分析）

> 目的：回答「为什么不用 beets / Picard / Music Tag Web / Czkawka」，并为插件路线图提供
> 生态坐标。竞品洞察的完整采纳账目见《平台化方案 v2.7》§1（D23–D26、X22–X23、R22）。
> 最后更新：2026-09-07（数据来源：v2.6 竞品分析输入 + 本项目生态审计）。

## 1. 竞品逐个对照

| 竞品 | 强项 | MusicForge 的差异 | 关系 |
|:--|:--|:--|:--|
| **beets**（Python，标签管理事实标准） | 插件生态最全（封面/歌词/genre/ReplayGain/指纹/转码/查重）；`$artist/$album` 模板语法 | 不碰加密容器（.ncm 等）；无图形化；无任务级 dry-run/trash/rollback 安全模型 | 模板语法与插件清单是 **D25 兼容别名与插件路线图** 的参照系；beets 用户迁移模板零重写 |
| **Picard**（MusicBrainz 官方 tagger） | AcoustID 声纹识别精准（无标签文件也能认曲）；MusicBrainz 元数据质量高 | 单文件交互式为主，没有批量治理/去重/清洗管线；MusicBrainz 对中文流行曲库覆盖弱 | AcoustID 路线被 **X22** 具体化（chromaprint 本地指纹 + AcoustID 查询走 v0.7.x 插件）；MusicBrainz 定位为国际/古典曲库源（**D23**） |
| **Music Tag Web**（中文圈刮削工具） | 中文源刮削方便、界面友好 | **默认口令 admin/admin**（NAS 场景反面教材）；无任务审计与回滚 | 刮削便利性被 cover-online/lyrics-online 吸收；其安全教训固化为 **R22**（首启随机 token 强制） |
| **Czkawka**（Rust，33.3k★ 查重） | 哈希缓存加速二次扫描；多媒介查重 | 只找不修：不会修标签、不会写封面、无回收站语义 | 哈希缓存方向佐证 **D17**；MusicForge 的查重内嵌在治理管线里（评分→回收站→可还原） |
| **unlock-music / anonymous5l/ncmdump 生态** | 加密格式解封装 | 反复删库/转移/DMCA 下架（2021 网易云 3,319 仓库） | 印证 **D10 四级模型 + R14**：高风险格式能力必须独立仓插件化、默认禁用、主仓零真实样本 |
| **auto-ncmdump**（446★） | 监听目录自动转换 | 无治理、无安全分级 | 实证 watcher 需求（**D13** 三级模型，P8） |
| **Lidarr** | 质量画像（Quality Profile）驱动自动升级 | 不做本地加密/去重治理 | 画像思想被 **D24** 吸收（三默认画像，reason 可复算不变） |
| **Navidrome**（23.4k★，Subsonic API） | 自托管音乐服务事实标准，73 客户端生态 | 不是整理器 | LibraryRefresher 首批适配器排序（**X23**：fnOS→Navidrome→Jellyfin/Emby→Plex） |

## 2. MusicForge 的独特位置

**唯一把「解封装 → 治理 → 安全执行 → 可回滚 → NAS 自动化」串成一条管线的项目**：

```text
.ncm 解封装（CRC 校验、显式失败）
  → 曲库治理（扫描 9 规则卡 / 内容去重+可解释评分 / 模板归位 / 歌单 / 风格码 genre）
    → 安全执行（破坏性默认 dry-run；牺牲项全进回收站；rollback.jsonl 整体还原）
      → 可审计（manifest.jsonl + 状态库双写）
        → NAS 自动化（watcher 三级模型 + 播放器重扫，P8）
```

竞品各自覆盖管线上的一两段；没有任何一个同时提供：加密容器支持、任务级 dry-run、
回收站语义、可解释的去重评分、以及「插件永不获得删除/移动/覆盖能力」的安全边界。

## 3. 竞品分析对方案的反向价值

1. **验证既有决策**：加密生态的删库史背书 D10/R14；auto-ncmdump 背书 D13；Czkawka 背书 D17。
2. **具体化远期项**：Picard 的 AcoustID 把 D17-L3 从「远期」落为 v0.7.x 插件（X22）。
3. **插件路线图坐标系**：beets 插件清单 ≈ MusicForge 插件仓的生态地图（三批路线，独立发版）。
4. **安全反面教材**：Music Tag Web 默认口令 → R22（首启随机 token 强制）。
5. **迁移友好**：beets/Music Tag Web 用户的模板配置零重写（D25 别名表）。

> 动态维护：P9（v1.0.0 收敛）时复核一次竞品动态并更新本矩阵（v2.6 采纳项）。
