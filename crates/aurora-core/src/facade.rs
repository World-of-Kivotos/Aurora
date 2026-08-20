//! 门面结构 [`Aurora`]：持有全局配置、共享 HTTP 客户端与运行环境，组合下层 crate 对外提供粗粒度
//! 异步 API。目标是让 iced 前端与 CLI 只依赖本 crate。
//!
//! 各操作（版本清单、安装、登录、启动、搜索）分散在同名子模块的 `impl Aurora` 块里，本文件只负责
//! 结构定义、构造与共享的内部辅助（目录布局、下载池、并发上下文装配）。远端端点基址以字段持有，
//! 生产走各官方/镜像默认，单元测试用 `with_*` 注入本地 mock。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aurora_base::mirror::MirrorSource;
use aurora_base::retry::RetryPolicy;
use aurora_download::{DownloadConfig, DownloadPool, Downloader, SourcePlan};
use aurora_install::{GameLayout, InstallContext, VERSION_MANIFEST_V2};
use aurora_instance::IsolationPolicy;
use aurora_version::RuntimeContext;

use crate::config::{AuroraConfig, DownloadSourcePolicy, MemorySettings, SourceSpeedCache};
use crate::error::Result;

/// 门面：组合下层 crate 的统一入口。
pub struct Aurora {
    config: AuroraConfig,
    http: reqwest::Client,
    runtime: RuntimeContext,
    data_dir: PathBuf,
    game_dir: PathBuf,
    config_path: PathBuf,
    /// 「自动测速」档的实测结论。放在门面上而不是每次装配下载池新建：结论要跨批次、跨一次装机内的
    /// 多个下载池复用，一次一探等于没缓存。
    speed_cache: Arc<SourceSpeedCache>,
    // 远端端点基址：生产默认，测试注入 mock。
    manifest_url: String,
    modrinth_base: String,
    curseforge_base: String,
    fabric_base: String,
    quilt_base: String,
    java_runtime_url: String,
    launcher_version: semver::Version,
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
            speed_cache: Arc::new(SourceSpeedCache::new()),
            manifest_url,
            modrinth_base: aurora_modplatform::MODRINTH_BASE.to_owned(),
            curseforge_base: aurora_modplatform::CURSEFORGE_BASE.to_owned(),
            fabric_base: "https://meta.fabricmc.net".to_owned(),
            quilt_base: "https://meta.quiltmc.org".to_owned(),
            java_runtime_url: aurora_java::MOJANG_JAVA_RUNTIME_ALL.to_owned(),
            launcher_version: semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(
                |source| crate::error::CoreError::InvalidLauncherVersion {
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    detail: source.to_string(),
                },
            )?,
        })
    }

    /// 只读访问当前配置。
    pub fn config(&self) -> &AuroraConfig {
        &self.config
    }

    /// 可变访问配置，供本 crate 内需要成批改字段的模块使用（如游戏目录列表的增删）。
    /// 不对外公开：外部改配置一律走语义明确的 setter，避免绕过它们附带的重算逻辑
    /// （例如 `set_version_list_source` 要同步重算清单地址）。
    pub(crate) fn config_mut(&mut self) -> &mut AuroraConfig {
        &mut self.config
    }

    /// 当前游戏目录（`.minecraft`）。
    pub fn game_dir(&self) -> &Path {
        &self.game_dir
    }

    /// 数据目录（便携模式下即 exe 所在目录，否则 `%LOCALAPPDATA%\Aurora`）。
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 配置文件是否已经落过盘。
    ///
    /// 用来判定「这是不是第一次启动」：`ConfigStore::load` 在文件缺失时返回默认配置而不创建文件，
    /// 所以只要没人调用过 `save_config`，它就一直不存在。初次设定走完保存一次，此后不再出现。
    pub fn config_saved(&self) -> bool {
        self.config_path.is_file()
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

    /// 设置文件下载源策略（下次装配下载池即生效）。
    ///
    /// 切到「自动测速」时丢弃旧结论：玩家会去动这个设置，通常正是因为当前用的源不好使，
    /// 沿用切换前测出来的排序等于无视他这次操作的意图。
    pub fn set_download_source(&mut self, policy: DownloadSourcePolicy) {
        if policy.measures_speed() {
            self.speed_cache.invalidate();
        }
        self.config.download_source = policy;
    }

    /// 按当前下载源策略测出候选源顺序（`Auto` 档现测或复用缓存，另两档直接返回固定顺序）。
    ///
    /// 装机开始前调一次即可：结论进 [`SourceSpeedCache`]，随后装配的下载池自动用上。三个重下载入口
    /// （[`Aurora::install`]、[`Aurora::install_managed_modpack`]、[`Aurora::sync_managed_modpack`]）
    /// 已在流程头部各 await 一次，本次装机的第一批下载就吃得到实测结论；新增重下载入口时同样要接。
    /// 刻意不在启动器启动时调——探针再轻也是网络请求，不该挡在开机路径上。探测失败不会冒泡成错误，
    /// 只是让顺序退回兜底档。
    pub async fn measure_download_sources(&self) -> Vec<MirrorSource> {
        self.speed_cache
            .measured_order(&self.http, self.config.download_source)
            .await
    }

    /// 设置版本列表源策略，并据此重算版本清单地址。
    ///
    /// 清单地址在构造时按此策略一次性改写为官方或镜像；只改配置字段不会生效，故这里同步重算。
    pub fn set_version_list_source(&mut self, policy: DownloadSourcePolicy) -> Result<()> {
        self.manifest_url = policy.rewrite_primary(VERSION_MANIFEST_V2)?;
        self.config.version_list_source = policy;
        Ok(())
    }

    /// 设置批量下载的文件级并发上限。
    pub fn set_download_concurrency(&mut self, concurrency: usize) {
        self.config.download_concurrency = concurrency;
    }

    /// 设置内存分配（-Xmx / -Xms）。
    pub fn set_memory(&mut self, memory: MemorySettings) {
        self.config.memory = memory;
    }

    /// 设置全局版本隔离档位。
    pub fn set_isolation_policy(&mut self, policy: IsolationPolicy) {
        self.config.isolation_policy = policy;
    }

    /// 设置找不到匹配 Java 时是否自动下载。
    pub fn set_auto_download_java(&mut self, enabled: bool) {
        self.config.auto_download_java = enabled;
    }

    /// 设置缓存目录（None 表示回落默认位置）。
    pub fn set_cache_directory(&mut self, dir: Option<PathBuf>) {
        self.config.cache_directory = dir;
    }

    /// 覆盖整合包最低版本门控使用的桌面启动器产品版本。
    ///
    /// Tauri 外壳应传入自身的 `CARGO_PKG_VERSION`，避免 core crate 与最终产品独立发版后误判。
    pub fn with_launcher_version(mut self, version: &str) -> Result<Self> {
        self.launcher_version = semver::Version::parse(version).map_err(|source| {
            crate::error::CoreError::InvalidLauncherVersion {
                version: version.to_owned(),
                detail: source.to_string(),
            }
        })?;
        Ok(self)
    }

    /// 设置游戏目录并写入配置（区别于仅改运行期字段的 [`Aurora::set_game_dir`]）。
    ///
    /// 同时更新运行期 `game_dir` 与 `config.game_directory`，`save_config` 落盘后下次 `load` 生效；
    /// 而 `set_game_dir` 只改运行期字段，供 CLI 的临时覆盖使用。
    pub fn set_game_directory(&mut self, game_dir: impl Into<PathBuf>) {
        let dir = game_dir.into();
        self.config.game_directory = Some(dir.clone());
        self.game_dir = dir;
    }

    /// 设置当前启动版本 id（`None` 表示清空选择，主页回落到扫描首项）。
    pub fn set_selected_version(&mut self, id: Option<String>) {
        self.config.selected_version = id;
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
        self.spawn_speed_probe_if_stale();
        let config = DownloadConfig {
            sources: self.download_source_plan(),
            ..DownloadConfig::default()
        };
        let downloader = Downloader::new(self.http.clone(), config);
        DownloadPool::new(downloader, self.config.download_concurrency)
    }

    /// 当前下载源方案：`Auto` 档取测速缓存里的实测顺序，缓存为空或过期时取兜底顺序。
    pub(crate) fn download_source_plan(&self) -> SourcePlan {
        SourcePlan::new(self.speed_cache.order_now(self.config.download_source))
    }

    /// 缓存不新鲜时在后台补一轮测速。
    ///
    /// 装配下载池是同步路径（调用点散落在各安装流程里），这里等不了网络，所以本次装配仍按兜底顺序走；
    /// 探针几百毫秒后落地，同一次装机的后续批次与下一次装机就用得上实测顺序了。这条兜底只服务于
    /// 版本列表、单个 mod 这类轻量批次；重下载入口都已在流程头部 await 过
    /// [`Aurora::measure_download_sources`]，走到这里时缓存已是新鲜的，本函数即刻返回。
    ///
    /// 没有 tokio 运行时（从同步上下文调用）时直接跳过：测速是优化，不值得为它 panic。
    fn spawn_speed_probe_if_stale(&self) {
        let policy = self.config.download_source;
        if !policy.measures_speed() || self.speed_cache.is_fresh() {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::debug!("当前无 tokio 运行时，跳过下载源测速");
            return;
        };
        let cache = self.speed_cache.clone();
        let client = self.http.clone();
        // 句柄丢弃即分离，任务照跑；结论写进缓存，无人等它的返回值。
        let _probe = handle.spawn(async move {
            cache.measured_order(&client, policy).await;
        });
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

    pub(crate) fn launcher_version(&self) -> &semver::Version {
        &self.launcher_version
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
            // 测速探针在测试里必须落回本机：生产样本地址打的是 mojang 与 bmclapi，让单元测试去够
            // 真外网等于把当天的网络状况写进断言，离线时还要为每个源各赔一次探针超时。指向一个必然
            // 拒连的回环端口，探测即刻失败并退回兜底顺序，走的正是「一个都没测出来」那条分支；要验
            // 探针本身的测试自行注入 mock 服务器构造缓存。
            speed_cache: Arc::new(SourceSpeedCache::with_probe(
                "http://127.0.0.1:1/probe",
                Arc::new(aurora_download::MirrorResolver),
                std::time::Duration::from_secs(30 * 60),
                std::time::Duration::from_secs(1),
            )),
            manifest_url: VERSION_MANIFEST_V2.to_owned(),
            modrinth_base: aurora_modplatform::MODRINTH_BASE.to_owned(),
            curseforge_base: aurora_modplatform::CURSEFORGE_BASE.to_owned(),
            fabric_base: "https://meta.fabricmc.net".to_owned(),
            quilt_base: "https://meta.quiltmc.org".to_owned(),
            java_runtime_url: aurora_java::MOJANG_JAVA_RUNTIME_ALL.to_owned(),
            launcher_version: semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .expect("core package version must be valid semver"),
        }
    }

    pub(crate) fn with_manifest_url(mut self, url: impl Into<String>) -> Self {
        self.manifest_url = url.into();
        self
    }

    /// 测试观测：取门面持有的测速缓存句柄，供断言「某个时刻结论是否已经落地」。
    pub(crate) fn speed_cache(&self) -> Arc<SourceSpeedCache> {
        self.speed_cache.clone()
    }

    /// 测试注入：把测速探针指向给定样本地址。地址落在本机时两个源解析到同一 URL，于是同一台 mock
    /// 服务器会收到两次探测请求。
    pub(crate) fn with_speed_probe(mut self, sample_url: impl Into<String>) -> Self {
        self.speed_cache = Arc::new(SourceSpeedCache::with_probe(
            sample_url,
            Arc::new(aurora_download::MirrorResolver),
            std::time::Duration::from_secs(30 * 60),
            std::time::Duration::from_secs(3),
        ));
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

    pub(crate) fn with_quilt_base(mut self, url: impl Into<String>) -> Self {
        self.quilt_base = url.into();
        self
    }

    pub(crate) fn with_curseforge_base(mut self, url: impl Into<String>) -> Self {
        self.curseforge_base = url.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn test_aurora() -> Aurora {
        Aurora::for_test(
            AuroraConfig::default(),
            PathBuf::from("/data"),
            PathBuf::from("/data/.minecraft"),
        )
    }

    #[test]
    fn config_setters_write_through() {
        let mut aurora = test_aurora();
        aurora.set_download_source(DownloadSourcePolicy::MirrorFirst);
        aurora.set_download_concurrency(16);
        aurora.set_auto_download_java(false);
        aurora.set_memory(MemorySettings {
            max_mb: 8192,
            min_mb: Some(1024),
            auto: false,
        });
        aurora.set_isolation_policy(IsolationPolicy::All);

        let cfg = aurora.config();
        assert_eq!(cfg.download_source, DownloadSourcePolicy::MirrorFirst);
        assert_eq!(cfg.download_concurrency, 16);
        assert!(!cfg.auto_download_java);
        assert_eq!(cfg.memory.max_mb, 8192);
        assert_eq!(cfg.memory.min_mb, Some(1024));
        assert_eq!(cfg.isolation_policy, IsolationPolicy::All);
    }

    #[test]
    fn set_version_list_source_rederives_manifest_url() {
        let mut aurora = test_aurora();
        aurora
            .set_version_list_source(DownloadSourcePolicy::MirrorFirst)
            .expect("改写清单地址");
        assert_eq!(
            aurora.config().version_list_source,
            DownloadSourcePolicy::MirrorFirst
        );
        // 清单地址应等于按新策略改写的结果；删掉 setter 里的重算行会退回默认地址，此断言即挂。
        let expected = DownloadSourcePolicy::MirrorFirst
            .rewrite_primary(VERSION_MANIFEST_V2)
            .expect("镜像改写");
        assert_eq!(aurora.manifest_url(), expected);
    }

    #[test]
    fn set_game_directory_syncs_runtime_and_config() {
        let mut aurora = test_aurora();
        aurora.set_game_directory(PathBuf::from("/games/mc"));
        assert_eq!(aurora.game_dir(), Path::new("/games/mc"));
        assert_eq!(
            aurora.config().game_directory.as_deref(),
            Some(Path::new("/games/mc"))
        );
    }

    #[test]
    fn set_selected_version_updates_config() {
        let mut aurora = test_aurora();
        assert!(aurora.config().selected_version.is_none());
        aurora.set_selected_version(Some("1.20.1-Forge_47.4.20".to_owned()));
        assert_eq!(
            aurora.config().selected_version.as_deref(),
            Some("1.20.1-Forge_47.4.20")
        );
        // 传 None 清空选择；删掉 setter 里的赋值这两条断言即挂。
        aurora.set_selected_version(None);
        assert!(aurora.config().selected_version.is_none());
    }

    #[test]
    fn download_plan_follows_measured_order() {
        let aurora = test_aurora();
        // 默认配置是自动测速，缓存还是冷的：走兜底顺序（镜像优先）。
        assert_eq!(
            aurora.download_source_plan().sources,
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );

        // 测出官方更快之后，装配出的方案必须跟着翻过来；删掉 download_source_plan 里的缓存查询即挂。
        aurora
            .speed_cache
            .seed(vec![MirrorSource::Official, MirrorSource::BmclApi]);
        assert_eq!(
            aurora.download_source_plan().sources,
            vec![MirrorSource::Official, MirrorSource::BmclApi]
        );
    }

    #[test]
    fn fixed_policy_ignores_measurement() {
        let mut aurora = test_aurora();
        aurora
            .speed_cache
            .seed(vec![MirrorSource::Official, MirrorSource::BmclApi]);
        aurora.set_download_source(DownloadSourcePolicy::MirrorFirst);
        // 玩家钉死镜像优先，实测结论无权推翻它。
        assert_eq!(
            aurora.download_source_plan().sources,
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
    }

    #[test]
    fn switching_to_auto_drops_stale_measurement() {
        let mut aurora = test_aurora();
        aurora
            .speed_cache
            .seed(vec![MirrorSource::Official, MirrorSource::BmclApi]);
        assert!(aurora.speed_cache.is_fresh());

        aurora.set_download_source(DownloadSourcePolicy::Auto);
        assert!(!aurora.speed_cache.is_fresh());
        assert_eq!(
            aurora.download_source_plan().sources,
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
    }

    #[test]
    fn download_pool_assembles_without_a_runtime() {
        // 装配下载池是同步路径，可能落在没有 tokio 运行时的上下文里；测速触发不得因此 panic。
        // 这条测试的断言就是「不 panic」：无运行时下「正确跳过」与「压根没写探测代码」在外部不可区分，
        // 探测确实会发生由 download_pool_spawns_a_background_probe_when_stale 负责证明。
        let aurora = test_aurora();
        let _pool = aurora.download_pool();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn download_pool_spawns_a_background_probe_when_stale() {
        let probe = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(206).set_body_bytes(vec![0u8]))
            .mount(&probe)
            .await;
        let aurora = test_aurora().with_speed_probe(format!("{}/probe", probe.uri()));

        // 有运行时时装配下载池必须补一轮后台探针。删掉 download_pool 里的 spawn_speed_probe_if_stale
        // 调用，缓存永远新鲜不了，这里就会等到超时。
        let _pool = aurora.download_pool();
        let landed = tokio::time::timeout(Duration::from_secs(5), async {
            while !aurora.speed_cache.is_fresh() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(landed.is_ok(), "后台探针未在 5 秒内把结论写进缓存");
        assert_eq!(
            probe.received_requests().await.expect("mock 请求记录").len(),
            2,
            "两个候选源应各被探一次"
        );
    }
}
