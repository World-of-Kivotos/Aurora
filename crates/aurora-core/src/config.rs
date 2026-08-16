//! 全局配置（config.json）与下载源三档策略。
//!
//! 门面持有一份可读写的全局配置：下载源与版本列表源各自的三档策略（对应功能矩阵「文件下载源 /
//! 版本列表源」两项独立三档设置）、下载并发、默认内存、版本隔离档位、微软 client_id、可选自定义
//! 游戏/缓存目录。凭据不进这里（归 aurora-auth 的加密存储）。
//!
//! 配置以 JSON 落在数据目录（`%LOCALAPPDATA%\Aurora\config.json`）。缺失时用全默认值，损坏则冒泡
//! 报错而非静默用默认覆盖（避免用户改坏的配置被无声吞掉）。

use std::path::{Path, PathBuf};

use aurora_base::mirror::{self, MirrorSource};
use aurora_download::SourcePlan;
use aurora_instance::IsolationPolicy;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// 下载源三档策略。对应功能矩阵的「文件下载源 / 版本列表源」三档设置。
///
/// 静态候选顺序：`OfficialFirst`/`Auto` 官方优先、镜像兜底；`MirrorFirst` 镜像优先、官方兜底。
/// `Auto` 语义为「自动测速」——其静态顺序同官方优先，运行期可用 [`SourcePlan::reorder_by_speed`]
/// 按实测时延重排（门面未在每次下载前强制测速，以免为小文件引入探测延迟）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSourcePolicy {
    /// 自动测速（默认）：静态顺序官方优先，可运行期测速重排。
    #[default]
    Auto,
    /// 尽量官方：官方优先，镜像兜底。
    OfficialFirst,
    /// 尽量镜像：BMCLAPI 镜像优先，官方兜底。
    MirrorFirst,
}

impl DownloadSourcePolicy {
    /// 该策略下按优先级排列的镜像源列表。
    pub fn mirror_order(self) -> Vec<MirrorSource> {
        match self {
            DownloadSourcePolicy::Auto | DownloadSourcePolicy::OfficialFirst => {
                vec![MirrorSource::Official, MirrorSource::BmclApi]
            }
            DownloadSourcePolicy::MirrorFirst => {
                vec![MirrorSource::BmclApi, MirrorSource::Official]
            }
        }
    }

    /// 该策略的首选源（用于不走 [`SourcePlan`] 换源的单源抓取，如 Java 运行时清单）。
    pub fn primary_mirror(self) -> MirrorSource {
        match self {
            DownloadSourcePolicy::MirrorFirst => MirrorSource::BmclApi,
            DownloadSourcePolicy::Auto | DownloadSourcePolicy::OfficialFirst => MirrorSource::Official,
        }
    }

    /// 构造该策略对应的下载源调度方案。
    pub fn source_plan(self) -> SourcePlan {
        SourcePlan::new(self.mirror_order())
    }

    /// 把一个官方 URL 按该策略的首选源改写（首选官方时原样返回）。
    pub fn rewrite_primary(self, url: &str) -> Result<String> {
        Ok(mirror::rewrite(url, &self.primary_mirror())?)
    }

    /// 中文显示名。
    pub fn display_name(self) -> &'static str {
        match self {
            DownloadSourcePolicy::Auto => "自动测速",
            DownloadSourcePolicy::OfficialFirst => "尽量官方",
            DownloadSourcePolicy::MirrorFirst => "尽量镜像",
        }
    }
}

/// 内存分配设置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySettings {
    /// 最大堆 `-Xmx`（MB）。
    pub max_mb: u32,
    /// 最小堆 `-Xms`（MB）；`None` 表示不显式设置。
    pub min_mb: Option<u32>,
}

impl Default for MemorySettings {
    fn default() -> Self {
        // 现代原版/轻 Mod 的稳妥默认；用户可在 config.json 或启动参数覆盖。
        Self {
            max_mb: 4096,
            min_mb: None,
        }
    }
}

/// 界面外观设置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// 当前背景；`None` 表示纯纸面——不装背景的人看到的仍是原来那个极简启动屏。
    pub background: Option<BackgroundRef>,
    /// 背景之上的纸色遮罩强度（百分比，0 到 [`MAX_BACKGROUND_VEIL`]）。
    ///
    /// 文字都落在不透明纸片上，可读性本不依赖它；这是给花图留的退路——
    /// 玩家的壁纸什么样都有，压一层纸色能把整屏观感拉回来。
    pub background_veil: u8,
}

/// 纸色遮罩的上限。再高纸色就把图盖没了，那不如直接不设背景。
pub const MAX_BACKGROUND_VEIL: u8 = 60;

