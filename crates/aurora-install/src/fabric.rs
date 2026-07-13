//! Fabric 与 Quilt 安装（无需本地安装器）。
//!
//! 两者的 meta 服务同构：`GET {base}/{apiver}/versions/loader/{game}` 列出可用 loader，
//! `GET {base}/{apiver}/versions/loader/{game}/{loader}/profile/json` 直接返回一份可落盘的
//! 版本 JSON（`inheritsFrom` 指向原版，libraries 仅含加载器自身的 maven 简写库）。因此安装 =
//! 取 profile JSON 原样落盘 + 下载它声明的加载器库，无需 Java 子进程。Fabric 走 v2、Quilt 走 v3。

use serde::Deserialize;

use crate::context::InstallContext;
use crate::error::{Error, Result};
use crate::net;
use crate::plan;
use aurora_version::VersionJson;

/// 加载器风味：决定 meta 基址、API 版本段与显示名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderFlavor {
    Fabric,
    Quilt,
}

impl LoaderFlavor {
    /// 显示名（专有名词，保留原文）。
    pub fn display_name(self) -> &'static str {
        match self {
            LoaderFlavor::Fabric => "Fabric",
            LoaderFlavor::Quilt => "Quilt",
        }
    }

    /// 官方 meta 基址（不带尾斜杠）。
    pub fn default_base(self) -> &'static str {
        match self {
            LoaderFlavor::Fabric => "https://meta.fabricmc.net",
            LoaderFlavor::Quilt => "https://meta.quiltmc.org",
        }
    }

    /// meta API 版本段。
    pub fn api_version(self) -> &'static str {
        match self {
            LoaderFlavor::Fabric => "v2",
            LoaderFlavor::Quilt => "v3",
        }
    }
}

/// meta 的 loader 列表条目里我们关心的字段。
#[derive(Debug, Clone, Deserialize)]
pub struct LoaderVersion {
    /// loader 版本号，如 `0.15.11`。
    pub version: String,
    /// 是否为稳定版（Quilt 缺省该字段时按 false）。
    #[serde(default)]
    pub stable: bool,
}

/// meta loader 列表的一个数组元素（只取 `loader` 子对象）。
#[derive(Debug, Clone, Deserialize)]
struct LoaderListEntry {
    loader: LoaderVersion,
}

/// 一次加载器安装的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderSummary {
    /// 合成版本的 id（profile JSON 自带，如 `fabric-loader-0.15.11-1.21`）。
    pub id: String,
    /// 实际安装的 loader 版本号。
    pub loader_version: String,
    /// 下载的加载器库文件数。
    pub libraries: usize,
}

/// Fabric/Quilt 安装器。
pub struct LoaderInstaller<'a> {
    cx: InstallContext<'a>,
    flavor: LoaderFlavor,
    base_url: String,
}

impl<'a> LoaderInstaller<'a> {
    /// 构造 Fabric 安装器。
    pub fn fabric(cx: InstallContext<'a>) -> Self {
        Self::with_flavor(cx, LoaderFlavor::Fabric)
    }

    /// 构造 Quilt 安装器。
    pub fn quilt(cx: InstallContext<'a>) -> Self {
        Self::with_flavor(cx, LoaderFlavor::Quilt)
    }

    /// 指定风味构造，默认走该风味的官方 meta 基址。
    pub fn with_flavor(cx: InstallContext<'a>, flavor: LoaderFlavor) -> Self {
        Self {
            cx,
            flavor,
            base_url: flavor.default_base().to_owned(),
        }
    }

    /// 覆盖 meta 基址（测试指向 mock，或改用镜像）。
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// 列出某游戏版本的全部可用 loader（meta 返回顺序：最新在前）。
    pub async fn list_loaders(&self, game_version: &str) -> Result<Vec<LoaderVersion>> {
        let url = format!(
            "{}/{}/versions/loader/{}",
            self.base_url.trim_end_matches('/'),
            self.flavor.api_version(),
            game_version
        );
        let entries: Vec<LoaderListEntry> =
            net::get_json(self.cx.client, &url, self.cx.policy, "loader 列表").await?;
        Ok(entries.into_iter().map(|e| e.loader).collect())
    }

    /// 选出推荐 loader：优先最新稳定版，无稳定版则取列表首个（meta 已按新→旧排序）。
    pub async fn latest_loader(&self, game_version: &str) -> Result<LoaderVersion> {
        let loaders = self.list_loaders(game_version).await?;
        loaders
            .iter()
            .find(|l| l.stable)
            .or_else(|| loaders.first())
            .cloned()
            .ok_or_else(|| Error::LoaderVersionNotFound {
                loader: self.flavor.display_name(),
                game_version: game_version.to_owned(),
            })
    }

    /// 取指定 game+loader 的 profile 版本 JSON 原始字节。
    pub async fn fetch_profile_json(
        &self,
        game_version: &str,
        loader_version: &str,
    ) -> Result<Vec<u8>> {
        let url = format!(
            "{}/{}/versions/loader/{}/{}/profile/json",
            self.base_url.trim_end_matches('/'),
            self.flavor.api_version(),
            game_version,
            loader_version
        );
        net::get_bytes(self.cx.client, &url, self.cx.policy).await
    }

