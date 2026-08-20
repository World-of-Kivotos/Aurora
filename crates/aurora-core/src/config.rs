//! 全局配置（config.json）与下载源三档策略。
//!
//! 门面持有一份可读写的全局配置：下载源与版本列表源各自的三档策略（对应功能矩阵「文件下载源 /
//! 版本列表源」两项独立三档设置）、下载并发、默认内存、版本隔离档位、微软 client_id、可选自定义
//! 游戏/缓存目录。凭据不进这里（归 aurora-auth 的加密存储）。
//!
//! 配置以 JSON 落在数据目录（`%LOCALAPPDATA%\Aurora\config.json`）。缺失时用全默认值，损坏则冒泡
//! 报错而非静默用默认覆盖（避免用户改坏的配置被无声吞掉）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aurora_base::mirror::{self, MirrorSource};
use aurora_download::{
    MirrorResolver, SourcePlan, SourceResolver, order_by_latency, probe_latencies,
};
use aurora_instance::IsolationPolicy;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

/// 下载源三档策略。对应功能矩阵的「文件下载源 / 版本列表源」三档设置。
///
/// - `OfficialFirst`：官方优先、镜像兜底，静态顺序，不测速。
/// - `MirrorFirst`：BMCLAPI 优先、官方兜底，静态顺序，不测速。
/// - `Auto`：真按实测排序——装机时对各候选源打一次轻量探针，按首字节时间升序排（见
///   [`SourceSpeedCache`]）；探不出结果时才落到 [`DownloadSourcePolicy::mirror_order`] 的兜底顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadSourcePolicy {
    /// 自动测速（默认）：按实测首字节时间排序，测不出来时退回镜像优先。
    #[default]
    Auto,
    /// 尽量官方：官方优先，镜像兜底。
    OfficialFirst,
    /// 尽量镜像：BMCLAPI 镜像优先，官方兜底。
    MirrorFirst,
}

impl DownloadSourcePolicy {
    /// 该策略下按优先级排列的镜像源列表。
    ///
    /// `Auto` 在这里给的是**兜底顺序**：实测结论由 [`SourceSpeedCache`] 持有，只有探针全部超时、
    /// 或样本地址根本解析不了时才会落到这一档。兜底取镜像优先而不是官方优先——玩家绝大多数在中国
    /// 大陆，官方源直连本就常年不稳（见 `aurora_base::mirror` 模块头），而「探针一个都没回来」恰恰
    /// 是官方直连最先垮的那种网况；BMCLAPI 打不通时引擎还会自动换到官方，反过来则要先赔上一整轮
    /// 重试超时才轮到镜像，代价不对等。
    pub fn mirror_order(self) -> Vec<MirrorSource> {
        match self {
            DownloadSourcePolicy::OfficialFirst => {
                vec![MirrorSource::Official, MirrorSource::BmclApi]
            }
            DownloadSourcePolicy::Auto | DownloadSourcePolicy::MirrorFirst => {
                vec![MirrorSource::BmclApi, MirrorSource::Official]
            }
        }
    }

    /// 该策略是否需要运行期测速。只有 `Auto` 需要，另两档是玩家显式指定的固定顺序。
    pub fn measures_speed(self) -> bool {
        matches!(self, DownloadSourcePolicy::Auto)
    }

    /// 该策略的首选源（用于不走 [`SourcePlan`] 换源的单源抓取，如 Java 运行时清单）。
    ///
    /// `Auto` 在这里取官方，与 [`DownloadSourcePolicy::mirror_order`] 的镜像优先刻意不一致：这条
    /// 路径是单源的，抓不到就直接失败，没有换源兜底；BMCLAPI 的元数据偶有滞后或 5xx，把没有退路的
    /// 抓取押上去不划算。清单只有几百 KB，直连慢一点毁不掉体验，而几百 MB 的本体与 assets 走的是
    /// [`DownloadSourcePolicy::mirror_order`] 那条有兜底、且会被测速重排的路。
    pub fn primary_mirror(self) -> MirrorSource {
        match self {
            DownloadSourcePolicy::MirrorFirst => MirrorSource::BmclApi,
            DownloadSourcePolicy::Auto | DownloadSourcePolicy::OfficialFirst => {
                MirrorSource::Official
            }
        }
    }

