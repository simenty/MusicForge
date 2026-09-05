# 安全政策

## 支持版本

| 版本 | 支持 |
|---|---|
| latest release | ✅ |
| master branch | ✅ |
| 旧版本 | ❌ |

## 报告漏洞

使用 GitHub [私有漏洞报告](https://github.com/simenty/MusicForge/security/advisories/new)。

请勿在公开 issue/PR/讨论中包含漏洞详情。

### 响应 SLA（承诺）

| 阶段 | 目标时限 |
|:--|:--|
| 确认收到 | ≤ 2 个工作日 |
| 初步评估与严重度分级 | ≤ 7 个工作日 |
| 高危（可致数据丢失/任意写）修复发布 | ≤ 30 天 |
| 中低危修复发布 | ≤ 90 天 |
| 公开披露 | 修复发布后同步，署名致谢报告者（可匿名） |

## 范围

- NCM 解析/解密逻辑中的内存安全问题（解析外部输入，fuzz 覆盖中）
- 路径穿越与模板注入导致输出逃出预期目录
- 插件 Host 隔离失效（权限准入被绕过、跨层文件写入）
- 状态库 `library.db` 的完整性问题
- 供应链（依赖完整性、发布物 SHA256 校验）
- **不适用**：本项目核心零网络依赖、零遥测——攻击面以本地文件解析与插件边界为主

## 相关文档

威胁模型四象限与缓解措施：[docs/threat-model.md](docs/threat-model.md) ·
插件边界：[PLUGIN_POLICY.md](PLUGIN_POLICY.md) · 依赖规则：[docs/dependency-policy.md](docs/dependency-policy.md)
