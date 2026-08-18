//! 整合包订阅状态持久化。
//!
//! 一个版本目录里存在 `modpack-subscription.json` 即表示该实例由整合包管理。文件只保存整合包标识
//! 与远端指针地址；具体清单解析、同步与快照由 `aurora-modpack` 负责。缺文件表示普通实例，文件损坏
//! 或字段非法则冒泡，不能静默退回普通实例，否则平台更新检查可能擅自改动受管 Mod。

use std::path::{Path, PathBuf};

use aurora_instance::AURORA_META_DIR;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::facade::Aurora;
use crate::modpack::ensure_version_relative_path;

const MODPACK_SUBSCRIPTION_FILE: &str = "modpack-subscription.json";
const INVALID_POINTER_URL: &str = "pointer_url 必须是带主机名的合法 HTTP(S) URL";
const POINTER_URL_WITH_CREDENTIALS: &str = "pointer_url 不能包含用户信息或密码";
const POINTER_URL_WITH_FRAGMENT: &str = "pointer_url 不能包含片段";

/// 实例订阅的整合包与其最新版本指针。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModpackSubscription {
    /// 服务端整合包标识。
    pub pack_id: String,
    /// 最新版本指针地址，只接受带主机名且不含用户信息、密码或片段的 HTTP(S) URL。
    pub pointer_url: String,
}

impl ModpackSubscription {
    fn validate(&self) -> std::result::Result<(), &'static str> {
        if self.pack_id.trim().is_empty() {
            return Err("pack_id 不能为空");
        }

        let url = reqwest::Url::parse(&self.pointer_url).map_err(|_| INVALID_POINTER_URL)?;
        if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
            return Err(INVALID_POINTER_URL);
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || raw_url_authority(&self.pointer_url).is_some_and(|authority| authority.contains('@'))
        {
            return Err(POINTER_URL_WITH_CREDENTIALS);
        }
        if url.fragment().is_some() {
            return Err(POINTER_URL_WITH_FRAGMENT);
        }
        Ok(())
    }
}

fn raw_url_authority(url: &str) -> Option<&str> {
    let (_, after_scheme) = url.split_once("://")?;
    let end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    Some(&after_scheme[..end])
}

/// `modpack-subscription.json` 的读写句柄。
#[derive(Debug, Clone)]
pub(crate) struct ModpackSubscriptionStore {
    path: PathBuf,
}

impl ModpackSubscriptionStore {
    /// 默认路径：`version_dir/.aurora/modpack-subscription.json`。
    fn for_version_dir(version_dir: &Path) -> Self {
        Self {
            path: version_dir
                .join(AURORA_META_DIR)
                .join(MODPACK_SUBSCRIPTION_FILE),
        }
    }

    /// 指定订阅文件路径（测试注入）。
    #[cfg(test)]
    fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 订阅文件路径。
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 读取订阅；文件缺失表示普通实例，存在但损坏或字段非法则冒泡。
    async fn load(&self) -> Result<Option<ModpackSubscription>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                let subscription: ModpackSubscription =
                    serde_json::from_slice(&bytes).map_err(|source| CoreError::ConfigParse {
                        path: self.path.clone(),
                        source,
                    })?;
                subscription.validate().map_err(|reason| {
                    CoreError::InvalidModpackSubscription {
                        path: self.path.clone(),
                        reason,
                    }
                })?;
                Ok(Some(subscription))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(aurora_base::Error::Io {
                path: self.path.clone(),
                source,
            }
            .into()),
        }
    }

    /// 校验并原子写入订阅。
    async fn save(&self, subscription: &ModpackSubscription) -> Result<()> {
        subscription
            .validate()
            .map_err(|reason| CoreError::InvalidModpackSubscription {
                path: self.path.clone(),
                reason,
            })?;
        let bytes = serde_json::to_vec_pretty(subscription).map_err(CoreError::ConfigSerialize)?;
        aurora_base::fs::atomic_write(&self.path, &bytes).await?;
        Ok(())
    }
}

