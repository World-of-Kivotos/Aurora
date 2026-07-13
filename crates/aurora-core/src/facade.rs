//! 门面结构 [`Aurora`]：持有全局配置、共享 HTTP 客户端与运行环境，组合下层 crate 对外提供粗粒度
//! 异步 API。目标是让 iced 前端与 CLI 只依赖本 crate。
//!
//! 各操作（版本清单、安装、登录、启动、搜索）分散在同名子模块的 `impl Aurora` 块里，本文件只负责
//! 结构定义、构造与共享的内部辅助（目录布局、下载池、并发上下文装配）。远端端点基址以字段持有，
//! 生产走各官方/镜像默认，单元测试用 `with_*` 注入本地 mock。

use std::path::{Path, PathBuf};

use aurora_base::retry::RetryPolicy;
use aurora_download::{DownloadConfig, DownloadPool, Downloader};
use aurora_install::{GameLayout, InstallContext, VERSION_MANIFEST_V2};
use aurora_version::RuntimeContext;

use crate::config::AuroraConfig;
use crate::error::Result;

/// 门面：组合下层 crate 的统一入口。
pub struct Aurora {
    config: AuroraConfig,
    http: reqwest::Client,
    runtime: RuntimeContext,
    data_dir: PathBuf,
    game_dir: PathBuf,
    config_path: PathBuf,
    // 远端端点基址：生产默认，测试注入 mock。
    manifest_url: String,
    modrinth_base: String,
    curseforge_base: String,
    fabric_base: String,
    quilt_base: String,
    java_runtime_url: String,
}

impl Aurora {
    /// 以默认数据目录构造：从 `<数据目录>/config.json` 载入配置，构建共享 HTTP 客户端。
    ///
    /// 游戏目录取配置的 `game_directory`，缺省为 `<数据目录>/.minecraft`。
    pub async fn load() -> Result<Self> {
        let data_dir = aurora_base::fs::data_dir()?;
        let config_path = data_dir.join("config.json");
        let config = crate::config::ConfigStore::at(&config_path).load().await?;
        Self::open(config, data_dir, config_path)
    }

    /// 用显式配置与数据目录构造（供 CLI 应用命令行覆盖后调用，或测试注入）。
    pub fn open(config: AuroraConfig, data_dir: PathBuf, config_path: PathBuf) -> Result<Self> {
        let http = aurora_base::http::build_client()?;
        let game_dir = config
            .game_directory
            .clone()
            .unwrap_or_else(|| data_dir.join(aurora_instance::MINECRAFT_DIR_NAME));
        // 版本清单地址随「版本列表源」策略选官方或镜像。
        let manifest_url = config
            .version_list_source
            .rewrite_primary(VERSION_MANIFEST_V2)?;
        Ok(Self {
            config,
            http,
            runtime: RuntimeContext::current(),
            data_dir,
            game_dir,
            config_path,
            manifest_url,
            modrinth_base: aurora_modplatform::MODRINTH_BASE.to_owned(),
            curseforge_base: aurora_modplatform::CURSEFORGE_BASE.to_owned(),
            fabric_base: "https://meta.fabricmc.net".to_owned(),
            quilt_base: "https://meta.quiltmc.org".to_owned(),
            java_runtime_url: aurora_java::MOJANG_JAVA_RUNTIME_ALL.to_owned(),
        })
    }

    /// 只读访问当前配置。
    pub fn config(&self) -> &AuroraConfig {
        &self.config
    }

    /// 当前游戏目录（`.minecraft`）。
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    /// 数据目录（`%LOCALAPPDATA%\Aurora`）。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 覆盖游戏目录（CLI `--game-dir`）。
    pub fn set_game_dir(&mut self, game_dir: impl Into<PathBuf>) {
        self.game_dir = game_dir.into();
    }

    /// 覆盖微软登录 client_id（CLI `--client-id`）。
    pub fn set_client_id(&mut self, client_id: impl Into<String>) {
        self.config.msa_client_id = Some(client_id.into());
    }

    /// 把当前配置写回配置文件。
    pub async fn save_config(&self) -> Result<()> {
        crate::config::ConfigStore::at(&self.config_path)
            .save(&self.config)
            .await
    }