/// 指向背景图库里的一张图。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundRef {
    /// `<数据目录>/backgrounds/` 下的文件名。
    ///
    /// 存文件名而不是绝对路径：数据目录随安装位置走（便携优先），
    /// 存绝对路径的话把整个 Aurora 文件夹搬到另一台机器，背景立刻断链。
    pub file: String,
    /// 导入时算出的平均色 `#rrggbb`。
    ///
    /// 图经自定义协议加载有延迟，前端先铺这个纯色再把图淡进来，避免开机闪一下白。
    pub tint: String,
    /// 主页右下角信息区背后那块图的亮度取样，用来决定压在上面的字该用墨色还是纸色。
    ///
    /// `None` 表示这张图是本功能上线之前导入的、还没量过。前端遇到 `None` 一律退回
    /// 不透明纸片，也就是改动前的行为——宁可多一块纸，也不能拿没量过的图去赌可读性。
    #[serde(default)]
    pub plate: Option<PlateZone>,
}

/// 主页右下角信息区背后那块图的亮度取样结果。
///
/// 存的是分位数而不是「均值 + 离散度」。字压在图上，能不能读取决于最不利的那一端，
/// 不是平均值：选了墨色字就怕区域里最暗的部分，选了纸色字就怕最亮的部分。
/// 直接存两端，前端拿它跟字色的达标线比一下即可，不需要再拍一个「离散度多大算花」的阈值。
///
/// 取 p10/p90 而不是最小/最大值：一小撮高光或暗点不该把整块区域判死。
///
/// 两个数都按 `0..=255` 映射 WCAG 相对亮度的 `0.0..=1.0`，存 u8 是因为配置要落 JSON，
/// 而它们只用来挑字色档位，小数位没有意义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlateZone {
    /// 相对亮度的第 10 百分位——区域里偏暗的那一端。墨色字要过的是这一关。
    pub p10: u8,
    /// 相对亮度的第 90 百分位——区域里偏亮的那一端。纸色字要过的是这一关。
    pub p90: u8,
}

/// 全局配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AuroraConfig {
    /// 文件下载源策略（库/资源/客户端 jar 等大文件）。
    pub download_source: DownloadSourcePolicy,
    /// 版本列表源策略（版本清单抓取）。
    pub version_list_source: DownloadSourcePolicy,
    /// 批量下载的文件级并发上限（对应功能矩阵「最大下载线程数」，默认 64）。
    pub download_concurrency: usize,
    /// 内存分配设置。
    pub memory: MemorySettings,
    /// 全局版本隔离档位。
    pub isolation_policy: IsolationPolicy,
    /// 微软登录 client_id（无内置默认；缺省时回落到环境变量 `AURORA_MSA_CLIENT_ID`）。
    pub msa_client_id: Option<String>,
    /// 自定义游戏目录（`.minecraft`）；缺省时用数据目录下的 `.minecraft`。
    pub game_directory: Option<PathBuf>,
    /// 额外的游戏目录，与当前目录并存。
    ///
    /// 初次设定扫到别的启动器（PCL2、官方启动器）的 `.minecraft` 时不会去改当前目录，
    /// 而是记在这里作为「其它文件夹」，让玩家随时切过去——他在别处攒了多年的存档不该
    /// 因为换了个启动器就得搬家。
    #[serde(default)]
    pub extra_game_directories: Vec<NamedDirectory>,
    /// 当前选中的启动版本 id（版本页设定，主页据此启动）；缺省或指向已卸载版本时主页回落到扫描首项。
    pub selected_version: Option<String>,
    /// 自定义缓存目录（对应功能矩阵「自定义缓存文件夹路径」）；缺省用系统默认。
    pub cache_directory: Option<PathBuf>,
    /// 找不到匹配 Java 时是否自动下载 Mojang 运行时。
    pub auto_download_java: bool,
    /// 界面外观（自定义背景）。
    #[serde(default)]
    pub appearance: AppearanceSettings,
}

/// 一条带名字的目录。名字是给人看的（「PCL2」「官方启动器」），路径才是身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedDirectory {
    pub name: String,
    pub path: PathBuf,
}

impl Default for AuroraConfig {
    fn default() -> Self {
        Self {
            download_source: DownloadSourcePolicy::default(),
            version_list_source: DownloadSourcePolicy::default(),
            download_concurrency: 64,
            memory: MemorySettings::default(),
            isolation_policy: IsolationPolicy::default(),
            msa_client_id: None,
            game_directory: None,
            extra_game_directories: Vec::new(),
            selected_version: None,
            cache_directory: None,
            auto_download_java: true,
            appearance: AppearanceSettings::default(),
        }
    }
}