impl Aurora {
    /// 该实例的整合包订阅句柄。
    pub(crate) async fn modpack_subscription_store(
        &self,
        version_id: &str,
    ) -> Result<ModpackSubscriptionStore> {
        let version_dir = self.checked_version_dir(version_id).await?;
        ensure_version_relative_path(
            self.game_dir(),
            &version_dir,
            Path::new(AURORA_META_DIR)
                .join(MODPACK_SUBSCRIPTION_FILE)
                .as_path(),
        )
        .await?;
        Ok(ModpackSubscriptionStore::for_version_dir(&version_dir))
    }

    /// 读取实例订阅；`None` 表示实例不受整合包管理。
    pub async fn modpack_subscription(
        &self,
        version_id: &str,
    ) -> Result<Option<ModpackSubscription>> {
        self.modpack_subscription_store(version_id)
            .await?
            .load()
            .await
    }

    /// 校验并原子写入实例订阅。
    pub async fn set_modpack_subscription(
        &self,
        version_id: &str,
        subscription: &ModpackSubscription,
    ) -> Result<()> {
        self.modpack_subscription_store(version_id)
            .await?
            .save(subscription)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;

    fn valid_subscription() -> ModpackSubscription {
        ModpackSubscription {
            pack_id: "wok".to_owned(),
            pointer_url: "https://api.mcwok.cn/api/v1/pack/latest".to_owned(),
        }
    }

    #[tokio::test]
    async fn missing_file_means_unmanaged_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModpackSubscriptionStore::for_version_dir(tmp.path());

        assert_eq!(
            store.path(),
            tmp.path().join(".aurora").join("modpack-subscription.json")
        );
        assert_eq!(store.load().await.unwrap(), None);
    }

    #[tokio::test]
    async fn round_trip_preserves_subscription_and_json_contract() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ModpackSubscriptionStore::for_version_dir(tmp.path());
        let subscription = valid_subscription();

        store.save(&subscription).await.unwrap();

