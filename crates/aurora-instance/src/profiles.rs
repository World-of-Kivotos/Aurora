//! `launcher_profiles.json` 兼容生成。
//!
//! 官方启动器与部分 Mod（尤其读取正版登录信息的旧式实现）依赖游戏目录根下的 `launcher_profiles.json`。
//! 当它缺失时，为该 `.minecraft` 生成一份含默认 Profile 与随机 `clientToken` 的兼容文件，保证这些
//! 消费者能被满足。已存在则原样保留，绝不覆盖用户/官启的现状。
//!
//! 时间戳用固定占位值（epoch）而非当前时间：避免引入时钟依赖、让生成结果可复现；官启会在实际使用时
//! 自行更新 `lastUsed`。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

const LAUNCHER_PROFILES_FILE: &str = "launcher_profiles.json";
const DEFAULT_PROFILE_KEY: &str = "(Default)";
const LAUNCHER_PROFILES_VERSION: u32 = 3;
/// 固定占位时间戳（epoch）。见模块文档：不引入时钟依赖、保证可复现。
const EPOCH_TIMESTAMP: &str = "1970-01-01T00:00:00.000Z";

/// `launcher_profiles.json` 的顶层结构（保留官启识别所需的关键字段）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherProfiles {
    pub profiles: BTreeMap<String, LauncherProfile>,
    pub settings: LauncherSettings,
    pub version: u32,
    #[serde(rename = "clientToken")]
    pub client_token: String,
}

/// 单个启动 Profile。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherProfile {
    pub name: String,
    #[serde(rename = "type")]
    pub profile_type: String,
    pub created: String,
    #[serde(rename = "lastUsed")]
    pub last_used: String,
    pub icon: String,
    #[serde(rename = "lastVersionId")]
    pub last_version_id: String,
}

/// 官启设置块，字段名与官方文件一致。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherSettings {
    #[serde(rename = "crashAssistance")]
    pub crash_assistance: bool,
    #[serde(rename = "enableAdvanced")]
    pub enable_advanced: bool,
    #[serde(rename = "enableAnalytics")]
    pub enable_analytics: bool,
    #[serde(rename = "enableHistorical")]
    pub enable_historical: bool,
    #[serde(rename = "enableReleases")]
    pub enable_releases: bool,
    #[serde(rename = "enableSnapshots")]
    pub enable_snapshots: bool,
    #[serde(rename = "keepLauncherOpen")]
    pub keep_launcher_open: bool,
    #[serde(rename = "profileSorting")]
    pub profile_sorting: String,
    #[serde(rename = "showGameLog")]
    pub show_game_log: bool,
    #[serde(rename = "showMenu")]
    pub show_menu: bool,
    #[serde(rename = "soundOn")]
    pub sound_on: bool,
}

impl Default for LauncherSettings {
    /// 与官方启动器一致的默认值。
    fn default() -> Self {
        Self {
            crash_assistance: true,
            enable_advanced: false,
            enable_analytics: true,
            enable_historical: false,
            enable_releases: true,
            enable_snapshots: false,
            keep_launcher_open: false,
            profile_sorting: "ByLastPlayed".to_string(),
            show_game_log: false,
            show_menu: false,
            sound_on: false,
        }
    }
}

impl LauncherProfiles {
    /// 用指定 `clientToken` 构造含单个默认 Profile 的兼容配置。
    pub fn with_client_token(client_token: impl Into<String>) -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            DEFAULT_PROFILE_KEY.to_string(),
            LauncherProfile {
                name: DEFAULT_PROFILE_KEY.to_string(),
                profile_type: "custom".to_string(),
                created: EPOCH_TIMESTAMP.to_string(),
                last_used: EPOCH_TIMESTAMP.to_string(),
                icon: "Grass".to_string(),
                last_version_id: "latest-release".to_string(),
            },
        );
        Self {
            profiles,
            settings: LauncherSettings::default(),
            version: LAUNCHER_PROFILES_VERSION,
            client_token: client_token.into(),
        }
    }

    /// 生成一份带随机 `clientToken` 的配置。
    pub fn generate() -> Self {
        Self::with_client_token(generate_client_token())
    }

    /// 序列化为美化 JSON 文本。
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|source| Error::Json {
            context: "launcher_profiles.json",
            path: PathBuf::from(LAUNCHER_PROFILES_FILE),
            source,
        })
    }
}