    /// 构造该策略对应的**静态**下载源调度方案，不含 `Auto` 的实测结论。
    ///
    /// 门面装配下载池走的是 [`SourceSpeedCache::order_now`]，那条路径才会把测速结果算进去；
    /// 本方法留给不持有测速缓存的调用方（如一次性的独立下载）。
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

/// 自动测速的探针目标：官方版本清单地址。
///
/// 选它而不是随手挑一个库或资源文件：BMCLAPI 对没缓存过的产物是回源拉取，拿冷门产物测出来的是
/// 「镜像回源」的耗时而不是线路质量；版本清单是两侧都最热的路径，量到的才是边缘节点的真实往返。
/// 它同时在镜像改写表里有对应条目，两个源解析到不同主机，比较才成立
/// （`probe_sample_url_differs_between_sources` 守着这条前提）。
///
/// 已知局限：官方侧真正扛下载量的是 `libraries.minecraft.net`、`resources.download.minecraft.net`
/// 等另外几台主机，与清单主机不同域名、可能落在不同边缘节点，而 BMCLAPI 侧这些路径全收敛到同一台。
/// 本探针拿清单主机的往返代表整个官方侧，遇上「清单通、库站不通」这类分化会排出偏乐观的顺序。仍不
/// 换目标：换成库或资源文件就掉进上面那个「量到的是镜像回源」的陷阱，而多主机加权探测的复杂度与
/// 收益不成比例——排错了也只是首选源不佳，引擎换源兜底仍在。
const PROBE_SAMPLE_URL: &str = aurora_install::VERSION_MANIFEST_V2;

/// 单个源的探针超时。给一个字节都要三秒的源，排序上让它输就是了，没必要等到 HTTP 客户端
/// 那 15 秒的连接超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// 测速结论的有效期。
///
/// 一次装机短则几分钟长则几十分钟，半小时能保证同一次装机只探一轮；再长就会把人锁在过期结论上
/// （换了网络、镜像临时挂了都属此列）。
const SPEED_TTL: Duration = Duration::from_secs(30 * 60);

/// `Auto` 档的测速结论缓存。
///
/// 只活在进程内，不落盘：测速量的是「此刻这条线路」，而玩家的网络会换（家宽/校园网/热点/代理），
/// 把昨天的结论从磁盘读回来复用，等于拿一个可能已经失效的判断替玩家做决定，比不测还糟。重启启动器
/// 即重测，成本是一次几百毫秒的探针。
///
/// 失效条件三条：超过 [`SPEED_TTL`]、[`SourceSpeedCache::invalidate`] 被显式调用（玩家在设置里切回
/// 自动测速时）、进程退出。
pub struct SourceSpeedCache {
    /// 探针打的样本地址。生产是 [`PROBE_SAMPLE_URL`]，测试注入本地 mock。
    sample_url: String,
    /// 把样本地址解析到各源的解析器。生产走镜像改写表，测试注入以把不同源指向不同 mock 服务器。
    resolver: Arc<dyn SourceResolver>,
    ttl: Duration,
    probe_timeout: Duration,
    state: Mutex<SpeedState>,
}

#[derive(Default)]
struct SpeedState {
    /// 最近一次成功的测速结论。
    latest: Option<Measurement>,
    /// 是否已有探测在途。装机流程会从多处装配下载池，没有这道闸就是同一轮网络状况被反复打探针。
    probing: bool,
}

struct Measurement {
    order: Vec<MirrorSource>,
    taken_at: Instant,
}

impl Default for SourceSpeedCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceSpeedCache {
    /// 生产构造：探针打版本清单，按镜像改写表解析。
    pub fn new() -> Self {
        Self::with_probe(
            PROBE_SAMPLE_URL,
            Arc::new(MirrorResolver),
            SPEED_TTL,
            PROBE_TIMEOUT,
        )
    }

