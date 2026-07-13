//! 版本安装编排：原版补全 + 可选 Mod 加载器（Fabric / Quilt）。
//!
//! 门面把 aurora-install 的原版安装器与加载器安装器串起来，按当前配置的下载源策略与并发跑批量下载，
//! 并在关键节点发出阶段事件。Forge/NeoForge 需要 Java 子进程执行 installer processors，属后续接入项，
//! 本轮加载器安装聚焦无需本地安装器的 Fabric / Quilt。

use aurora_install::{LoaderInstaller, LoaderSummary, VanillaInstaller, VanillaSummary};

use crate::error::Result;
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::{Aurora, make_context};

/// 可安装的 Mod 加载器选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderChoice {
    /// Fabric。
    Fabric,
    /// Quilt。
    Quilt,
}

impl LoaderChoice {
    /// 加载器显示名（专有名词保留原文）。
    pub fn display_name(self) -> &'static str {
        match self {
            LoaderChoice::Fabric => "Fabric",
            LoaderChoice::Quilt => "Quilt",
        }
    }
}

/// 一次安装的结果：原版摘要 + 可选加载器摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// 原版安装摘要。
    pub vanilla: VanillaSummary,
    /// 加载器安装摘要（未装加载器时为 `None`）。
    pub loader: Option<LoaderSummary>,
}

impl Aurora {
    /// 安装指定原版版本，并可选叠加一个 Mod 加载器。
    ///
    /// `loader_version` 仅在指定了 `loader` 时有意义，缺省取该加载器对该游戏版本的推荐版。
    pub async fn install(
        &self,
        id: &str,
        loader: Option<LoaderChoice>,
        loader_version: Option<&str>,
        events: Option<&EventSink>,
    ) -> Result<InstallOutcome> {
        let layout = self.layout();
        let pool = self.download_pool();
        let policy = self.retry_policy();
        let http = self.http();
        let cx = make_context(&http, &pool, &layout, self.runtime(), &policy);

        emit(events, CoreEvent::stage(format!("开始安装原版 {id}")));
        let vanilla = VanillaInstaller::new(cx)
            .with_manifest_url(self.manifest_url())
            .install(id)
            .await?;
        emit(
            events,
            CoreEvent::stage(format!(
                "原版 {} 安装完成：库 {} / 资源 {} / natives {}",
                vanilla.id, vanilla.libraries, vanilla.assets, vanilla.natives
            )),
        );

        let loader = match loader {
            Some(choice) => {
                emit(
                    events,
                    CoreEvent::stage(format!("开始安装 {} 加载器", choice.display_name())),
                );
                let installer = match choice {
                    LoaderChoice::Fabric => {
                        LoaderInstaller::fabric(cx).with_base_url(self.fabric_base())
                    }
                    LoaderChoice::Quilt => {
                        LoaderInstaller::quilt(cx).with_base_url(self.quilt_base())
                    }
                };
                let summary = installer.install(id, loader_version).await?;
                emit(
                    events,
                    CoreEvent::stage(format!(
                        "{} 加载器安装完成：{}（loader {}）",
                        choice.display_name(),
                        summary.id,
                        summary.loader_version
                    )),
                );
                Some(summary)
            }
            None => None,
        };

        Ok(InstallOutcome { vanilla, loader })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 安装一个「空壳」原版：清单指向 mock，版本 JSON 无下载/资源/库，install 只落版本 JSON。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_vanilla_downloads_version_json_and_reports_zero_counts() {
        let server = MockServer::start().await;
        let base = server.uri();

        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"latest":{{"release":"test","snapshot":"test"}},
                    "versions":[{{"id":"test","type":"release","url":"{base}/test.json",
                    "time":"t","releaseTime":"t"}}]}}"#
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/test.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":"test","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
            ))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc.clone())
            .with_manifest_url(format!("{base}/manifest.json"));

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let outcome = aurora.install("test", None, None, Some(&tx)).await.unwrap();

        assert_eq!(outcome.vanilla.id, "test");
        assert_eq!(outcome.vanilla.libraries, 0);
        assert_eq!(outcome.vanilla.assets, 0);
        assert_eq!(outcome.vanilla.natives, 0);
        assert!(outcome.loader.is_none());

        // 版本 JSON 已落盘到 versions/test/test.json。
        let json_path = mc.join("versions").join("test").join("test.json");
        let on_disk = tokio::fs::read_to_string(&json_path).await.unwrap();
        assert!(on_disk.contains("net.minecraft.client.main.Main"));

        // 至少发出「开始」与「完成」两条阶段事件。
        drop(tx);
        let mut stages = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let CoreEvent::Stage(s) = ev {
                stages.push(s);
            }
        }
        assert!(stages.iter().any(|s| s.contains("开始安装原版 test")));
        assert!(stages.iter().any(|s| s.contains("原版 test 安装完成")));
    }

    /// 原版 + Fabric 加载器：门面先装空壳原版，再叠加 Fabric（profile JSON 落盘 + 下加载器库）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_with_fabric_loader_wires_both_stages() {
        let server = MockServer::start().await;
        let base = server.uri();

        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"latest":{{"release":"test","snapshot":"test"}},
                    "versions":[{{"id":"test","type":"release","url":"{base}/test.json",
                    "time":"t","releaseTime":"t"}}]}}"#
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/test.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":"test","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
            ))
            .mount(&server)
            .await;
        // Fabric meta profile JSON：inheritsFrom test，含一个走 mock 仓库的加载器库。
        Mock::given(method("GET"))
            .and(path("/v2/versions/loader/test/0.15.0/profile/json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"id":"fabric-loader-0.15.0-test","inheritsFrom":"test",
                    "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                    "libraries":[{{"name":"net.example:loader:0.15.0","url":"{base}/"}}]}}"#
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/net/example/loader/0.15.0/loader-0.15.0.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"loader-jar".to_vec()))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc.clone())
            .with_manifest_url(format!("{base}/manifest.json"))
            .with_fabric_base(&base);

        let outcome = aurora
            .install("test", Some(LoaderChoice::Fabric), Some("0.15.0"), None)
            .await
            .unwrap();

        assert_eq!(outcome.vanilla.id, "test");
        let loader = outcome.loader.expect("应装上 Fabric 加载器");
        assert_eq!(loader.id, "fabric-loader-0.15.0-test");
        assert_eq!(loader.loader_version, "0.15.0");
        assert_eq!(loader.libraries, 1);

        // 加载器 profile JSON 落盘、加载器库落到 libraries 下。
        let profile = mc
            .join("versions")
            .join("fabric-loader-0.15.0-test")
            .join("fabric-loader-0.15.0-test.json");
        assert!(profile.is_file());
        let lib = mc
            .join("libraries")
            .join("net")
            .join("example")
            .join("loader")
            .join("0.15.0")
            .join("loader-0.15.0.jar");
        assert_eq!(tokio::fs::read(&lib).await.unwrap(), b"loader-jar");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_unknown_version_bubbles_installer_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/manifest.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"latest":{"release":"a","snapshot":"a"},"versions":[]}"#,
            ))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc)
            .with_manifest_url(format!("{}/manifest.json", server.uri()));

        let err = aurora.install("nope", None, None, None).await.unwrap_err();
        // 清单里没有该版本 -> 安装器报 InstallerEntryMissing，经门面冒泡为 Install 变体。
        assert!(matches!(err, crate::error::CoreError::Install(_)));
    }
}