/// 生成一个无连字符的随机 `clientToken`（32 位十六进制，源自 v4 UUID）。
pub fn generate_client_token() -> String {
    Uuid::new_v4().simple().to_string()
}

/// 缺失时在 `mc_dir` 下生成 `launcher_profiles.json`，返回 `true`；已存在则不动，返回 `false`。
pub async fn ensure_launcher_profiles(mc_dir: &Path) -> Result<bool> {
    let path = mc_dir.join(LAUNCHER_PROFILES_FILE);
    match tokio::fs::metadata(&path).await {
        Ok(_) => Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let profiles = LauncherProfiles::generate();
            let bytes = serde_json::to_vec_pretty(&profiles).map_err(|source| Error::Json {
                context: "launcher_profiles.json",
                path: path.clone(),
                source,
            })?;
            aurora_base::fs::atomic_write(&path, &bytes).await?;
            Ok(true)
        }
        Err(source) => Err(Error::Io { path, source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_client_token_builds_expected_shape() {
        let profiles = LauncherProfiles::with_client_token("deadbeef");
        assert_eq!(profiles.client_token, "deadbeef");
        assert_eq!(profiles.version, 3);

        let value: serde_json::Value = serde_json::to_value(&profiles).unwrap();
        assert_eq!(value["clientToken"], serde_json::json!("deadbeef"));
        assert_eq!(value["version"], serde_json::json!(3));
        // 默认 Profile 存在且键名/字段正确。
        let profile = &value["profiles"]["(Default)"];
        assert_eq!(profile["name"], serde_json::json!("(Default)"));
        assert_eq!(profile["type"], serde_json::json!("custom"));
        assert_eq!(profile["lastVersionId"], serde_json::json!("latest-release"));
        assert_eq!(profile["icon"], serde_json::json!("Grass"));
        // 官启设置键存在。
        assert_eq!(value["settings"]["enableSnapshots"], serde_json::json!(false));
        assert_eq!(
            value["settings"]["profileSorting"],
            serde_json::json!("ByLastPlayed")
        );
    }

    #[test]
    fn generate_client_token_is_32_hex() {
        let token = generate_client_token();
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        // 两次生成应不同（随机）。
        assert_ne!(token, generate_client_token());
    }

    #[test]
    fn profiles_round_trip_through_json() {
        let original = LauncherProfiles::with_client_token("token123");
        let json = original.to_json().unwrap();
        let parsed: LauncherProfiles = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[tokio::test]
    async fn ensure_creates_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        let path = mc.join("launcher_profiles.json");
        assert!(!path.exists());

        let created = ensure_launcher_profiles(mc).await.unwrap();
        assert!(created);
        assert!(path.is_file());

        // 生成的文件可被解析回来，且 clientToken 是 32 位十六进制。
        let bytes = tokio::fs::read(&path).await.unwrap();
        let parsed: LauncherProfiles = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.client_token.len(), 32);
        assert!(parsed.profiles.contains_key("(Default)"));
    }

    #[tokio::test]
    async fn ensure_does_not_overwrite_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        let path = mc.join("launcher_profiles.json");
        // 预置一份用户文件。
        tokio::fs::write(&path, b"{\"userdata\":42}").await.unwrap();

        let created = ensure_launcher_profiles(mc).await.unwrap();
        assert!(!created);
        // 内容原样保留。
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"{\"userdata\":42}");
    }
}
