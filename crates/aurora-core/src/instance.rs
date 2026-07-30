//! 实例（已安装版本）的用户设置与工作目录解析。
//!
//! 本模块是「装 Mod 写哪个目录」与「启动读哪个目录」的唯一来源：两条链路都必须经
//! [`Aurora::resolve_working_dir_with`]，版本级隔离覆盖在那里统一读取。若各自算各自的，
//! 覆盖只在一侧生效就会让两个目录分叉，玩家遇到的是「装了却不生效」——一种没有任何报错、
//! 全靠自己发现的静默失效。

use aurora_instance::{
    ResolvedIsolation, VERSIONS_DIR, VersionSettings, VersionSettingsStore, discover_versions,
};
use aurora_launch::resolve_game_directory;

use crate::error::{CoreError, Result};
use crate::facade::Aurora;

impl Aurora {
    /// 读取版本级设置：缺文件即全默认（未自定义），文件损坏则冒泡，不静默重置——
    /// 悄悄重置会丢掉用户的收藏、描述与隔离覆盖，比报错更糟。
    pub async fn version_settings(&self, version_id: &str) -> Result<VersionSettings> {
        Ok(self.version_settings_store(version_id).load().await?)
    }

    /// 覆盖写版本级设置（原子写）。
    pub async fn set_version_settings(
        &self,
        version_id: &str,
        settings: &VersionSettings,
    ) -> Result<()> {
        self.version_settings_store(version_id).save(settings).await?;
        Ok(())
    }

    /// 解析某已安装版本的运行工作目录，自行发现版本以取得判定所需事实。
    ///
    /// 版本本地未安装返回 [`CoreError::VersionNotInstalled`]。
    pub async fn resolve_working_dir(&self, version_id: &str) -> Result<ResolvedIsolation> {
        let scan = discover_versions(self.game_dir()).await?;
        let target = scan
            .versions
            .iter()
            .find(|v| v.id == version_id)
            .ok_or_else(|| CoreError::VersionNotInstalled {
                id: version_id.to_owned(),
            })?;
        self.resolve_working_dir_with(version_id, target.has_mod_loader(), target.is_release())
            .await
    }

    /// 同上，但由调用方提供版本事实——启动链路已经扫描过版本，不必再扫一次盘。
    ///
    /// 这是本 crate 唯一调用 [`resolve_game_directory`] 的地方。新增任何「需要知道某版本工作目录」
    /// 的功能都必须走这里，不要另起一次 `resolve_game_directory`，否则隔离覆盖会再次分叉。
    pub(crate) async fn resolve_working_dir_with(
        &self,
        version_id: &str,
        has_mod_loader: bool,
        is_release: bool,
    ) -> Result<ResolvedIsolation> {
        let over = self.version_settings(version_id).await?.isolation;
        Ok(resolve_game_directory(
            self.game_dir(),
            version_id,
            self.config().isolation_policy,
            over,
            has_mod_loader,
            is_release,
        )
        .await?)
    }