    /// 安装指定 game 版本上的加载器。`loader_version` 为 None 时自动选推荐版。
    ///
    /// 前提：对应原版已安装（profile JSON 靠 `inheritsFrom` 复用原版本体/资源，本步只补加载器库）。
    pub async fn install(
        &self,
        game_version: &str,
        loader_version: Option<&str>,
    ) -> Result<LoaderSummary> {
        let loader_version = match loader_version {
            Some(v) => v.to_owned(),
            None => self.latest_loader(game_version).await?.version,
        };

        let raw = self.fetch_profile_json(game_version, &loader_version).await?;
        let version = VersionJson::from_json_str(&String::from_utf8_lossy(&raw))?;
        let id = version.id.clone();

        // 原样落盘 profile JSON（保留未建模字段），再下加载器库。
        let json_path = self.cx.layout.version_json(&id);
        aurora_base::fs::atomic_write(&json_path, &raw).await?;

        let tasks = plan::library_tasks(&version, self.cx.runtime, self.cx.layout)?;
        let libraries = self.cx.run_batch(tasks, "加载器库", None).await?;

        Ok(LoaderSummary {
            id,
            loader_version,
            libraries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_base::retry::RetryPolicy;
    use aurora_download::{DownloadPool, Downloader};
    use aurora_version::{OsName, RuntimeContext};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn fast_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 2,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
            multiplier: 2.0,
            jitter: false,
        }
    }

    #[test]
    fn flavor_endpoints() {
        assert_eq!(LoaderFlavor::Fabric.api_version(), "v2");
        assert_eq!(LoaderFlavor::Quilt.api_version(), "v3");
        assert_eq!(LoaderFlavor::Quilt.default_base(), "https://meta.quiltmc.org");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn latest_loader_prefers_stable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/versions/loader/1.21"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"loader":{"version":"0.16.0-beta.1","stable":false}},
                    {"loader":{"version":"0.15.11","stable":true}}]"#,
            ))
            .mount(&server)
            .await;

        let client = aurora_base::http::build_client().unwrap();
        let pool = DownloadPool::new(Downloader::with_defaults(client.clone()), 4);
        let layout = crate::layout::GameLayout::new(std::env::temp_dir());
        let runtime = RuntimeContext::new(OsName::Windows, "x86_64", 64);
        let policy = fast_policy();
        let cx = InstallContext::new(&client, &pool, &layout, &runtime, &policy);

        let installer = LoaderInstaller::fabric(cx).with_base_url(server.uri());
        let chosen = installer.latest_loader("1.21").await.unwrap();
        assert_eq!(chosen.version, "0.15.11", "应跳过 beta 选稳定版");
        assert!(chosen.stable);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_persists_profile_and_downloads_library() {
        let server = MockServer::start().await;
        let base = server.uri();

        // profile JSON：inheritsFrom 原版，含一个走 mock 仓库的加载器库。
        let profile = format!(
            r#"{{
                "id":"fabric-loader-0.15.11-1.21",
                "inheritsFrom":"1.21",
                "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                "libraries":[
                    {{"name":"net.fabricmc:fabric-loader:0.15.11","url":"{base}/"}}
                ]
            }}"#
        );
        Mock::given(method("GET"))
            .and(path("/v2/versions/loader/1.21/0.15.11/profile/json"))
            .respond_with(ResponseTemplate::new(200).set_body_string(profile))
            .mount(&server)
            .await;
        // 加载器库文件本体。
        Mock::given(method("GET"))
            .and(path(
                "/net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fabric-loader-jar".to_vec()))
            .mount(&server)
            .await;

        let client = aurora_base::http::build_client().unwrap();
        let pool = DownloadPool::new(Downloader::with_defaults(client.clone()), 4);
        let dir = tempfile::tempdir().unwrap();
        let layout = crate::layout::GameLayout::new(dir.path());
        let runtime = RuntimeContext::new(OsName::Windows, "x86_64", 64);
        let policy = fast_policy();
        let cx = InstallContext::new(&client, &pool, &layout, &runtime, &policy);

        let installer = LoaderInstaller::fabric(cx).with_base_url(&base);
        let summary = installer.install("1.21", Some("0.15.11")).await.unwrap();

        assert_eq!(summary.id, "fabric-loader-0.15.11-1.21");
        assert_eq!(summary.loader_version, "0.15.11");
        assert_eq!(summary.libraries, 1);

        // profile JSON 原样落盘。
        let json_on_disk =
            std::fs::read(layout.version_json("fabric-loader-0.15.11-1.21")).unwrap();
        assert!(String::from_utf8_lossy(&json_on_disk).contains("KnotClient"));
        // 加载器库落到 libraries 下。
        let lib = layout
            .library_path("net/fabricmc/fabric-loader/0.15.11/fabric-loader-0.15.11.jar");
        assert_eq!(std::fs::read(lib).unwrap(), b"fabric-loader-jar");
    }
}