    /// 注入探针目标、解析器与两个时限构造（测试把各源指向不同的本地 mock）。
    pub fn with_probe(
        sample_url: impl Into<String>,
        resolver: Arc<dyn SourceResolver>,
        ttl: Duration,
        probe_timeout: Duration,
    ) -> Self {
        Self {
            sample_url: sample_url.into(),
            resolver,
            ttl,
            probe_timeout,
            state: Mutex::new(SpeedState::default()),
        }
    }

    /// 当前该用的候选顺序：有未过期的实测结论就用它，否则用该策略的兜底顺序。
    ///
    /// 纯读缓存，不发任何请求——装配下载池是同步路径，不能在这里等网络。
    pub fn order_now(&self, policy: DownloadSourcePolicy) -> Vec<MirrorSource> {
        if !policy.measures_speed() {
            return policy.mirror_order();
        }
        self.fresh().unwrap_or_else(|| policy.mirror_order())
    }

    /// 是否已有未过期的实测结论。
    pub fn is_fresh(&self) -> bool {
        self.fresh().is_some()
    }

    /// 取候选顺序：`Auto` 且缓存过期或从未测过时，现测一轮并写回；其余情况直接返回静态顺序。
    ///
    /// 探测失败（超时、非 2xx、样本地址解析不了）一律不冒泡——测速是优化不是前置条件，
    /// 它挂掉只该让顺序退回兜底，不该让整个装机失败。
    pub async fn measured_order(
        &self,
        client: &reqwest::Client,
        policy: DownloadSourcePolicy,
    ) -> Vec<MirrorSource> {
        let fallback = policy.mirror_order();
        if !policy.measures_speed() {
            return fallback;
        }
        if let Some(order) = self.fresh() {
            return order;
        }
        if !self.claim_probe() {
            // 已有探测在途：本次直接用兜底顺序，不叠第二轮探针。
            return fallback;
        }
        let probed = self.probe(client, &fallback).await;
        self.finish_probe(probed.clone());
        probed.unwrap_or(fallback)
    }

    /// 丢弃已有结论，下次取用时重测。
    pub fn invalidate(&self) {
        self.lock().latest = None;
    }

    /// 对候选源逐个打探针并按首字节时间升序排。全军覆没时返回 `None`，让调用方走兜底且不写缓存。
    async fn probe(
        &self,
        client: &reqwest::Client,
        candidates: &[MirrorSource],
    ) -> Option<Vec<MirrorSource>> {
        let measured = match probe_latencies(
            client,
            &self.sample_url,
            candidates,
            self.resolver.as_ref(),
            self.probe_timeout,
        )
        .await
        {
            Ok(measured) => measured,
            Err(error) => {
                tracing::warn!(%error, "下载源测速的样本地址无法解析，本次退回兜底顺序");
                return None;
            }
        };

        for (source, latency) in &measured {
            match latency {
                Some(rtt) => tracing::debug!(
                    source = source.display_name(),
                    millis = rtt.as_millis() as u64,
                    "下载源探针返回"
                ),
                None => {
                    tracing::debug!(source = source.display_name(), "下载源探针超时或失败")
                }
            }
        }

        if measured.iter().all(|(_, latency)| latency.is_none()) {
            // 一个都没回来时排出来的顺序纯是入参原序，不是测量结果，缓存它等于把噪声当结论用半小时。
            tracing::warn!("所有下载源探针均未返回，本次退回兜底顺序且不写缓存");
            return None;
        }
        Some(order_by_latency(measured))
    }

    fn fresh(&self) -> Option<Vec<MirrorSource>> {
        let state = self.lock();
        let latest = state.latest.as_ref()?;
        if latest.taken_at.elapsed() < self.ttl {
            Some(latest.order.clone())
        } else {
            None
        }
    }

    /// 抢占探测权。已有探测在途时返回 false。
    fn claim_probe(&self) -> bool {
        let mut state = self.lock();
        if state.probing {
            return false;
        }
        state.probing = true;
        true
    }