        assert_eq!(store.load().await.unwrap(), Some(subscription));
        let value: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(store.path()).await.unwrap()).unwrap();
        assert_eq!(value["pack_id"], "wok");
        assert_eq!(
            value["pointer_url"],
            "https://api.mcwok.cn/api/v1/pack/latest"
        );
        assert_eq!(value.as_object().unwrap().len(), 2);

        let files: Vec<_> = std::fs::read_dir(store.path().parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(files, vec!["modpack-subscription.json"]);
    }

    #[tokio::test]
    async fn corrupt_file_bubbles_without_becoming_unmanaged() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("modpack-subscription.json");
        tokio::fs::write(&path, b"{ broken").await.unwrap();
        let store = ModpackSubscriptionStore::at(&path);

        let err = store.load().await.unwrap_err();
        match err {
            CoreError::ConfigParse { path: bad, .. } => assert_eq!(bad, path),
            other => panic!("期望解析错误，得到 {other:?}"),
        }
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"{ broken");
    }

    #[tokio::test]
    async fn load_rejects_invalid_subscription_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let cases = [
            (
                r#"{"pack_id":"   ","pointer_url":"https://example.com/latest"}"#,
                "pack_id 不能为空",
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"ftp://example.com/latest"}"#,
                INVALID_POINTER_URL,
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"not a url"}"#,
                INVALID_POINTER_URL,
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"https://"}"#,
                INVALID_POINTER_URL,
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"https://user@example.com/latest"}"#,
                POINTER_URL_WITH_CREDENTIALS,
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"https://user:secret@example.com/latest"}"#,
                POINTER_URL_WITH_CREDENTIALS,
            ),
            (
                r#"{"pack_id":"wok","pointer_url":"https://@example.com/latest"}"#,
                POINTER_URL_WITH_CREDENTIALS,
            ),
            (
                r##"{"pack_id":"wok","pointer_url":"https://example.com/latest#stable"}"##,
                POINTER_URL_WITH_FRAGMENT,
            ),
        ];

        for (index, (json, expected_reason)) in cases.into_iter().enumerate() {
            let path = tmp.path().join(format!("subscription-{index}.json"));
            tokio::fs::write(&path, json).await.unwrap();
            let err = ModpackSubscriptionStore::at(&path)
                .load()
                .await
                .unwrap_err();
            assert!(matches!(
                err,
                CoreError::InvalidModpackSubscription { path: bad, reason }
                    if bad == path && reason == expected_reason
            ));
        }
    }

    #[test]
    fn validate_accepts_hosted_http_urls_without_credentials_or_fragments() {
        for pointer_url in [
            "http://example.com/latest",
            "https://example.com:8443/latest?channel=stable",
            "https://[::1]/latest",
        ] {
            let subscription = ModpackSubscription {
                pack_id: "wok".to_owned(),
                pointer_url: pointer_url.to_owned(),
            };

            assert_eq!(subscription.validate(), Ok(()), "{pointer_url}");
        }
    }

    #[tokio::test]
    async fn save_rejects_invalid_subscription_without_creating_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("modpack-subscription.json");
        let store = ModpackSubscriptionStore::at(&path);
        let invalid = ModpackSubscription {
            pack_id: "wok".to_owned(),
            pointer_url: "file:///tmp/latest.json".to_owned(),
        };

        let err = store.save(&invalid).await.unwrap_err();
        assert!(matches!(
            err,
            CoreError::InvalidModpackSubscription { path: bad, reason }
                if bad == path && reason == INVALID_POINTER_URL
        ));
        assert!(!tokio::fs::try_exists(&path).await.unwrap());
    }

    #[tokio::test]
    async fn aurora_api_uses_version_meta_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().join(".minecraft");
        let aurora = Aurora::for_test(
            AuroraConfig::default(),
            tmp.path().to_path_buf(),
            mc.clone(),
        );
        let subscription = valid_subscription();

        aurora
            .set_modpack_subscription("1.20.1-fabric", &subscription)
            .await
            .unwrap();

        assert_eq!(
            aurora
                .modpack_subscription_store("1.20.1-fabric")
                .await
                .unwrap()
                .path(),
            mc.join("versions")
                .join("1.20.1-fabric")
                .join(".aurora")
                .join("modpack-subscription.json")
        );
        assert_eq!(
            aurora.modpack_subscription("1.20.1-fabric").await.unwrap(),
            Some(subscription)
        );
    }

    #[tokio::test]
    async fn public_subscription_apis_reject_traversal_without_external_reads_or_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().join(".minecraft");
        let outside = mc.join("outside").join(".aurora");
        tokio::fs::create_dir_all(&outside).await.unwrap();
        let sentinel = outside.join(MODPACK_SUBSCRIPTION_FILE);
        tokio::fs::write(&sentinel, b"sentinel").await.unwrap();
        let aurora = Aurora::for_test(AuroraConfig::default(), tmp.path().to_path_buf(), mc);
        let traversal = "../outside";

        let store_error = aurora
            .modpack_subscription_store(traversal)
            .await
            .unwrap_err();
        assert!(matches!(
            store_error,
            CoreError::UnsafeModpackPath { ref path, .. } if path == traversal
        ));
        let read_error = aurora.modpack_subscription(traversal).await.unwrap_err();
        assert!(matches!(
            read_error,
            CoreError::UnsafeModpackPath { ref path, .. } if path == traversal
        ));
        let write_error = aurora
            .set_modpack_subscription(traversal, &valid_subscription())
            .await
            .unwrap_err();
        assert!(matches!(
            write_error,
            CoreError::UnsafeModpackPath { ref path, .. } if path == traversal
        ));
        let updates_error = aurora.check_updates(traversal).await.unwrap_err();
        assert!(matches!(
            updates_error,
            CoreError::UnsafeModpackPath { ref path, .. } if path == traversal
        ));
        assert_eq!(tokio::fs::read(&sentinel).await.unwrap(), b"sentinel");
    }
}