/// 配置文件的读写存储。默认位置 `<数据目录>/config.json`，可注入路径供测试。
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    /// 指定配置文件路径构造。
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 配置文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 载入配置。文件不存在返回全默认；存在但内容损坏冒泡 [`CoreError::ConfigParse`]。
    pub async fn load(&self) -> Result<AuroraConfig> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| CoreError::ConfigParse {
                path: self.path.clone(),
                source,
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(AuroraConfig::default()),
            Err(source) => Err(CoreError::ConfigIo {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// 持久化配置（原子写入，带父目录创建）。
    pub async fn save(&self, config: &AuroraConfig) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(config).map_err(CoreError::ConfigSerialize)?;
        aurora_base::fs::atomic_write(&self.path, &bytes).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let c = AuroraConfig::default();
        assert_eq!(c.download_source, DownloadSourcePolicy::Auto);
        assert_eq!(c.version_list_source, DownloadSourcePolicy::Auto);
        assert_eq!(c.download_concurrency, 64);
        assert_eq!(c.memory.max_mb, 4096);
        assert!(c.memory.min_mb.is_none());
        assert!(c.auto_download_java);
        assert_eq!(c.isolation_policy, IsolationPolicy::ModLoadersAndNonRelease);
    }

    #[test]
    fn source_policy_orders_mirrors() {
        assert_eq!(
            DownloadSourcePolicy::OfficialFirst.mirror_order(),
            vec![MirrorSource::Official, MirrorSource::BmclApi]
        );
        assert_eq!(
            DownloadSourcePolicy::MirrorFirst.mirror_order(),
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
        // 自动测速的静态顺序同官方优先。
        assert_eq!(
            DownloadSourcePolicy::Auto.mirror_order(),
            vec![MirrorSource::Official, MirrorSource::BmclApi]
        );
        assert_eq!(
            DownloadSourcePolicy::MirrorFirst.primary_mirror(),
            MirrorSource::BmclApi
        );
        assert_eq!(
            DownloadSourcePolicy::Auto.primary_mirror(),
            MirrorSource::Official
        );
    }

    #[test]
    fn source_plan_candidate_order_follows_policy() {
        // 尽量镜像：Mojang 库 URL 的首选候选应是 BMCLAPI。
        let plan = DownloadSourcePolicy::MirrorFirst.source_plan();
        let got = plan
            .candidates("https://libraries.minecraft.net/foo/bar.jar")
            .unwrap();
        assert_eq!(got[0], "https://bmclapi2.bangbang93.com/maven/foo/bar.jar");
        assert_eq!(got[1], "https://libraries.minecraft.net/foo/bar.jar");
    }

    #[test]
    fn rewrite_primary_maps_manifest_to_mirror_only_when_mirror_first() {
        let manifest = aurora_install::VERSION_MANIFEST_V2;
        assert_eq!(
            DownloadSourcePolicy::MirrorFirst
                .rewrite_primary(manifest)
                .unwrap(),
            "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json"
        );
        // 官方优先/自动时清单地址原样。
        assert_eq!(
            DownloadSourcePolicy::OfficialFirst
                .rewrite_primary(manifest)
                .unwrap(),
            manifest
        );
    }

    #[tokio::test]
    async fn store_roundtrips_config_and_missing_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let store = ConfigStore::at(&path);

        // 文件不存在 -> 全默认。
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded, AuroraConfig::default());

        // 改几个字段后落盘再读回，应完全一致。
        let config = AuroraConfig {
            download_source: DownloadSourcePolicy::MirrorFirst,
            download_concurrency: 32,
            memory: MemorySettings {
                max_mb: 8192,
                min_mb: Some(1024),
            },
            msa_client_id: Some("client-abc".to_owned()),
            ..AuroraConfig::default()
        };
        store.save(&config).await.unwrap();

        let reloaded = ConfigStore::at(&path).load().await.unwrap();
        assert_eq!(reloaded, config);
    }

    #[tokio::test]
    async fn partial_config_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        // 只写一个字段，其余应取默认（serde(default)）。
        tokio::fs::write(&path, br#"{"download_concurrency": 8}"#)
            .await
            .unwrap();
        let loaded = ConfigStore::at(&path).load().await.unwrap();
        assert_eq!(loaded.download_concurrency, 8);
        assert_eq!(loaded.download_source, DownloadSourcePolicy::Auto);
        assert_eq!(loaded.memory.max_mb, 4096);
    }

    #[tokio::test]
    async fn corrupt_config_bubbles_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        tokio::fs::write(&path, b"{ not json").await.unwrap();
        let err = ConfigStore::at(&path).load().await.unwrap_err();
        assert!(matches!(err, CoreError::ConfigParse { .. }));
    }
}