    /// 交回探测权，顺带写入结论（`None` 表示这轮没测出东西，保持缓存为空以便下次重测）。
    fn finish_probe(&self, order: Option<Vec<MirrorSource>>) {
        let mut state = self.lock();
        state.probing = false;
        if let Some(order) = order {
            state.latest = Some(Measurement {
                order,
                taken_at: Instant::now(),
            });
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SpeedState> {
        // 锁内只有几次 Vec 克隆与赋值，不会 panic；真中毒说明别处已经炸了，此时继续用脏状态更危险。
        self.state.lock().expect("下载源测速缓存锁中毒")
    }
}

#[cfg(test)]
impl SourceSpeedCache {
    /// 测试注入：写入一份「刚测出来」的结论。
    pub(crate) fn seed(&self, order: Vec<MirrorSource>) {
        self.finish_probe(Some(order));
    }
}

/// 内存分配设置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySettings {
    /// 最大堆 `-Xmx`（MB）。`auto` 为真时这个值不参与启动，但仍原样保留——
    /// 关掉自动就该回到用户上次亲手拖的那个数，而不是被自动算出来的数悄悄覆盖掉。
    pub max_mb: u32,
    /// 最小堆 `-Xms`（MB）；`None` 表示不显式设置。
    ///
    /// 设置界面不再暴露它（PCL2 同样不暴露，`-Xms` 对玩家几乎没有调节价值），
    /// 但配置与启动路径一律保留：老配置里写过的值继续生效，启动选项也仍可显式覆盖。
    pub min_mb: Option<u32>,
    /// 是否按本机可用内存自动分配最大堆。
    ///
    /// 默认关闭，且必须保持关闭：老配置里没有这个键，`#[serde(default)]` 会让它落到这里的默认值上，
    /// 默认开就等于在一次升级里把所有老用户亲手设的内存数悄悄换掉。
    pub auto: bool,
}

impl Default for MemorySettings {
    fn default() -> Self {
        // 现代原版/轻 Mod 的稳妥默认；用户可在 config.json 或启动参数覆盖。
        Self {
            max_mb: 4096,
            min_mb: None,
            auto: false,
        }
    }
}

/// 玻璃模式：界面材质用哪一套处理背景图。
///
/// 缺省是 `Frost`，而且这个默认值是安全侧而不是审美选择：配置读不出来、字段是老版本写的、
/// 或者以后这个枚举被删掉，界面都落在纯毛玻璃这一档，不会出现「浮层带高光却没有底」的样子。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlassMode {
    /// 纯毛玻璃。所有材质只有模糊与饱和，零额外合成开销。
    #[default]
    Frost,
    /// 毛玻璃 + 小件液态：只给白名单里那几个小件补受光亮边与斜向高光。
    ///
    /// 两档的纸色不透明度完全相同，所以对比度预算表在两档下同时成立，不必维护两套。
    Liquid,
}

/// 界面外观设置。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    /// 玩家自选的背景；`None` 表示没自选过，此时铺的是按当前游戏挑的内置背景
    /// （见 background.rs 的 `builtin_backgrounds`），而不是空白纸面。
    pub background: Option<BackgroundRef>,
    /// 背景之上的纸色遮罩强度（百分比，0 到 [`MAX_BACKGROUND_VEIL`]）。
    ///
    /// 文字都落在不透明纸片上，可读性本不依赖它；这是给花图留的退路——
    /// 玩家的壁纸什么样都有，压一层纸色能把整屏观感拉回来。
    pub background_veil: u8,
    /// 玻璃模式。与背景同属外观，一起落进这份配置，才能随 Aurora 文件夹一起搬家。
    pub glass: GlassMode,
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
        assert!(!c.memory.auto, "自动分配默认必须关闭");
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
        // 自动测速探不出结果时的兜底是镜像优先，不是官方优先。
        assert_eq!(
            DownloadSourcePolicy::Auto.mirror_order(),
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
        assert_eq!(
            DownloadSourcePolicy::MirrorFirst.primary_mirror(),
            MirrorSource::BmclApi
        );
        // 单源抓取没有换源兜底，自动测速档在这条路径上仍取官方。
        assert_eq!(
            DownloadSourcePolicy::Auto.primary_mirror(),
            MirrorSource::Official
        );
        assert!(DownloadSourcePolicy::Auto.measures_speed());
        assert!(!DownloadSourcePolicy::OfficialFirst.measures_speed());
        assert!(!DownloadSourcePolicy::MirrorFirst.measures_speed());
    }