    // ---- 内部共享装配 ----

    /// 共享 HTTP 客户端（克隆廉价：内部 `Arc`）。
    pub(crate) fn http(&self) -> reqwest::Client {
        self.http.clone()
    }

    /// 运行环境快照。
    pub(crate) fn runtime(&self) -> &RuntimeContext {
        &self.runtime
    }

    /// 当前游戏目录的路径布局。
    pub(crate) fn layout(&self) -> GameLayout {
        GameLayout::new(&self.game_dir)
    }

    /// 按当前配置的下载源策略与并发上限装配一个批量下载池。
    pub(crate) fn download_pool(&self) -> DownloadPool {
        let config = DownloadConfig {
            sources: self.config.download_source.source_plan(),
            ..DownloadConfig::default()
        };
        let downloader = Downloader::new(self.http.clone(), config);
        DownloadPool::new(downloader, self.config.download_concurrency)
    }

    /// 元数据抓取的退避重试策略。
    pub(crate) fn retry_policy(&self) -> RetryPolicy {
        RetryPolicy::default()
    }

    /// 版本清单地址（已按版本列表源策略改写）。
    pub(crate) fn manifest_url(&self) -> &str {
        &self.manifest_url
    }

    pub(crate) fn modrinth_base(&self) -> &str {
        &self.modrinth_base
    }

    pub(crate) fn curseforge_base(&self) -> &str {
        &self.curseforge_base
    }

    pub(crate) fn fabric_base(&self) -> &str {
        &self.fabric_base
    }

    pub(crate) fn quilt_base(&self) -> &str {
        &self.quilt_base
    }

    pub(crate) fn java_runtime_url(&self) -> &str {
        &self.java_runtime_url
    }
}

/// 装配一个安装上下文所需的一束借用（生命周期绑定到传入的各组件）。
///
/// 安装/补全类操作先建 `layout`/`pool`/`policy` 局部变量，再借此函数打成 [`InstallContext`]，避免每处
/// 重复五参数样板。
pub(crate) fn make_context<'a>(
    http: &'a reqwest::Client,
    pool: &'a DownloadPool,
    layout: &'a GameLayout,
    runtime: &'a RuntimeContext,
    policy: &'a RetryPolicy,
) -> InstallContext<'a> {
    InstallContext::new(http, pool, layout, runtime, policy)
}

#[cfg(test)]
impl Aurora {
    /// 测试构造：以显式配置、数据目录、游戏目录建门面，端点基址随后用 `with_*` 注入。
    pub(crate) fn for_test(config: AuroraConfig, data_dir: PathBuf, game_dir: PathBuf) -> Self {
        let http = aurora_base::http::build_client().expect("构建测试 HTTP 客户端");
        Self {
            config,
            http,
            runtime: RuntimeContext::new(aurora_version::OsName::Windows, "x86_64", 64),
            data_dir,
            config_path: game_dir.join("config.json"),
            game_dir,
            manifest_url: VERSION_MANIFEST_V2.to_owned(),
            modrinth_base: aurora_modplatform::MODRINTH_BASE.to_owned(),
            curseforge_base: aurora_modplatform::CURSEFORGE_BASE.to_owned(),
            fabric_base: "https://meta.fabricmc.net".to_owned(),
            quilt_base: "https://meta.quiltmc.org".to_owned(),
            java_runtime_url: aurora_java::MOJANG_JAVA_RUNTIME_ALL.to_owned(),
        }
    }

    pub(crate) fn with_manifest_url(mut self, url: impl Into<String>) -> Self {
        self.manifest_url = url.into();
        self
    }

    pub(crate) fn with_modrinth_base(mut self, url: impl Into<String>) -> Self {
        self.modrinth_base = url.into();
        self
    }

    pub(crate) fn with_fabric_base(mut self, url: impl Into<String>) -> Self {
        self.fabric_base = url.into();
        self
    }

    pub(crate) fn with_curseforge_base(mut self, url: impl Into<String>) -> Self {
        self.curseforge_base = url.into();
        self
    }
}