    /// 版本设置文件句柄：`versions/<id>/.aurora/settings.json`。
    fn version_settings_store(&self, version_id: &str) -> VersionSettingsStore {
        VersionSettingsStore::for_version_dir(&self.game_dir().join(VERSIONS_DIR).join(version_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use aurora_instance::{IsolationOverride, IsolationPolicy};

    /// 在 versions/<id>/<id>.json 落一份最小合法版本 JSON（正式原版，不装加载器）。
    async fn put_version(mc: &std::path::Path, id: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","type":"release","mainClass":"m"}}"#),
        )
        .await
        .unwrap();
    }

    async fn aurora_at(mc: &std::path::Path, policy: IsolationPolicy) -> Aurora {
        let mut aurora = Aurora::for_test(
            AuroraConfig::default(),
            mc.to_path_buf(),
            mc.to_path_buf(),
        );
        aurora.set_isolation_policy(policy);
        aurora
    }

    /// 全局关闭隔离、版本级强制开启：工作目录必须进版本文件夹。
    /// 把 resolve_working_dir_with 里的 over 换回硬编码 FollowGlobal，此测试即挂。
    #[tokio::test]
    async fn version_override_enabled_isolates_against_global_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.21").await;
        let aurora = aurora_at(mc, IsolationPolicy::Disabled).await;

        // 未设置覆盖时跟随全局：共享 .minecraft 根。
        let before = aurora.resolve_working_dir("1.21").await.unwrap();
        assert!(!before.isolated);
        assert_eq!(before.working_dir, mc.to_path_buf());

        aurora
            .set_version_settings(
                "1.21",
                &VersionSettings {
                    isolation: IsolationOverride::Enabled,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let after = aurora.resolve_working_dir("1.21").await.unwrap();
        assert!(after.isolated);
        assert_eq!(after.working_dir, mc.join("versions").join("1.21"));
    }

    /// 全局全量隔离、版本级强制关闭：工作目录必须退回 .minecraft 根。
    #[tokio::test]
    async fn version_override_disabled_shares_root_against_global_all() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.21").await;
        let aurora = aurora_at(mc, IsolationPolicy::All).await;

        let before = aurora.resolve_working_dir("1.21").await.unwrap();
        assert!(before.isolated);

        aurora
            .set_version_settings(
                "1.21",
                &VersionSettings {
                    isolation: IsolationOverride::Disabled,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let after = aurora.resolve_working_dir("1.21").await.unwrap();
        assert!(!after.isolated);
        assert_eq!(after.working_dir, mc.to_path_buf());
    }

    /// 装 Mod 的目录必须恒等于启动工作目录下的 mods——版本级覆盖翻转后依然成立。
    /// 这条不变量一旦破裂就是「装了却不生效」，故对每种覆盖各验一次。
    #[tokio::test]
    async fn mods_dir_stays_identical_to_launch_working_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.21").await;

        for (policy, over) in [
            (IsolationPolicy::Disabled, IsolationOverride::FollowGlobal),
            (IsolationPolicy::Disabled, IsolationOverride::Enabled),
            (IsolationPolicy::All, IsolationOverride::FollowGlobal),
            (IsolationPolicy::All, IsolationOverride::Disabled),
        ] {
            let aurora = aurora_at(mc, policy).await;
            aurora
                .set_version_settings(
                    "1.21",
                    &VersionSettings {
                        isolation: over,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();

            let launch_dir = aurora.resolve_working_dir("1.21").await.unwrap().working_dir;
            let mods_dir = aurora.resolve_mods_dir("1.21").await.unwrap();
            assert_eq!(
                mods_dir,
                launch_dir.join("mods"),
                "策略 {policy:?} 覆盖 {over:?} 下装载目录与启动目录分叉"
            );
        }
    }

    /// 没有设置文件时行为与修复前一致（跟随全局），保证老实例不受影响。
    #[tokio::test]
    async fn missing_settings_file_follows_global_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.21").await;

        let shared = aurora_at(mc, IsolationPolicy::Disabled).await;
        assert!(!shared.resolve_working_dir("1.21").await.unwrap().isolated);

        let isolated = aurora_at(mc, IsolationPolicy::All).await;
        assert!(isolated.resolve_working_dir("1.21").await.unwrap().isolated);
    }

    /// 未安装的版本：解析工作目录前就冒泡，不去读设置文件。
    #[tokio::test]
    async fn resolve_working_dir_for_uninstalled_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path(), IsolationPolicy::All).await;

        let err = aurora.resolve_working_dir("ghost").await.unwrap_err();
        assert!(matches!(err, CoreError::VersionNotInstalled { id } if id == "ghost"));
    }

    /// 设置往返：写入的描述、收藏与覆盖能原样读回。
    #[tokio::test]
    async fn version_settings_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.21").await;
        let aurora = aurora_at(mc, IsolationPolicy::Disabled).await;

        assert_eq!(
            aurora.version_settings("1.21").await.unwrap(),
            VersionSettings::default()
        );

        let settings = VersionSettings {
            description: Some("生存存档专用".into()),
            favorite: true,
            isolation: IsolationOverride::Enabled,
            ..Default::default()
        };
        aurora.set_version_settings("1.21", &settings).await.unwrap();
        assert_eq!(aurora.version_settings("1.21").await.unwrap(), settings);
    }
}