    #[test]
    fn probe_sample_url_differs_between_sources() {
        // 样本地址必须在镜像改写表内有对应条目，否则两个源解析到同一个端点，测速纯属自己跟自己比。
        let official = mirror::rewrite(PROBE_SAMPLE_URL, &MirrorSource::Official).unwrap();
        let bmcl = mirror::rewrite(PROBE_SAMPLE_URL, &MirrorSource::BmclApi).unwrap();
        assert_ne!(official, bmcl);
        assert!(bmcl.starts_with(aurora_base::mirror::BMCL_BASE));
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
                auto: true,
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

    /// 自动分配这个键是后加的，老 config.json 里没有它。
    ///
    /// 这条钉的是升级安全：老用户亲手设的 max_mb 必须原样保留，`auto` 必须落成 false。
    /// 一旦有人把 `MemorySettings::default().auto` 改成 true，这台机器上所有老配置
    /// 会在下次启动时被自动算出来的数顶掉，而且没有任何提示——所以它值一条独立断言。
    #[tokio::test]
    async fn config_without_auto_key_keeps_manual_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        tokio::fs::write(&path, br#"{"memory":{"max_mb":10240,"min_mb":2048}}"#)
            .await
            .unwrap();
        let loaded = ConfigStore::at(&path).load().await.unwrap();
        assert_eq!(loaded.memory.max_mb, 10240);
        assert_eq!(loaded.memory.min_mb, Some(2048));
        assert!(!loaded.memory.auto);
    }

    #[tokio::test]
    async fn corrupt_config_bubbles_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        tokio::fs::write(&path, b"{ not json").await.unwrap();
        let err = ConfigStore::at(&path).load().await.unwrap_err();
        assert!(matches!(err, CoreError::ConfigParse { .. }));
    }

    // ---- 自动测速 ----

    /// 把两个源分别钉到两台本地 mock 服务器上，探针于是量的是这两台的真实响应时延。
    struct PinnedResolver {
        official: String,
        bmcl: String,
    }

    impl SourceResolver for PinnedResolver {
        fn resolve(
            &self,
            _url: &str,
            source: &MirrorSource,
        ) -> aurora_download::Result<String> {
            Ok(match source {
                MirrorSource::Official => self.official.clone(),
                MirrorSource::BmclApi => self.bmcl.clone(),
                MirrorSource::Provided(url) => url.clone(),
            })
        }
    }

