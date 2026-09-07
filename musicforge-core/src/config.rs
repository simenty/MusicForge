//! 桌面配置文件（X36）——P6a 设置持久化落地的最小实现。
//!
//! 设计要点：
//!
//! - **X36**：config 落地即含 `"plugins": {"enabled": []}` 空段——v0.7.0 上线时
//!   用户看到的是「解锁」而非「新增陌生入口」；
//! - **schema_version = 1**（工程治理 4.2）；高于当前版本 → 拒绝打开
//!   （对齐 [`crate::db::Db::open`] 的「降级不猜」语义）；
//! - **向后兼容**：未知键忽略（NDJSON/JSON 行级扩展的同一兼容规则）；
//! - 刻意**不引入 serde derive**（`serde_json::Value` 手工构造，
//    对齐 manifest.rs 的依赖面最小约定，依赖政策 §4.1）；
//! - 配置是**可再生**的：损坏 → 显式报 `MF-CONFIG-INVALID`，绝不静默重建伪装成功。

use std::path::{Path, PathBuf};

use crate::error::NcmError;

/// 当前配置 schema 版本。
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
/// 配置文件名（位于 [`crate::db::local_config_dir`] 下）。
pub const CONFIG_FILE_NAME: &str = "config.json";

/// 已启用插件名列表与确认记录（X36 空段约定；P6b 增补 `acked`）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginsConfig {
    /// 已启用插件（`plugin.json` 的 `name`）；空 = 全禁用（默认，离线铁律）
    pub enabled: Vec<String>,
    /// P6b ACK 闸：已显式确认过高风险提示的插件（如格式迁移）；
    /// X35 规则——缺键/缺段默认空，向后兼容
    pub acked: Vec<String>,
}

/// 应用配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub schema_version: u32,
    pub plugins: PluginsConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            plugins: PluginsConfig::default(),
        }
    }
}

impl AppConfig {
    /// 序列化（`plugins.enabled` 恒输出——X36 空段在首写时就可见）。
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": self.schema_version,
            "plugins": {
                "enabled": self.plugins.enabled,
                "acked": self.plugins.acked,
            },
        })
    }

    /// 反序列化：未知键忽略；schema 高于当前 → 显式拒绝；缺段 → 默认值。
    pub fn from_value(v: &serde_json::Value) -> Result<Self, NcmError> {
        let version = v.get("schema_version").and_then(|x| x.as_u64());
        let version = match version {
            Some(n) => u32::try_from(n)
                .map_err(|_| NcmError::Config(format!("config.schema_version 越界: {n}")))?,
            None => CONFIG_SCHEMA_VERSION,
        };
        if version > CONFIG_SCHEMA_VERSION {
            return Err(NcmError::Config(format!(
                "config schema 版本 {version} 高于本程序支持的 {CONFIG_SCHEMA_VERSION}：拒绝打开以免误读新格式（请升级 MusicForge）"
            )));
        }
        let list = |key: &str| -> Vec<String> {
            v.get("plugins")
                .and_then(|p| p.get(key))
                .and_then(|e| e.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(Self {
            schema_version: version,
            plugins: PluginsConfig {
                enabled: list("enabled"),
                acked: list("acked"),
            },
        })
    }

    /// 从指定路径加载：文件缺失 → 默认配置（**不落盘**——写入是显式动作）。
    pub fn load(path: &Path) -> Result<Self, NcmError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(NcmError::Config(format!("读取 {}: {e}", path.display()))),
        };
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| NcmError::Config(format!("解析 {}: {e}", path.display())))?;
        Self::from_value(&v)
    }

    /// 保存到指定路径（原子写：临时文件 + 重命名，绝不写一半）。
    pub fn save(&self, path: &Path) -> Result<(), NcmError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| NcmError::Config(format!("创建 {}: {e}", parent.display())))?;
            }
        }
        let text = serde_json::to_string_pretty(&self.to_value())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)
            .map_err(|e| NcmError::Config(format!("写入 {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| NcmError::Config(format!("重命名 {}: {e}", path.display())))?;
        Ok(())
    }

    /// 默认路径：`local_config_dir()/config.json`。
    pub fn default_path() -> PathBuf {
        crate::db::local_config_dir().join(CONFIG_FILE_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_empty_plugins_enabled_section() {
        // X36：config 首写即含 "plugins": {"enabled": []} 空段
        let v = AppConfig::default().to_value();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["plugins"]["enabled"], serde_json::json!([]));
    }

    #[test]
    fn roundtrip_preserves_enabled_list() {
        let cfg = AppConfig {
            plugins: PluginsConfig {
                enabled: vec!["ai-openai-compatible".into(), "lyrics-online".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let back = AppConfig::from_value(&cfg.to_value()).unwrap();
        assert_eq!(back, cfg);
    }

    /// P6b：acked（ACK 闸确认记录）roundtrip + 缺段默认空（X35 兼容）。
    #[test]
    fn acked_list_roundtrip_and_defaults() {
        let cfg = AppConfig {
            plugins: PluginsConfig {
                enabled: vec!["kwm-migration".into()],
                acked: vec!["kwm-migration".into()],
            },
            ..Default::default()
        };
        let back = AppConfig::from_value(&cfg.to_value()).unwrap();
        assert_eq!(back, cfg);

        // 旧版 config（无 acked 键）→ 默认空，绝不报错
        let legacy = serde_json::json!({ "schema_version": 1, "plugins": { "enabled": ["a"] } });
        let cfg = AppConfig::from_value(&legacy).unwrap();
        assert!(cfg.plugins.acked.is_empty());
        assert_eq!(cfg.plugins.enabled, vec!["a".to_string()]);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let v = serde_json::json!({
            "schema_version": 1,
            "plugins": { "enabled": ["a"], "future_key": 1 },
            "unknown_top_level": true,
        });
        let cfg = AppConfig::from_value(&v).unwrap();
        assert_eq!(cfg.plugins.enabled, vec!["a".to_string()]);
    }

    #[test]
    fn missing_sections_fall_back_to_defaults() {
        let cfg = AppConfig::from_value(&serde_json::json!({})).unwrap();
        assert_eq!(cfg, AppConfig::default());
    }

    #[test]
    fn higher_schema_version_is_rejected() {
        let v = serde_json::json!({ "schema_version": 2, "plugins": { "enabled": [] } });
        let err = AppConfig::from_value(&v).unwrap_err();
        assert_eq!(err.mf_code(), "MF-CONFIG-INVALID");
    }

    #[test]
    fn file_roundtrip_is_atomic_and_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(
            AppConfig::load(&path).unwrap(),
            AppConfig::default(),
            "缺失 → 默认"
        );

        let cfg = AppConfig {
            plugins: PluginsConfig {
                enabled: vec!["cover-online".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.save(&path).unwrap();
        assert!(
            !path.with_extension("json.tmp").exists(),
            "临时文件必须已被重命名"
        );
        assert_eq!(AppConfig::load(&path).unwrap(), cfg);
    }

    #[test]
    fn corrupt_config_fails_loudly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = AppConfig::load(&path).unwrap_err();
        assert_eq!(
            err.mf_code(),
            "MF-CONFIG-INVALID",
            "损坏配置显式失败，绝不静默重建"
        );
    }
}
