# 安全政策

## 支持版本

| 版本 | 支持 |
|---|---|
| latest release | ✅ |
| main branch | ✅ |
| 旧版本 | ❌ |

## 报告漏洞

使用 GitHub [私有漏洞报告](https://github.com/{owner}/MusicForge/security/advisories/new)。

请勿在公开 issue/PR/讨论中包含漏洞详情。

## 范围

- NCM 解析/解密逻辑中的内存安全问题
- 路径穿越（模板注入导致输出逃出预期目录）
- 供应链（依赖完整性）
- **不适用**：本项目零网络依赖、零遥测——攻击面仅限本地文件解析