    /// 起一台按给定状态码与延迟应答的 mock 源。
    async fn mock_source(status: u16, delay: Duration) -> wiremock::MockServer {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(status).set_delay(delay))
            .mount(&server)
            .await;
        server
    }

    fn cache_for(official: &wiremock::MockServer, bmcl: &wiremock::MockServer) -> SourceSpeedCache {
        cache_with_ttl(official, bmcl, Duration::from_secs(60))
    }

    fn cache_with_ttl(
        official: &wiremock::MockServer,
        bmcl: &wiremock::MockServer,
        ttl: Duration,
    ) -> SourceSpeedCache {
        SourceSpeedCache::with_probe(
            "https://libraries.minecraft.net/probe",
            Arc::new(PinnedResolver {
                official: format!("{}/probe", official.uri()),
                bmcl: format!("{}/probe", bmcl.uri()),
            }),
            ttl,
            Duration::from_secs(3),
        )
    }

    async fn hits(server: &wiremock::MockServer) -> usize {
        server.received_requests().await.unwrap().len()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn faster_source_is_ranked_first() {
        // 官方快、镜像慢：排出来必须是官方在前——这恰好与兜底顺序相反，能真正证明排序生效。
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(250)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        let order = cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(order, vec![MirrorSource::Official, MirrorSource::BmclApi]);
        assert!(cache.is_fresh());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slower_official_sinks_below_mirror() {
        let official = mock_source(200, Duration::from_millis(250)).await;
        let bmcl = mock_source(200, Duration::ZERO).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        let order = cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(order, vec![MirrorSource::BmclApi, MirrorSource::Official]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn all_probes_failing_falls_back_and_is_not_cached() {
        // 两个源都 500：既没有可比的时延，也不该把这轮噪声当结论缓存半小时。
        let official = mock_source(500, Duration::ZERO).await;
        let bmcl = mock_source(500, Duration::ZERO).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        let order = cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(order, DownloadSourcePolicy::Auto.mirror_order());
        assert!(!cache.is_fresh());

        // 没缓存 -> 下一次仍会重新探测。
        let before = hits(&official).await;
        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(hits(&official).await, before + 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_source_down_still_yields_a_usable_order() {
        // 官方探不通、镜像通：失败源沉底而不是让整件事失败。
        let official = mock_source(503, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(120)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        let order = cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(order, vec![MirrorSource::BmclApi, MirrorSource::Official]);
        assert!(cache.is_fresh());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cache_hit_issues_no_further_probes() {
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(200)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        let first = cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        let (official_hits, bmcl_hits) = (hits(&official).await, hits(&bmcl).await);
        assert_eq!(official_hits, 1);
        assert_eq!(bmcl_hits, 1);

        for _ in 0..3 {
            let again = cache
                .measured_order(&client, DownloadSourcePolicy::Auto)
                .await;
            assert_eq!(again, first);
        }
        // 缓存命中就不该再碰网络：请求数必须一个不涨。
        assert_eq!(hits(&official).await, official_hits);
        assert_eq!(hits(&bmcl).await, bmcl_hits);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn expired_cache_is_remeasured() {
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(80)).await;
        let cache = cache_with_ttl(&official, &bmcl, Duration::from_millis(1));
        let client = aurora_base::http::build_client().unwrap();

        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(hits(&official).await, 1);

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!cache.is_fresh());
        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(hits(&official).await, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalidate_forces_a_new_measurement() {
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(80)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(hits(&official).await, 1);

        cache.invalidate();
        assert!(!cache.is_fresh());
        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(hits(&official).await, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fixed_policies_never_probe() {
        // 玩家显式选定的两档是承诺，不是建议：一个探针都不该发。
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(200)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        assert_eq!(
            cache
                .measured_order(&client, DownloadSourcePolicy::OfficialFirst)
                .await,
            vec![MirrorSource::Official, MirrorSource::BmclApi]
        );
        assert_eq!(
            cache
                .measured_order(&client, DownloadSourcePolicy::MirrorFirst)
                .await,
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
        assert_eq!(hits(&official).await, 0);
        assert_eq!(hits(&bmcl).await, 0);
        assert!(!cache.is_fresh());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn order_now_reads_cache_without_touching_network() {
        let official = mock_source(200, Duration::ZERO).await;
        let bmcl = mock_source(200, Duration::from_millis(200)).await;
        let cache = cache_for(&official, &bmcl);
        let client = aurora_base::http::build_client().unwrap();

        // 冷缓存 -> 兜底顺序，且一个请求都不发。
        assert_eq!(
            cache.order_now(DownloadSourcePolicy::Auto),
            DownloadSourcePolicy::Auto.mirror_order()
        );
        assert_eq!(hits(&official).await, 0);

        cache
            .measured_order(&client, DownloadSourcePolicy::Auto)
            .await;
        assert_eq!(
            cache.order_now(DownloadSourcePolicy::Auto),
            vec![MirrorSource::Official, MirrorSource::BmclApi]
        );
        // 固定档不看实测结论。
        assert_eq!(
            cache.order_now(DownloadSourcePolicy::MirrorFirst),
            vec![MirrorSource::BmclApi, MirrorSource::Official]
        );
    }
}
