//! Aurora Tauri 外壳的 IPC 层。
//!
//! 只承担一件事：把 aurora-core 门面（[`Aurora`]）经 `#[tauri::command]` 暴露给 React 前端。
//! 三条纪律贯穿全文件：
//! 1. 门面放进 managed state，用 `tokio::sync::RwLock` 包裹——`Aurora` 兼有 `&self` 异步方法与
//!    `set_client_id`/`set_game_dir` 这类 `&mut self` 方法，异步锁才能把两类方法都用上。
//!    取读写锁而非互斥锁是必须的：命令在持锁期间会跑网络请求（搜索、版本列表、依赖解析），
//!    而三十多个命令里只有 update_config 与 set_game_directory 需要可变借用。用 Mutex 会让
//!    所有命令全局串行，一次慢请求就把界面上其它操作全堵住——启动预取并发拉五份数据时尤其明显。
//! 2. 绝不把 aurora-core 原始类型（尤其含登录令牌的 [`Account`]）整体过 IPC；命令一律返回本文件
//!    定义的瘦 DTO，只映射前端需要的安全字段。
//! 3. 进度/事件走一条固定范式：命令内建一个 `tokio::mpsc` 通道作为门面的 [`EventSink`]，另起一个
//!    转发任务把 [`CoreEvent`] 逐条 `emit` 成 Tauri 事件推给前端（见 `create_offline_account`）。

mod logging;

use std::path::PathBuf;
use std::sync::Arc;

use aurora_core::{
    builtin_background_bytes, builtin_backgrounds, detect_crash, Account, AccountType,
    AggregateResult, Aurora, BackgroundEntry, BuiltinBackground, CoreEvent, CrashReport,
    DetectSource, DeviceCodeResponse, DownloadSourcePolicy, GameSession, GlassMode, History,
    InstallPlan,
    InstalledMod, InstanceMatch, IsolationOverride, IsolationPolicy, JavaInstallation, JavaVersion,
    LaunchOptions, Ledger, LoaderChoice, LogLine, LogStream, MemoryAdvice, MemorySettings, ModLoader,
    ManagedModpackFile, ManagedModpackStatus, ModVersionInfo, ModpackInstallOutcome,
    ModpackSyncError, ModpackSyncOutcome, ModpackSyncProgress, NamedDirectory, Platform,
    ResolvedIsolation, ResourceType, RollbackCheck, SearchHit, SearchQuery, SortField,
    UpdateCandidate, VersionManifest, VersionScan, VersionSettings,
};
use aurora_core::folders::GameDirectoryEntry;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex, RwLock};

/// 前端订阅进度事件的统一事件名。install/launch 等后续长任务照抄本范式时复用同一事件名，
/// 前端按负载里的 `kind` 区分阶段/告警/下载进度。
const CORE_EVENT: &str = "aurora://core-event";

/// 微软设备码登录专用事件名。门面回调拿到设备码时经此推送 user_code/验证网址给前端展示。
const DEVICE_CODE_EVENT: &str = "aurora://device-code";

/// 游戏进程日志事件名。每行 stdout/stderr 输出经此推送给前端日志窗口。
const GAME_LOG_EVENT: &str = "aurora://game-log";

/// 游戏崩溃事件名。进程异常退出时由会话监控任务推送一份 [`CrashReport`]（诊断 + 可疑 Mod + 日志路径）。
///
/// 与 [`GAME_LOG_EVENT`] 分开：日志是流水，崩溃是一次性的结论，前端要据此弹提示条而不是刷列表。
/// 玩家主动点「结束游戏」不会触发本事件（[`detect_crash`] 对启动器主动终止的会话短路）。
const GAME_CRASH_EVENT: &str = "aurora://game-crash";

// ===== 面向前端的瘦 DTO =====
//
// 下面这些枚举（DownloadSourcePolicy / IsolationPolicy / AccountType / MemorySettings）在 aurora-core
// 已 `derive(Serialize)` 且带 snake_case 重命名，直接内嵌即可得到稳定的 JSON 表示，无需在此重复映射。
// 唯独 Account 含令牌，绝不整体序列化——单独摘成 AccountDto。

/// 全局配置 DTO（对应前端设置/主页需要读取的安全字段）。
#[derive(Serialize)]
struct ConfigDto {
    /// 当前游戏目录（`.minecraft`）绝对路径。
    game_dir: String,
    /// 数据目录（`%LOCALAPPDATA%\Aurora`）绝对路径。
    data_dir: String,
    /// 文件下载源策略。
    download_source: DownloadSourcePolicy,
    /// 版本列表源策略。
    version_list_source: DownloadSourcePolicy,
    /// 批量下载文件级并发上限。
    download_concurrency: usize,
    /// 内存分配设置。
    memory: MemorySettings,
    /// 全局版本隔离档位。
    isolation_policy: IsolationPolicy,
    /// 是否已配置微软登录 client_id（不回传 id 本身，前端只需知道能否走正版登录）。
    has_client_id: bool,
    /// 找不到匹配 Java 时是否自动下载。
    auto_download_java: bool,
    /// 当前选中的启动版本 id（版本页设定，主页据此启动）；未选择时为 null。
    selected_version: Option<String>,
}

/// 已安装版本探测到的加载器 DTO。
#[derive(Serialize)]
struct LoaderDto {
    /// 加载器名称（Fabric/Quilt/Forge/NeoForge/OptiFine/LiteLoader）。
    kind: String,
    /// 加载器版本号（无法确定时为 null）。
    version: Option<String>,
}

/// 一个成功解析的已安装版本 DTO。
#[derive(Serialize)]
struct InstalledVersionDto {
    /// 版本 id（等于版本目录名）。
    id: String,
    /// 基础 Minecraft 版本：modded 取版本 JSON 的 inheritsFrom，vanilla 即 id。
    mc_version: String,
    /// 是否正式版（type == release）。
    is_release: bool,
    /// 是否装有任一 Mod 加载器。
    has_mod_loader: bool,
    /// 探测到的加载器列表。
    loaders: Vec<LoaderDto>,
}

/// 一个无法解析的版本目录 DTO。
#[derive(Serialize)]
struct BrokenVersionDto {
    id: String,
    /// 损坏原因的人类可读说明。
    reason: String,
}

/// 版本级设置 DTO：用户设置本身 + 按该设置解析出的实际工作目录状态。
#[derive(Serialize)]
struct VersionSettingsDto {
    /// 自定义描述（未设置为 null）。
    description: Option<String>,
    /// 自定义图标标识（未设置为 null）。
    icon: Option<String>,
    /// 是否收藏。
    favorite: bool,
    /// 自定义分类名（未设置为 null）。
    category: Option<String>,
    /// 版本级隔离覆盖档位。
    isolation: IsolationOverride,
    /// 该版本运行时的游戏工作目录绝对路径（mod 装进它下面的 `mods/`）。
    working_dir: String,
    /// 最终是否隔离（已综合全局档位、版本级覆盖与本地数据强制）。
    isolated: bool,
    /// 是否因版本目录下已有 mods/saves 而被强制隔离——此时覆盖设为「不隔离」也不会生效，
    /// 界面需要据此解释为何开关看起来没起作用。
    forced_by_local_data: bool,
}

impl VersionSettingsDto {
    fn new(settings: VersionSettings, resolved: ResolvedIsolation) -> Self {
        Self {
            description: settings.description,
            icon: settings.icon,
            favorite: settings.favorite,
            category: settings.category,
            isolation: settings.isolation,
            working_dir: resolved.working_dir.display().to_string(),
            isolated: resolved.isolated,
            forced_by_local_data: resolved.forced_by_local_data,
        }
    }
}

/// 写入版本级设置的入参（整体覆盖）。
#[derive(Deserialize)]
struct VersionSettingsInput {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    favorite: bool,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    isolation: IsolationOverride,
}

/// `versions/` 扫描结果 DTO。
#[derive(Serialize)]
struct VersionScanDto {
    versions: Vec<InstalledVersionDto>,
    broken: Vec<BrokenVersionDto>,
}

/// 账户 DTO——只暴露 uuid / name / account_type，绝不含任何 access/refresh 令牌。
#[derive(Serialize)]
struct AccountDto {
    uuid: String,
    name: String,
    account_type: AccountType,
}

/// 进度事件 DTO：CoreEvent 摊平成带 `kind` 标签的 JSON，前端 listen 后按 kind 分支处理。
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CoreEventDto {
    /// 阶段推进（人类可读的一句话）。
    Stage { message: String },
    /// 非阻断性告警。
    Warning { message: String },
    /// 批量下载进度快照。
    Download {
        total: u64,
        finished: u64,
        bytes: u64,
        speed: u64,
    },
    /// 受管整合包同步的结构化阶段与进度。
    ModpackSync {
        operation_id: Option<String>,
        progress: ModpackSyncProgress,
    },
}

impl From<CoreEvent> for CoreEventDto {
    fn from(event: CoreEvent) -> Self {
        match event {
            CoreEvent::Stage(message) => CoreEventDto::Stage { message },
            CoreEvent::Warning(message) => CoreEventDto::Warning { message },
            CoreEvent::Download(p) => CoreEventDto::Download {
                total: p.total,
                finished: p.finished,
                bytes: p.bytes,
                speed: p.speed,
            },
            CoreEvent::ModpackSync(progress) => CoreEventDto::ModpackSync {
                operation_id: None,
                progress,
            },
        }
    }
}

fn correlated_modpack_event(event: CoreEvent, operation_id: &str) -> CoreEventDto {
    match event {
        CoreEvent::ModpackSync(progress) => CoreEventDto::ModpackSync {
            operation_id: Some(operation_id.to_owned()),
            progress,
        },
        other => CoreEventDto::from(other),
    }
}

/// 微软设备码 DTO：登录进入设备码阶段时经 [`DEVICE_CODE_EVENT`] 推给前端，供其展示待输入短码与验证网址。
#[derive(Serialize, Clone)]
struct DeviceCodeDto {
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
    message: String,
}

impl From<&DeviceCodeResponse> for DeviceCodeDto {
    fn from(device: &DeviceCodeResponse) -> Self {
        DeviceCodeDto {
            user_code: device.user_code.clone(),
            verification_uri: device.verification_uri.clone(),
            expires_in: device.expires_in,
            interval: device.interval,
            message: device.message.clone(),
        }
    }
}

/// 游戏进程一行输出 DTO（经 [`GAME_LOG_EVENT`] 推送）。
#[derive(Serialize, Clone)]
struct GameLogDto {
    /// 来源流：stdout / stderr。
    stream: String,
    text: String,
}

impl From<LogLine> for GameLogDto {
    fn from(line: LogLine) -> Self {
        GameLogDto {
            stream: match line.stream {
                LogStream::Stdout => "stdout",
                LogStream::Stderr => "stderr",
            }
            .to_owned(),
            text: line.text,
        }
    }
}

/// 版本清单里的单个版本条目 DTO。
#[derive(Serialize)]
struct ManifestVersionDto {
    id: String,
    /// 版本类型（release / snapshot / old_beta / old_alpha）。
    release_type: String,
    /// 该版本完整 JSON 的下载地址。
    url: String,
    time: String,
    release_time: String,
    sha1: Option<String>,
    compliance_level: Option<u32>,
}

/// latest 区块 DTO：最新正式版与最新快照的 id。
#[derive(Serialize)]
struct LatestDto {
    release: String,
    snapshot: String,
}

/// 版本清单 DTO。
#[derive(Serialize)]
struct ManifestDto {
    latest: LatestDto,
    versions: Vec<ManifestVersionDto>,
}

/// 原版安装摘要 DTO。
#[derive(Serialize)]
struct VanillaSummaryDto {
    id: String,
    libraries: usize,
    assets: usize,
    natives: u32,
}

/// 加载器安装摘要 DTO。
#[derive(Serialize)]
struct LoaderSummaryDto {
    id: String,
    loader_version: String,
    libraries: usize,
}

/// 一次安装结果 DTO：原版摘要 + 可选加载器摘要。
#[derive(Serialize)]
struct InstallOutcomeDto {
    vanilla: VanillaSummaryDto,
    loader: Option<LoaderSummaryDto>,
}

/// 归一后的 Java 版本号 DTO。
#[derive(Serialize)]
struct JavaVersionDto {
    major: u32,
    minor: u32,
    security: u32,
    build: u32,
    raw: String,
}

/// 一个已探测 Java 安装的 DTO。
#[derive(Serialize)]
struct JavaInstallationDto {
    path: String,
    version: JavaVersionDto,
    is_64bit: bool,
    vendor: String,
    /// 探测来源：registry / common_dir / path / managed。
    source: String,
}

/// 一次 Java 运行时安装结果 DTO。
#[derive(Serialize)]
struct InstalledRuntimeDto {
    component: String,
    version: JavaVersionDto,
    java_executable: String,
}

/// 启动成功 DTO：仅回传进程 id（会话本体存于后端 managed state，不过 IPC）。
#[derive(Serialize)]
struct LaunchedDto {
    pid: Option<u32>,
}

/// 聚合搜索里某平台的失败记录 DTO。
#[derive(Serialize)]
struct PlatformErrorDto {
    platform: Platform,
    message: String,
}

/// 聚合搜索结果 DTO。命中直接复用已 `Serialize` 的核心 [`SearchHit`]（不含任何令牌，安全），
/// 失败记录摊平成平台 + 文案。
#[derive(Serialize)]
struct SearchResultDto {
    hits: Vec<SearchHit>,
    errors: Vec<PlatformErrorDto>,
}

/// 一次模组安装结果 DTO。
#[derive(Serialize)]
struct ModInstallOutcomeDto {
    file_name: String,
    path: String,
    platform: Platform,
}

/// 一次 kill 请求：附一条回执通道，让 `stop_game` 拿到 kill 的真实成败，而不是把请求发出去就报成功。
type KillRequest = tokio::sync::oneshot::Sender<Result<(), String>>;

/// 运行中游戏的 kill 句柄（会话监控任务的请求端）。
type KillHandle = tokio::sync::mpsc::UnboundedSender<KillRequest>;

/// 运行中的游戏槽：启动时存入 kill 句柄，`stop_game` 取用，进程结束后由监控任务自行摘除。
///
/// 为什么存句柄而不是 [`GameSession`] 本体：`GameSession::wait`（等进程退出、回收读取任务、flush 日志
/// 归档，产出 `ExitReport`）消耗 self，而 `kill` 要 `&mut self`——同一个会话没法既留在槽里供 kill、
/// 又交给某处去 wait。故会话在启动时就移交给一条监控任务独占，本槽只留一条请求通道。
///
/// 与门面分列两个 managed state：门面是 `RwLock<Aurora>`，而会话生命周期独立于门面——启动后门面锁应
/// 尽快释放以便其它命令继续读写配置，故不塞进门面而单列一个槽。内层用 `Arc` 是为了让监控任务也能
/// 持有同一个槽，进程退出时把自己的句柄摘掉。
struct RunningGame(Arc<Mutex<Option<KillHandle>>>);

// ===== 映射辅助 =====

fn account_dto(account: &Account) -> AccountDto {
    AccountDto {
        uuid: account.uuid.clone(),
        name: account.name.clone(),
        account_type: account.account_type,
    }
}

fn scan_dto(scan: VersionScan) -> VersionScanDto {
    VersionScanDto {
        versions: scan
            .versions
            .into_iter()
            .map(|v| InstalledVersionDto {
                id: v.id.clone(),
                mc_version: v.json.inherits_from.clone().unwrap_or_else(|| v.id.clone()),
                is_release: v.is_release(),
                has_mod_loader: v.has_mod_loader(),
                loaders: v
                    .loaders
                    .iter()
                    .map(|l| LoaderDto {
                        kind: l.kind.as_str().to_owned(),
                        version: l.version.clone(),
                    })
                    .collect(),
            })
            .collect(),
        broken: scan
            .broken
            .into_iter()
            .map(|b| BrokenVersionDto {
                id: b.id,
                reason: match b.reason {
                    aurora_core::BrokenReason::MissingJson => "缺少版本 JSON".to_owned(),
                    aurora_core::BrokenReason::Parse(detail) => format!("版本 JSON 损坏：{detail}"),
                },
            })
            .collect(),
    }
}

/// 读取当前选中账户：门面自己跨「加密凭据库 + 明文离线库」两处仲裁，这里只负责摘成 DTO。
fn read_current_account(aurora: &Aurora) -> Result<Option<AccountDto>, String> {
    let current = aurora.current_account().map_err(|e| e.to_string())?;
    Ok(current.as_ref().map(account_dto))
}

// ---- 字符串枚举映射 ----
//
// 前端传字符串（loader:"forge"、platform:"modrinth"、policy:"mirror_first"…），命令内 match 成核心枚举。
// 取值统一用核心类型 serde 的 snake_case 表示，与出参 DTO 序列化保持同一套命名，round-trip 一致。
// 非法值报清晰错误，绝不 panic、绝不静默兜底一个默认值。

fn parse_loader_choice(name: &str) -> Result<LoaderChoice, String> {
    match name {
        "fabric" => Ok(LoaderChoice::Fabric),
        "quilt" => Ok(LoaderChoice::Quilt),
        "forge" => Ok(LoaderChoice::Forge),
        "neoforge" => Ok(LoaderChoice::NeoForge),
        other => Err(format!("未知加载器 {other}（支持 fabric/quilt/forge/neoforge）")),
    }
}

fn parse_platform(name: &str) -> Result<Platform, String> {
    match name {
        "modrinth" => Ok(Platform::Modrinth),
        "curseforge" => Ok(Platform::CurseForge),
        other => Err(format!("未知资源平台 {other}（支持 modrinth/curseforge）")),
    }
}

fn parse_download_source_policy(name: &str) -> Result<DownloadSourcePolicy, String> {
    match name {
        "auto" => Ok(DownloadSourcePolicy::Auto),
        "official_first" => Ok(DownloadSourcePolicy::OfficialFirst),
        "mirror_first" => Ok(DownloadSourcePolicy::MirrorFirst),
        other => Err(format!("未知下载源策略 {other}（支持 auto/official_first/mirror_first）")),
    }
}

fn parse_isolation_policy(name: &str) -> Result<IsolationPolicy, String> {
    match name {
        "disabled" => Ok(IsolationPolicy::Disabled),
        "mod_loaders_only" => Ok(IsolationPolicy::ModLoadersOnly),
        "non_release_only" => Ok(IsolationPolicy::NonReleaseOnly),
        "mod_loaders_and_non_release" => Ok(IsolationPolicy::ModLoadersAndNonRelease),
        "all" => Ok(IsolationPolicy::All),
        other => Err(format!("未知隔离档位 {other}")),
    }
}

fn parse_resource_type(name: &str) -> Result<ResourceType, String> {
    match name {
        "mod" => Ok(ResourceType::Mod),
        "modpack" => Ok(ResourceType::Modpack),
        "resource_pack" => Ok(ResourceType::ResourcePack),
        "shader" => Ok(ResourceType::Shader),
        "data_pack" => Ok(ResourceType::DataPack),
        "plugin" => Ok(ResourceType::Plugin),
        other => Err(format!("未知资源类型 {other}")),
    }
}

fn parse_mod_loader(name: &str) -> Result<ModLoader, String> {
    match name {
        "fabric" => Ok(ModLoader::Fabric),
        "quilt" => Ok(ModLoader::Quilt),
        "forge" => Ok(ModLoader::Forge),
        "neoforge" => Ok(ModLoader::NeoForge),
        "liteloader" => Ok(ModLoader::LiteLoader),
        other => Err(format!("未知加载器 {other}")),
    }
}

fn parse_sort_field(name: &str) -> Result<SortField, String> {
    match name {
        "relevance" => Ok(SortField::Relevance),
        "downloads" => Ok(SortField::Downloads),
        "follows" => Ok(SortField::Follows),
        "newest" => Ok(SortField::Newest),
        "updated" => Ok(SortField::Updated),
        other => Err(format!("未知排序字段 {other}")),
    }
}

/// [`DetectSource`] 摊平成稳定的 snake_case 字符串（DetectSource 本身未 derive Serialize）。
fn detect_source_str(source: DetectSource) -> &'static str {
    match source {
        DetectSource::Registry => "registry",
        DetectSource::CommonDir => "common_dir",
        DetectSource::Path => "path",
        DetectSource::Managed => "managed",
    }
}

fn java_version_dto(version: &JavaVersion) -> JavaVersionDto {
    JavaVersionDto {
        major: version.major,
        minor: version.minor,
        security: version.security,
        build: version.build,
        raw: version.raw.clone(),
    }
}

fn java_installation_dto(java: &JavaInstallation) -> JavaInstallationDto {
    JavaInstallationDto {
        path: java.path.display().to_string(),
        version: java_version_dto(&java.version),
        is_64bit: java.is_64bit,
        vendor: java.vendor.clone(),
        source: detect_source_str(java.source).to_owned(),
    }
}

fn manifest_dto(manifest: VersionManifest) -> ManifestDto {
    ManifestDto {
        latest: LatestDto {
            release: manifest.latest.release,
            snapshot: manifest.latest.snapshot,
        },
        versions: manifest
            .versions
            .into_iter()
            .map(|v| ManifestVersionDto {
                id: v.id,
                release_type: v.release_type,
                url: v.url,
                time: v.time,
                release_time: v.release_time,
                sha1: v.sha1,
                compliance_level: v.compliance_level,
            })
            .collect(),
    }
}

fn search_result_dto(result: AggregateResult) -> SearchResultDto {
    SearchResultDto {
        hits: result.hits,
        errors: result
            .errors
            .into_iter()
            .map(|e| PlatformErrorDto {
                platform: e.platform,
                message: e.error.to_string(),
            })
            .collect(),
    }
}

// ---- 事件推送小工具 ----
//
// 安装/启动那套「mpsc 通道当 EventSink + 桥接任务」的范式只适用于收 [`EventSink`] 的门面方法。
// Mod 治理那批方法（哈希反查、更新检查）在 core 里是一趟到底的批处理，签名不收 sink，也就没有中间进度
// 可转发。故这里只在任务首尾各推一条阶段说明，让前端知道任务开始与结束——绝不编造并不存在的百分比。

fn emit_stage(app: &AppHandle, message: String) {
    let _ = app.emit(CORE_EVENT, CoreEventDto::Stage { message });
}

fn emit_warning(app: &AppHandle, message: String) {
    let _ = app.emit(CORE_EVENT, CoreEventDto::Warning { message });
}

// ---- 账户库访问（凭据加密仅 Windows）----
//
// 账户的读取/切换/删除在门面上已是跨平台的（离线账户走明文库，凭据账户走 DPAPI 库），命令直接调即可。
// 只有微软/authlib 登录仍是 Windows 专属：非 Windows 下明确报“平台不受支持”，绝不静默假装成功。

#[cfg(not(windows))]
const WINDOWS_ONLY: &str = "该操作在当前平台不受支持（账户凭据加密仅限 Windows）";

/// 按 uuid 取出完整账户（含令牌，仅供内部传给 launch_account，绝不过 IPC）。
/// 门面跨两个库寻址，故离线账户同样可按 uuid 启动，不必非走 offline_name 那条路。
fn find_account_impl(aurora: &Aurora, uuid: &str) -> Result<Account, String> {
    aurora
        .find_account(uuid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("账户 {uuid} 不存在"))
}

/// 启动前静默续期：微软账户缓存的 Minecraft 令牌过期时用 refresh_token 换新并回写；其它账户原样返回。
/// refresh_token 也失效时给出可操作的重登提示。
#[cfg(windows)]
async fn ensure_fresh_impl(aurora: &Aurora, account: &Account) -> Result<Account, String> {
    aurora.ensure_microsoft_fresh(account).await.map_err(|e| {
        format!("微软账户续期失败，登录可能已过期，请在账户页重新登录（或检查网络）：{e}")
    })
}

#[cfg(not(windows))]
async fn ensure_fresh_impl(_aurora: &Aurora, account: &Account) -> Result<Account, String> {
    // 非 Windows 无账户库，此路径实际到不了（find_account_impl 已先行报错）；原样返回以保证跨平台编译。
    Ok(account.clone())
}

// ===== IPC 命令 =====
//
// 全部为 async：借用 managed state 的命令必须返回 Result（Tauri 对借用 State 的异步命令的硬性要求），
// 内部一律 `state.read().await` 取门面（改配置的两个命令用 `write()`）。CoreError 经 `to_string()`
// 转成字符串上抛，让前端能显示；
// 不在命令里 try/catch 生吞。

/// 读取全局配置（含游戏目录、内存、下载源策略、是否已配 client_id 等）。
#[tauri::command]
async fn get_config(state: State<'_, RwLock<Aurora>>) -> Result<ConfigDto, String> {
    let aurora = state.read().await;
    let config = aurora.config();
    Ok(ConfigDto {
        game_dir: aurora.game_dir().display().to_string(),
        data_dir: aurora.data_dir().display().to_string(),
        download_source: config.download_source,
        version_list_source: config.version_list_source,
        download_concurrency: config.download_concurrency,
        memory: config.memory,
        isolation_policy: config.isolation_policy,
        has_client_id: config.msa_client_id.is_some(),
        auto_download_java: config.auto_download_java,
        selected_version: config.selected_version.clone(),
    })
}

/// 内存态势：本机物理内存、其它程序占用、滑块刻度阶梯、自动分配此刻会给多少。
///
/// 设置页每次进入「游戏」那一页都会取一次。刻意不做成订阅推送：可用内存每秒都在动，
/// 推上去只会让那根条抖个不停；玩家要的是「我现在拖到哪合适」，一张打开页面那一刻的快照就够。
#[tauri::command]
async fn memory_advice(state: State<'_, RwLock<Aurora>>) -> Result<MemoryAdvice, String> {
    let aurora = state.read().await;
    aurora.memory_advice().await.map_err(|e| e.to_string())
}

/// 扫描游戏目录下已安装的版本（含损坏版本单列）。
#[tauri::command]
async fn list_installed(state: State<'_, RwLock<Aurora>>) -> Result<VersionScanDto, String> {
    let aurora = state.read().await;
    let scan = aurora.list_installed().await.map_err(|e| e.to_string())?;
    Ok(scan_dto(scan))
}

/// 检查一个实例是否受整合包管理，并返回当前成功快照与可用发布版本。普通实例返回 null。
#[tauri::command]
async fn managed_modpack_status(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Option<ManagedModpackStatus>, String> {
    let aurora = state.read().await;
    aurora
        .managed_modpack_status(&version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 读取成功快照里的受管文件归属。普通实例返回 null，尚未成功同步的受管实例返回空数组。
#[tauri::command]
async fn managed_modpack_files(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Option<Vec<ManagedModpackFile>>, String> {
    let aurora = state.read().await;
    aurora
        .managed_modpack_files(&version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 把受管实例同步到刚检查到的目标版本。失败保持结构化，不压平成不可操作的一句话。
#[tauri::command]
async fn sync_managed_modpack(
    app: AppHandle,
    version_id: String,
    target_version: String,
    operation_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<ModpackSyncOutcome, ModpackSyncError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = forwarder.emit(CORE_EVENT, correlated_modpack_event(event, &operation_id));
        }
    });

    let result = {
        let aurora = state.read().await;
        aurora
            .sync_managed_modpack(&version_id, &target_version, Some(&tx))
            .await
    };

    drop(tx);
    let _ = forward_task.await;
    result
}

/// 从远端指针完成 Minecraft、加载器、订阅与空快照同步，返回真正可启动的实例 id。
#[tauri::command]
async fn install_managed_modpack(
    app: AppHandle,
    pointer_url: String,
    operation_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<ModpackInstallOutcome, ModpackSyncError> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = forwarder.emit(CORE_EVENT, correlated_modpack_event(event, &operation_id));
        }
    });

    let result = {
        let aurora = state.read().await;
        aurora.install_managed_modpack(&pointer_url, Some(&tx)).await
    };

    drop(tx);
    let _ = forward_task.await;
    result
}

/// 读取当前选中账户（可能没有）。
#[tauri::command]
async fn current_account(state: State<'_, RwLock<Aurora>>) -> Result<Option<AccountDto>, String> {
    let aurora = state.read().await;
    read_current_account(&aurora)
}

/// 保存一个离线账户（写进数据目录下的明文 `offline_accounts.json`，关掉启动器再开还在），并把它设为
/// 当前账户；同时示范“进度事件流”范式。同名重复创建是幂等的，不会长出第二条。
///
/// 为什么这样转发（后续 install/launch 页面照抄的模板）：aurora-core 与任何 UI 框架解耦，它只认一个
/// `tokio::mpsc<CoreEvent>` 作为 [`EventSink`]，不知道 Tauri 的存在。而 Tauri 的 `app.emit` 才是把事件
/// 送进 WebView 的机制。于是这里建一个 mpsc 通道当 EventSink 传进门面，另 spawn 一个桥接任务把收到的
/// 每条 CoreEvent 翻译成 DTO 后 emit 出去——core 保持框架无关，前端又能实时收到阶段/告警/下载进度。
/// 长任务（安装/启动）只需把 `create_offline_account` 换成对应的门面异步方法，桥接骨架原样复用即可。
#[tauri::command]
async fn create_offline_account(
    app: AppHandle,
    name: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();

    // 桥接任务：通道关闭（sender 全部 drop）后 recv 返回 None，循环自然结束。
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            // emit 失败（无监听者/窗口已关）不影响主流程，事件本就是尽力而为的通知。
            let _ = forwarder.emit(CORE_EVENT, CoreEventDto::from(event));
        }
    });

    // 单独作用域持锁：门面方法本身很快，尽早释放锁再去 await 桥接任务。
    let account = {
        let aurora = state.read().await;
        aurora
            .add_offline_account(&name, Some(&tx))
            .map_err(|e| e.to_string())?
    };

    // 丢弃唯一的 sender，让桥接任务排空剩余事件后退出，再等它 join。
    drop(tx);
    let _ = forward_task.await;

    Ok(account_dto(&account))
}

/// 微软设备码登录（两段式）。第一段：门面回调拿到设备码时经 [`DEVICE_CODE_EVENT`] 把待输入短码与验证
/// 网址推给前端；随后 await 轮询直至令牌换取完成、账户落库，返回登录到的账户。
///
/// 全程持有门面锁：设备码轮询期间前端应停在登录弹窗，其它需要门面的命令暂等属预期行为。
#[tauri::command]
async fn microsoft_login(
    app: AppHandle,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    microsoft_login_impl(app, state).await
}

#[cfg(windows)]
async fn microsoft_login_impl(
    app: AppHandle,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    let aurora = state.read().await;
    let account = aurora
        .microsoft_login(|device| {
            // emit 失败（无监听者/窗口已关）不影响登录主流程。
            let _ = app.emit(DEVICE_CODE_EVENT, DeviceCodeDto::from(device));
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(account_dto(&account))
}

#[cfg(not(windows))]
async fn microsoft_login_impl(
    _app: AppHandle,
    _state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    Err(WINDOWS_ONLY.to_owned())
}

/// Authlib-Injector（第三方验证服务器）用户名密码登录，成功后账户落库并返回。
#[tauri::command]
async fn authlib_login(
    server_url: String,
    username: String,
    password: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    authlib_login_impl(&server_url, &username, &password, state).await
}

#[cfg(windows)]
async fn authlib_login_impl(
    server_url: &str,
    username: &str,
    password: &str,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    let aurora = state.read().await;
    let account = aurora
        .authlib_login(server_url, username, password)
        .await
        .map_err(|e| e.to_string())?;
    Ok(account_dto(&account))
}

#[cfg(not(windows))]
async fn authlib_login_impl(
    _server_url: &str,
    _username: &str,
    _password: &str,
    _state: State<'_, RwLock<Aurora>>,
) -> Result<AccountDto, String> {
    Err(WINDOWS_ONLY.to_owned())
}

/// 读取账户库中的全部账户（只含 uuid/name/type，无任何令牌）。
#[tauri::command]
async fn list_accounts(state: State<'_, RwLock<Aurora>>) -> Result<Vec<AccountDto>, String> {
    let aurora = state.read().await;
    let accounts = aurora.accounts().map_err(|e| e.to_string())?;
    Ok(accounts.iter().map(account_dto).collect())
}

/// 切换当前选中账户。
#[tauri::command]
async fn set_current_account(uuid: String, state: State<'_, RwLock<Aurora>>) -> Result<(), String> {
    let aurora = state.read().await;
    aurora.set_current_account(&uuid).map_err(|e| e.to_string())
}

/// 删除账户。
#[tauri::command]
async fn remove_account(uuid: String, state: State<'_, RwLock<Aurora>>) -> Result<(), String> {
    let aurora = state.read().await;
    aurora.remove_account(&uuid).map_err(|e| e.to_string())
}

/// 拉取官方版本清单（最新正式版/快照 + 全部可安装版本条目）。
#[tauri::command]
async fn list_manifest(state: State<'_, RwLock<Aurora>>) -> Result<ManifestDto, String> {
    let aurora = state.read().await;
    let manifest = aurora.list_manifest().await.map_err(|e| e.to_string())?;
    Ok(manifest_dto(manifest))
}

/// 安装指定原版版本，并可选叠加一个 Mod 加载器（进度经 [`CORE_EVENT`] 推送）。
#[tauri::command]
async fn install_version(
    app: AppHandle,
    id: String,
    loader: Option<String>,
    loader_version: Option<String>,
    state: State<'_, RwLock<Aurora>>,
) -> Result<InstallOutcomeDto, String> {
    let loader_choice = match loader {
        Some(name) => Some(parse_loader_choice(&name)?),
        None => None,
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = forwarder.emit(CORE_EVENT, CoreEventDto::from(event));
        }
    });

    let outcome = {
        let aurora = state.read().await;
        aurora
            .install(&id, loader_choice, loader_version.as_deref(), Some(&tx))
            .await
            .map_err(|e| e.to_string())?
    };

    drop(tx);
    let _ = forward_task.await;

    Ok(InstallOutcomeDto {
        vanilla: VanillaSummaryDto {
            id: outcome.vanilla.id,
            libraries: outcome.vanilla.libraries,
            assets: outcome.vanilla.assets,
            natives: outcome.vanilla.natives,
        },
        loader: outcome.loader.map(|l| LoaderSummaryDto {
            id: l.id,
            loader_version: l.loader_version,
            libraries: l.libraries,
        }),
    })
}

/// 启动一个已安装版本。给定 `account_uuid` 走在线账户启动，否则用 `offline_name` 走离线启动
/// （两者都缺则报错，不静默兜底）。
///
/// 建两条通道：游戏每行输出经 [`GAME_LOG_EVENT`] 推送、启动阶段事件经 [`CORE_EVENT`] 推送。拿到会话后
/// 存入 [`RunningGame`] 供 `stop_game` 取用，返回进程 id。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn launch_game(
    app: AppHandle,
    version_id: String,
    account_uuid: Option<String>,
    offline_name: Option<String>,
    max_memory_mb: Option<u32>,
    min_memory_mb: Option<u32>,
    fullscreen: bool,
    extra_jvm_args: Vec<String>,
    extra_game_args: Vec<String>,
    resolution: Option<(u32, u32)>,
    demo: bool,
    state: State<'_, RwLock<Aurora>>,
    running: State<'_, RunningGame>,
) -> Result<LaunchedDto, String> {
    let options = LaunchOptions {
        max_memory_mb,
        min_memory_mb,
        fullscreen,
        extra_jvm_args,
        extra_game_args,
        resolution,
        demo,
    };

    // 阶段/告警/下载事件 -> core-event。
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let event_app = app.clone();
    let event_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            let _ = event_app.emit(CORE_EVENT, CoreEventDto::from(event));
        }
    });

    // 游戏进程逐行输出 -> game-log。接收端交给会话监控任务，那里同时兼管 kill 与退出后的崩溃判定
    // （通道关闭正是「进程输出已结束」的可靠信号，详见 [`spawn_game_monitor`]）。
    let (log_tx, log_rx) = tokio::sync::mpsc::channel::<LogLine>(256);

    let session = {
        let aurora = state.read().await;
        let launched = match account_uuid.as_deref() {
            Some(uuid) => {
                let account = find_account_impl(&aurora, uuid)?;
                // 启动前静默续期：微软账户缓存令牌过期则用 refresh_token 换新，避免拿废令牌启动。
                let account = ensure_fresh_impl(&aurora, &account).await?;
                aurora
                    .launch_account(&version_id, &account, &options, Some(log_tx), Some(&event_tx))
                    .await
            }
            None => {
                let name = offline_name
                    .as_deref()
                    .ok_or_else(|| "启动需提供 account_uuid 或 offline_name".to_owned())?;
                aurora
                    .launch_offline(&version_id, name, &options, Some(log_tx), Some(&event_tx))
                    .await
            }
        };
        launched.map_err(|e| e.to_string())?
    };

    let pid = session.id();

    // kill 句柄进槽、会话本体移交监控任务。直接覆盖旧值：上一局若已自行结束，其句柄早被自己摘掉；
    // 若仍在跑，覆盖后旧句柄只是不再可达，旧监控任务照常把那一局收尾（不会丢日志与崩溃诊断）。
    let (kill_tx, kill_rx) = tokio::sync::mpsc::unbounded_channel::<KillRequest>();
    *running.0.lock().await = Some(kill_tx.clone());
    spawn_game_monitor(
        app.clone(),
        running.0.clone(),
        version_id,
        session,
        log_rx,
        kill_tx,
        kill_rx,
    );

    drop(event_tx);
    let _ = event_task.await;

    Ok(LaunchedDto { pid })
}

/// 起一条游戏会话监控任务：实时转发日志、随时响应 kill，进程结束后判定崩溃并推送诊断。
///
/// 三件事必须挤在同一条任务里，因为它们都要用到同一个 [`GameSession`]：`kill` 要 `&mut`、`wait` 要
/// 所有权，跨任务分持做不到。串起它们的是日志通道——[`aurora_launch`] 的两个读取任务在 stdout/stderr
/// 双双关闭时才会丢掉发送端，因此 `log_rx` 返回 `None` 就是「进程输出已结束」的可靠信号。于是 select
/// 循环期间会话留在本任务手里供 kill 使用，信号到来后再独占它去 `wait`。
///
/// `own_kill_tx` 是本任务自己那条句柄的克隆，只用来在收尾时确认槽里放的还是自己（而不是下一局游戏
/// 刚放进去的句柄），避免误摘。
#[allow(clippy::too_many_arguments)]
fn spawn_game_monitor(
    app: AppHandle,
    slot: Arc<Mutex<Option<KillHandle>>>,
    version_id: String,
    mut session: GameSession,
    mut log_rx: tokio::sync::mpsc::Receiver<LogLine>,
    own_kill_tx: KillHandle,
    mut kill_rx: tokio::sync::mpsc::UnboundedReceiver<KillRequest>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                line = log_rx.recv() => match line {
                    Some(line) => {
                        // emit 失败（窗口已关）不影响监控本身，日志本就是尽力而为的通知。
                        let _ = app.emit(GAME_LOG_EVENT, GameLogDto::from(line));
                    }
                    None => break,
                },
                Some(ack) = kill_rx.recv() => {
                    let result = session.kill().await.map_err(|e| e.to_string());
                    // 回执发不出去只说明 stop_game 那侧已经不等了，不改变 kill 已经执行的事实。
                    let _ = ack.send(result);
                }
            }
        }

        // 先摘句柄再 wait：wait 可能还要等上一会儿，这期间 stop_game 应当直接变回幂等空操作，
        // 而不是对着一个正在收尾的会话发 kill。
        {
            let mut guard = slot.lock().await;
            if guard
                .as_ref()
                .is_some_and(|tx| tx.same_channel(&own_kill_tx))
            {
                *guard = None;
            }
        }

        // 归档路径必须在 wait 之前取——wait 消耗会话。
        let archive_path = session.archive_path().map(|p| p.display().to_string());
        let exit = match session.wait().await {
            Ok(exit) => exit,
            Err(err) => {
                // 本 crate 没有接日志框架，失败经告警事件如实上报，不静默咽下。
                emit_warning(
                    &app,
                    format!("等待游戏进程退出失败，本次不做崩溃判定：{err}"),
                );
                return;
            }
        };
        // 玩家主动点「结束游戏」的退出在此短路（ExitReport::terminated_by_launcher 已置位），
        // 不会被报成崩溃。
        if !detect_crash(&exit) {
            return;
        }

        // 诊断喂的是退出报告里缓存的最近若干行，而不是回读归档文件：这份文本就是本次会话的现场，
        // 归档若因磁盘问题没能建立，读文件反而会拿到上一局的日志、指认一场不存在的崩溃。
        let log_text = exit.recent_lines.join("\n");
        let diagnosed = {
            let state = app.state::<RwLock<Aurora>>();
            let aurora = state.read().await;
            aurora.diagnose_crash(&version_id, &log_text).await
        };
        match diagnosed {
            Ok(mut report) => {
                // 诊断文本与归档文件同出一源，直接把路径补上供 UI 提供「打开日志」。
                report.log_path = archive_path;
                let _ = app.emit(GAME_CRASH_EVENT, report);
            }
            Err(err) => {
                emit_warning(&app, format!("游戏异常退出，但崩溃诊断失败：{err}"));
            }
        }
    });
}

/// 结束当前运行中的游戏进程（对应“取消/强制结束”）。无运行中的游戏时为幂等空操作。
///
/// 会话本体在监控任务手里，故这里发一条 kill 请求并等它的回执，把 kill 的真实成败原样返回。
#[tauri::command]
async fn stop_game(running: State<'_, RunningGame>) -> Result<(), String> {
    let handle = running.0.lock().await.clone();
    let Some(kill_tx) = handle else {
        return Ok(());
    };

    let (ack_tx, ack_rx) = tokio::sync::oneshot::channel();
    if kill_tx.send(ack_tx).is_err() {
        // 接收端已随监控任务一起结束，即进程本就已经退出：没有可结束的游戏，与槽为空同义。
        return Ok(());
    }
    match ack_rx.await {
        Ok(result) => result,
        // 监控任务在收到请求与回执之间结束了——同样意味着进程已自行退出。这不是被吞掉的错误，
        // 而是「已经没有运行中的游戏」这一确凿结果。
        Err(_) => Ok(()),
    }
}

/// 探测本机全部可用 Java（注册表 / 常见目录 / PATH）。
#[tauri::command]
async fn detect_java(state: State<'_, RwLock<Aurora>>) -> Result<Vec<JavaInstallationDto>, String> {
    let aurora = state.read().await;
    let installations = aurora.detect_java().await;
    Ok(installations.iter().map(java_installation_dto).collect())
}

/// 下载并安装匹配主版本的 Mojang Java 运行时（进度经 [`CORE_EVENT`] 推送）。
#[tauri::command]
async fn install_java(
    app: AppHandle,
    required_major: u32,
    state: State<'_, RwLock<Aurora>>,
) -> Result<InstalledRuntimeDto, String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = forwarder.emit(CORE_EVENT, CoreEventDto::from(event));
        }
    });

    let runtime = {
        let aurora = state.read().await;
        aurora
            .install_java(required_major, Some(&tx))
            .await
            .map_err(|e| e.to_string())?
    };

    drop(tx);
    let _ = forward_task.await;

    Ok(InstalledRuntimeDto {
        component: runtime.component,
        version: java_version_dto(&runtime.version),
        java_executable: runtime.java_executable.display().to_string(),
    })
}

/// 更新全局配置：每个 Some 字段调对应 setter，最后落盘。字段全可选，未提供者不动。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn update_config(
    download_source: Option<String>,
    version_list_source: Option<String>,
    download_concurrency: Option<usize>,
    memory: Option<MemorySettings>,
    isolation_policy: Option<String>,
    auto_download_java: Option<bool>,
    cache_directory: Option<String>,
    client_id: Option<String>,
    selected_version: Option<String>,
    state: State<'_, RwLock<Aurora>>,
) -> Result<(), String> {
    let mut aurora = state.write().await;
    if let Some(policy) = download_source {
        aurora.set_download_source(parse_download_source_policy(&policy)?);
    }
    if let Some(policy) = version_list_source {
        aurora
            .set_version_list_source(parse_download_source_policy(&policy)?)
            .map_err(|e| e.to_string())?;
    }
    if let Some(concurrency) = download_concurrency {
        aurora.set_download_concurrency(concurrency);
    }
    if let Some(memory) = memory {
        aurora.set_memory(memory);
    }
    if let Some(policy) = isolation_policy {
        aurora.set_isolation_policy(parse_isolation_policy(&policy)?);
    }
    if let Some(enabled) = auto_download_java {
        aurora.set_auto_download_java(enabled);
    }
    if let Some(dir) = cache_directory {
        aurora.set_cache_directory(Some(PathBuf::from(dir)));
    }
    if let Some(id) = client_id {
        aurora.set_client_id(id);
    }
    if let Some(id) = selected_version {
        aurora.set_selected_version(Some(id));
    }
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 设置游戏目录（`.minecraft`）并落盘。
#[tauri::command]
async fn set_game_directory(path: String, state: State<'_, RwLock<Aurora>>) -> Result<(), String> {
    let mut aurora = state.write().await;
    aurora.set_game_directory(PathBuf::from(path));
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 是否为首次启动（配置文件还没落过盘）。前端据此决定要不要走初次设定。
#[tauri::command]
async fn is_first_run(state: State<'_, RwLock<Aurora>>) -> Result<bool, String> {
    let aurora = state.read().await;
    Ok(!aurora.config_saved())
}

/// 列出全部已知游戏目录（当前 + 其它文件夹），含各自当前是否可达。
#[tauri::command]
async fn list_game_directories(
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<GameDirectoryEntry>, String> {
    let aurora = state.read().await;
    Ok(aurora.game_directories())
}

/// 探测机器上尚未记录的其它 `.minecraft`（官方启动器、PCL2 等），只报告不写入。
#[tauri::command]
async fn discover_game_directories(
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<NamedDirectory>, String> {
    let aurora = state.read().await;
    Ok(aurora.discover_game_directories())
}

/// 记下一个「其它文件夹」并落盘。同路径已存在时只更新名字。
#[tauri::command]
async fn add_game_directory(
    name: String,
    path: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<(), String> {
    let mut aurora = state.write().await;
    aurora.add_game_directory(name, PathBuf::from(path));
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 移除一个「其它文件夹」记录（只动配置，不碰磁盘上的文件）。
#[tauri::command]
async fn remove_game_directory(
    path: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<bool, String> {
    let mut aurora = state.write().await;
    let removed = aurora.remove_game_directory(&PathBuf::from(path));
    if removed {
        aurora.save_config().await.map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

/// 把某个目录切为当前游戏目录，原当前目录自动转入「其它文件夹」。
#[tauri::command]
async fn switch_game_directory(
    path: String,
    name: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<(), String> {
    let mut aurora = state.write().await;
    aurora.switch_game_directory(PathBuf::from(path), name);
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 走完初次设定：确定游戏目录、收下选中的其它文件夹，然后把配置落盘。
///
/// 落盘这一下同时也是「不再是首次启动」的标记，所以必须在这里做完整一次保存，
/// 而不是等用户之后碰巧改了别的设置才写。
#[tauri::command]
async fn complete_first_run(
    game_dir: String,
    extras: Vec<NamedDirectory>,
    state: State<'_, RwLock<Aurora>>,
) -> Result<(), String> {
    let mut aurora = state.write().await;
    let target = PathBuf::from(game_dir);
    // 目录可能还不存在（用户选了个新位置）：先建出来，免得首页扫描直接报错。
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|e| format!("创建游戏目录失败：{e}"))?;
    aurora.set_game_directory(target);
    for extra in extras {
        aurora.add_game_directory(extra.name, extra.path);
    }
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 聚合搜索 Modrinth + CurseForge。前端传字符串枚举，命令内构造 [`SearchQuery`]。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn search_resources(
    query: Option<String>,
    resource_type: String,
    loaders: Vec<String>,
    game_versions: Vec<String>,
    sort: String,
    limit: u32,
    offset: u32,
    state: State<'_, RwLock<Aurora>>,
) -> Result<SearchResultDto, String> {
    let mut parsed_loaders = Vec::with_capacity(loaders.len());
    for loader in &loaders {
        parsed_loaders.push(parse_mod_loader(loader)?);
    }
    let search_query = SearchQuery {
        query,
        resource_type: parse_resource_type(&resource_type)?,
        loaders: parsed_loaders,
        game_versions,
        sort: parse_sort_field(&sort)?,
        limit,
        offset,
    };

    let result = {
        let aurora = state.read().await;
        aurora.search(&search_query).await.map_err(|e| e.to_string())?
    };
    Ok(search_result_dto(result))
}

/// 把某平台上的一个模组版本安装到指定实例的 mods 目录（进度经 [`CORE_EVENT`] 推送）。
#[tauri::command]
async fn install_mod(
    app: AppHandle,
    version_id: String,
    platform: String,
    project_id: String,
    mod_version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<ModInstallOutcomeDto, String> {
    let target_platform = parse_platform(&platform)?;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<CoreEvent>();
    let forwarder = app.clone();
    let forward_task = tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = forwarder.emit(CORE_EVENT, CoreEventDto::from(event));
        }
    });

    let outcome = {
        let aurora = state.read().await;
        aurora
            .install_mod(&version_id, target_platform, &project_id, &mod_version_id, Some(&tx))
            .await
            .map_err(|e| e.to_string())?
    };

    drop(tx);
    let _ = forward_task.await;

    Ok(ModInstallOutcomeDto {
        file_name: outcome.file_name,
        path: outcome.path.display().to_string(),
        platform: outcome.platform,
    })
}

/// 列出指定实例已装模组（含禁用态与解析出的元数据）。
///
/// [`InstalledMod`] 已 derive `Serialize` 且不含任何令牌，直接透传即安全，无需再摊 DTO。
#[tauri::command]
async fn list_mods(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<InstalledMod>, String> {
    let aurora = state.read().await;
    aurora.list_mods(&version_id).await.map_err(|e| e.to_string())
}

/// 启用/禁用指定实例里的某个模组，返回切换后的磁盘路径。
#[tauri::command]
async fn set_mod_enabled(
    version_id: String,
    file_name: String,
    enabled: bool,
    state: State<'_, RwLock<Aurora>>,
) -> Result<String, String> {
    let aurora = state.read().await;
    let path = aurora
        .set_mod_enabled(&version_id, &file_name, enabled)
        .await
        .map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// 读取某已安装版本的用户设置，并附上按当前设置解析出的实际工作目录。
///
/// 解析结果与设置一起返回，是为了让界面能常驻回显「这个实例的文件到底落在哪、是否隔离」——
/// 隔离状态只写在设置里而不显示实际路径，玩家仍旧无从判断 mod 会被装到哪个 mods 目录。
#[tauri::command]
async fn get_version_settings(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<VersionSettingsDto, String> {
    let aurora = state.read().await;
    let settings = aurora
        .version_settings(&version_id)
        .await
        .map_err(|e| e.to_string())?;
    let resolved = aurora
        .resolve_working_dir(&version_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(VersionSettingsDto::new(settings, resolved))
}

/// 整体覆盖某已安装版本的用户设置，返回写入后重新解析的结果。
///
/// 取整体覆盖而非逐字段 patch：`description` 这类 `Option` 字段在 patch 语义下无法区分
/// 「不改」与「清空」，而这里的字段数很少，前端读出完整对象改完写回即可，语义无歧义。
#[tauri::command]
async fn set_version_settings(
    version_id: String,
    settings: VersionSettingsInput,
    state: State<'_, RwLock<Aurora>>,
) -> Result<VersionSettingsDto, String> {
    let aurora = state.read().await;
    let next = VersionSettings {
        description: settings.description,
        icon: settings.icon,
        favorite: settings.favorite,
        category: settings.category,
        isolation: settings.isolation,
    };
    aurora
        .set_version_settings(&version_id, &next)
        .await
        .map_err(|e| e.to_string())?;
    // 隔离覆盖可能刚被改写，这里重新解析，让前端拿到的路径与状态就是下次启动/装 Mod 会用的那份。
    let resolved = aurora
        .resolve_working_dir(&version_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(VersionSettingsDto::new(next, resolved))
}

// ===== Mod 治理：版本列表 / 落位匹配 / 依赖计划 / 更新 / 变更史 / 崩溃诊断 =====
//
// 这一组的返回类型（[`ModVersionInfo`] / [`InstanceMatch`] / [`InstallPlan`] / [`UpdateCandidate`] /
// [`History`] / [`RollbackCheck`] / [`Ledger`] / [`CrashReport`]）在 aurora-core 均已 `derive(Serialize)`
// 且字段命名就是前端契约里那套 snake_case，也不含任何令牌，故直接透传，不再重复摊一层 DTO——
// 多摊一层只会让两处字段名各自漂移。

/// 列出某工程的全部可用版本（跨平台统一模型，按发布时间倒序）。
///
/// `game_versions` / `loaders` 传空数组表示该维度不过滤。工程不存在时平台的 404 原样冒泡，
/// 不吞成空列表——「这个工程一个版本都没有」与「这个工程根本不存在」对界面是两件事。
#[tauri::command]
async fn list_mod_versions(
    platform: String,
    project_id: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<ModVersionInfo>, String> {
    let target_platform = parse_platform(&platform)?;
    let mut parsed_loaders = Vec::with_capacity(loaders.len());
    for loader in &loaders {
        parsed_loaders.push(parse_mod_loader(loader)?);
    }

    let aurora = state.read().await;
    aurora
        .list_mod_versions(
            target_platform,
            &project_id,
            &game_versions,
            &parsed_loaders,
        )
        .await
        .map_err(|e| e.to_string())
}

/// 为某工程算出判定矩阵（每个已装实例配上该工程在其上最合适的版本），供下载页的安装落位层直接渲染。
///
/// 返回顺序已按「完美匹配 > 可能可行 > 不兼容」排好，界面默认选中第一项即可。
#[tauri::command]
async fn match_instances(
    platform: String,
    project_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<InstanceMatch>, String> {
    let target_platform = parse_platform(&platform)?;
    let aurora = state.read().await;
    aurora
        .match_instances(target_platform, &project_id)
        .await
        .map_err(|e| e.to_string())
}

/// 解析依赖并产出安装计划（只自动收 Required 依赖，其余进 `skipped` 如实说明）。
#[tauri::command]
async fn plan_install(
    version_id: String,
    platform: String,
    project_id: String,
    mod_version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<InstallPlan, String> {
    let target_platform = parse_platform(&platform)?;
    let aurora = state.read().await;
    aurora
        .plan_install(&version_id, target_platform, &project_id, &mod_version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 对卷宗里没有身份的已装 Mod 做哈希反查补身份，返回补上的条数。
///
/// 逐个文件算哈希再联网反查，Mod 多的实例会跑上一阵，故首尾各推一条阶段事件（见 [`emit_stage`]）。
#[tauri::command]
async fn identify_installed_mods(
    app: AppHandle,
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<usize, String> {
    emit_stage(&app, format!("正在反查 {version_id} 里未知来源的 Mod"));

    let added = {
        let aurora = state.read().await;
        aurora
            .identify_installed_mods(&version_id)
            .await
            .map_err(|e| e.to_string())?
    };

    emit_stage(&app, format!("来源反查完成，补上 {added} 个 Mod 的身份"));
    Ok(added)
}

/// 检查该实例可更新的 Mod。实例未装 Mod 加载器时返回空列表。
///
/// 每条卷宗记录都要发一次平台请求，故与 [`identify_installed_mods`] 同样首尾各推一条阶段事件。
#[tauri::command]
async fn check_updates(
    app: AppHandle,
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<UpdateCandidate>, String> {
    emit_stage(&app, format!("正在检查 {version_id} 的 Mod 更新"));

    let candidates = {
        let aurora = state.read().await;
        aurora
            .check_updates(&version_id)
            .await
            .map_err(|e| e.to_string())?
    };

    emit_stage(
        &app,
        format!("更新检查完成，{} 个 Mod 有新版本", candidates.len()),
    );
    Ok(candidates)
}

/// 读取该实例的变更历史（时间正序）。
#[tauri::command]
async fn list_history(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<History, String> {
    let aurora = state.read().await;
    aurora.history(&version_id).await.map_err(|e| e.to_string())
}

/// 逐条判断历史事件能否回滚，顺序与 [`list_history`] 一致，界面可按下标对齐。
#[tauri::command]
async fn rollback_checks(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<RollbackCheck>, String> {
    let aurora = state.read().await;
    aurora
        .rollback_checks(&version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 回滚一次更新事件：把 `<file>.old` 改回原名、删掉新文件，并追加一条回滚事件。
#[tauri::command]
async fn rollback(
    version_id: String,
    event_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<(), String> {
    let aurora = state.read().await;
    aurora
        .rollback(&version_id, &event_id)
        .await
        .map_err(|e| e.to_string())
}

/// 统计该实例 `.old` 备份占用的总字节数，供界面显式告知回滚能力的磁盘代价。
#[tauri::command]
async fn backup_size(version_id: String, state: State<'_, RwLock<Aurora>>) -> Result<u64, String> {
    let aurora = state.read().await;
    aurora
        .backup_size(&version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 分析给定日志文本，产出诊断并与该实例卷宗 join 出可疑文件。
#[tauri::command]
async fn diagnose_crash(
    version_id: String,
    log_text: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<CrashReport, String> {
    let aurora = state.read().await;
    aurora
        .diagnose_crash(&version_id, &log_text)
        .await
        .map_err(|e| e.to_string())
}

/// 读取该实例最近一次归档日志并诊断；从没跑过（无归档）返回 null。
#[tauri::command]
async fn last_crash(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Option<CrashReport>, String> {
    let aurora = state.read().await;
    aurora
        .last_crash(&version_id)
        .await
        .map_err(|e| e.to_string())
}

/// 读取该实例的安装来源卷宗（Mod 身份索引）。
///
/// 卷宗只是索引，磁盘才是权威：界面列已装内容时必须以 `list_mods` 的扫盘结果为骨架，再拿本命令的
/// 结果补身份，绝不能反过来拿卷宗决定文件存不存在。
#[tauri::command]
async fn list_ledger(
    version_id: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<Ledger, String> {
    let aurora = state.read().await;
    aurora
        .ledger_store(&version_id)
        .load()
        .await
        .map_err(|e| e.to_string())
}

// ===== 自定义背景 =====

/// 背景图协议名。Windows 上 WebView 看到的是 `http://aurora-bg.localhost/...`。
const BACKGROUND_SCHEME: &str = "aurora-bg";

/// 当前背景的固定路径。前端拼 `?v=<戳>` 让 WebView 缓存失效。
const BACKGROUND_CURRENT_PATH: &str = "/current";

/// 图库单张图的路径前缀，后接 percent-encoded 文件名。
const BACKGROUND_LIBRARY_PREFIX: &str = "/library/";

/// 内置背景的路径前缀，后接白名单 id（`master` / `arena`）。
///
/// 与图库那条分开而不是复用 `/library/`：内置图不在数据目录里，也没有文件名可言，
/// 混进同一个前缀就得靠「先找文件、找不到再当 id 试试」这种猜法来分辨，那是把两套东西
/// 揉成一个含糊的名字空间——玩家真导入一张叫 `master` 的图时就说不清该给哪张。
const BACKGROUND_BUILTIN_PREFIX: &str = "/builtin/";

/// 主页右下角信息区背后那块图的亮度取样 DTO。两端都按 0..=255 映射相对亮度 0..=1。
#[derive(Serialize)]
struct PlateZoneDto {
    /// 第 10 百分位（偏暗那端）。
    p10: u8,
    /// 第 90 百分位（偏亮那端）。
    p90: u8,
}

/// 界面外观 DTO。
#[derive(Serialize)]
struct AppearanceDto {
    /// 玩家自选背景的文件名；null 表示没自选过，此时用按游戏挑的内置背景。
    background: Option<String>,
    /// 当前背景的平均色，供图加载完成前铺底，避免闪白。
    tint: Option<String>,
    /// 右下角信息区的亮度取样；null 表示这张图还没量过（本功能上线前导入的）。
    plate: Option<PlateZoneDto>,
    /// 纸色遮罩强度（百分比）。
    veil: u8,
    /// 玻璃模式（frost / liquid）。前端据此往 documentElement 写 data-glass。
    glass: GlassMode,
}

fn appearance_dto(aurora: &Aurora) -> AppearanceDto {
    let appearance = &aurora.config().appearance;
    AppearanceDto {
        background: appearance.background.as_ref().map(|b| b.file.clone()),
        tint: appearance.background.as_ref().map(|b| b.tint.clone()),
        plate: appearance
            .background
            .as_ref()
            .and_then(|b| b.plate.as_ref())
            .map(|p| PlateZoneDto {
                p10: p.p10,
                p90: p.p90,
            }),
        veil: appearance.background_veil,
        glass: appearance.glass,
    }
}

/// 读取当前外观设置。
#[tauri::command]
async fn get_appearance(state: State<'_, RwLock<Aurora>>) -> Result<AppearanceDto, String> {
    // 先给老配置补上缺失的取样数据。放在读取外观这一步而不是等玩家重新选图：
    // 那种「顺带补上」的迁移只对恰好去点了那一下的人生效，等于没做。
    {
        let mut aurora = state.write().await;
        if aurora.backfill_plate_zone() {
            aurora.save_config().await.map_err(|e| e.to_string())?;
        }
    }
    let aurora = state.read().await;
    Ok(appearance_dto(&aurora))
}

/// 列出背景图库。
#[tauri::command]
async fn list_backgrounds(
    state: State<'_, RwLock<Aurora>>,
) -> Result<Vec<BackgroundEntry>, String> {
    let aurora = state.read().await;
    aurora.list_backgrounds().await.map_err(|e| e.to_string())
}

/// 导入一张外部图片并立刻设为当前背景。
///
/// 导入与启用合成一步：玩家点「选择图片」的唯一预期就是马上看到它，
/// 分成两个命令只会让界面多一次往返和一个中间态。
#[tauri::command]
async fn import_background(
    path: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AppearanceDto, String> {
    let mut aurora = state.write().await;
    let imported = aurora
        .import_background(&path)
        .await
        .map_err(|e| e.to_string())?;
    aurora
        .set_background(Some(imported.file))
        .map_err(|e| e.to_string())?;
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(appearance_dto(&aurora))
}

/// 切换当前背景；传 null 清掉自选，回到按游戏挑的内置背景。
#[tauri::command]
async fn set_background(
    file: Option<String>,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AppearanceDto, String> {
    let mut aurora = state.write().await;
    aurora.set_background(file).map_err(|e| e.to_string())?;
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(appearance_dto(&aurora))
}

/// 从图库删掉一张图。删的是当前那张时自动清掉自选，回到内置背景。
#[tauri::command]
async fn remove_background(
    file: String,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AppearanceDto, String> {
    let mut aurora = state.write().await;
    aurora
        .remove_background(&file)
        .await
        .map_err(|e| e.to_string())?;
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(appearance_dto(&aurora))
}

/// 设置纸色遮罩强度（超出上限自动钳住）。
#[tauri::command]
async fn set_background_veil(
    veil: u8,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AppearanceDto, String> {
    let mut aurora = state.write().await;
    aurora.set_background_veil(veil);
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(appearance_dto(&aurora))
}

/// 切换玻璃模式（frost / liquid）。
///
/// 与背景走同一份外观配置：它同样是「这台机器上界面长什么样」，不该另立一处存储，
/// 否则搬走 Aurora 文件夹时背景跟着走、玻璃模式留在原机器上。
#[tauri::command]
async fn set_glass_mode(
    glass: GlassMode,
    state: State<'_, RwLock<Aurora>>,
) -> Result<AppearanceDto, String> {
    let mut aurora = state.write().await;
    aurora.set_glass_mode(glass);
    aurora.save_config().await.map_err(|e| e.to_string())?;
    Ok(appearance_dto(&aurora))
}

/// 列出随二进制分发的内置背景（id + 取样值）。
///
/// 玩家没导入过壁纸时界面按当前游戏铺其中一张，取样值决定右下角那撮字用哪一档——
/// 与自选壁纸走的是同一套判定，所以必须由这里下发，前端不能自己对着图估一个。
///
/// 直接返回门面类型而不另立 DTO：它只有 id/tint/plate 三个安全字段，
/// 序列化形态与 [`AppearanceDto`] 的 plate 一致（`list_backgrounds` 同样是直接透传）。
#[tauri::command]
async fn list_builtin_backgrounds() -> Result<Vec<BuiltinBackground>, String> {
    // 首次调用要解码两张 1600x1124 的 JPEG，是 CPU 密集的同步活，不能占着运行时的工作线程；
    // 之后命中门面里的缓存，开销只剩一次克隆。
    tokio::task::spawn_blocking(|| builtin_backgrounds().to_vec())
        .await
        .map_err(|e| e.to_string())
}

/// 协议请求解析出的取图来源。
enum BackgroundSource {
    /// 图库里的一张图，字节要去数据目录读。名字仍是不可信输入。
    Library(String),
    /// 内置背景，字节就在二进制里。
    Builtin(&'static [u8]),
}

impl std::fmt::Debug for BackgroundSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Library(file) => f.debug_tuple("Library").field(file).finish(),
            // 只打长度不打内容：一张内置图有几十万字节，断言失败时会把整个测试输出淹掉。
            Self::Builtin(bytes) => f
                .debug_tuple("Builtin")
                .field(&format_args!("{} 字节", bytes.len()))
                .finish(),
        }
    }
}

impl PartialEq for BackgroundSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Library(a), Self::Library(b)) => a == b,
            // 内置图按「是不是同一份字节」比，逐字节比几十万个只为得到同一个答案。
            (Self::Builtin(a), Self::Builtin(b)) => std::ptr::eq(*a, *b),
            _ => false,
        }
    }
}

/// 把协议请求的路径解析成取图来源。
///
/// `/current` 查配置里的当前背景，`/library/<名>` 取图库指定那张，`/builtin/<id>` 取内置那张。
/// 图库文件名经 percent 解码后仍要过门面的图库校验——这里只负责还原字符串，越界判定是
/// `read_background` 的职责；内置 id 则在这里当场查白名单，因为它压根不对应任何路径，
/// 查不到就是没有这张图。
fn background_source_for(path: &str, current: Option<String>) -> Option<BackgroundSource> {
    if path == BACKGROUND_CURRENT_PATH {
        return current.map(BackgroundSource::Library);
    }
    if let Some(encoded) = path.strip_prefix(BACKGROUND_BUILTIN_PREFIX) {
        let decoded = percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .ok()?;
        return builtin_background_bytes(&decoded).map(BackgroundSource::Builtin);
    }
    let encoded = path.strip_prefix(BACKGROUND_LIBRARY_PREFIX)?;
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .ok()?;
    Some(BackgroundSource::Library(decoded.into_owned()))
}

/// 取图失败时的响应。
///
/// 原因同时写进响应体与日志：`<img>` 标签本身不显示它，DevTools 的网络面板能看到，
/// 而玩家报问题时手上只有日志文件。
fn background_error(status: tauri::http::StatusCode, reason: String) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header(tauri::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(reason.into_bytes())
        .expect("文本响应必定可构造")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 先装 subscriber 再做任何事：配置载入、数据目录探测这些最容易出问题的动作都在下面，
    // 晚一步初始化，恰恰是最想看的那几行日志就丢了。
    let log_path = logging::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        log = ?log_path,
        "Aurora 启动"
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 自更新与「装完重启」。签名校验由插件按 tauri.conf.json 里的 pubkey 做，
        // 私钥只存在于 CI 的 secret 里，本机与仓库都不该有。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // 选背景图要拿磁盘绝对路径，WebView 里的 <input type="file"> 给不了，只能走原生对话框。
        .plugin(tauri_plugin_dialog::init())
        // 自定义背景的取图通道。走专用协议而不是开放 assetProtocol：后者要放开一整片本地路径
        // 才能读到图库目录，而这里只暴露「图库里的一张图」这一件事，文件名还要过门面的越界校验。
        .register_asynchronous_uri_scheme_protocol(BACKGROUND_SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<RwLock<Aurora>>();
                let aurora = state.read().await;
                let current = aurora
                    .config()
                    .appearance
                    .background
                    .as_ref()
                    .map(|b| b.file.clone());
                let Some(source) = background_source_for(&path, current) else {
                    // 没设背景时请求 /current、以及不在白名单里的内置 id，都落这里。
                    responder.respond(background_error(
                        tauri::http::StatusCode::NOT_FOUND,
                        format!("路径 {path} 没有对应的背景图"),
                    ));
                    return;
                };
                let bytes = match source {
                    // 内置图的字节是 'static 的，这里复制一份是 responder 要求所有权；
                    // 有 immutable 缓存头兜着，一张图整个会话只走这一次。
                    BackgroundSource::Builtin(bytes) => bytes.to_vec(),
                    BackgroundSource::Library(file) => match aurora.read_background(&file).await {
                        Ok(bytes) => bytes,
                        // 越界文件名与「图被手动删了」都落这里，对 WebView 一律 404。
                        Err(err) => {
                            tracing::warn!(%file, error = %err, "背景图读取失败");
                            responder.respond(background_error(
                                tauri::http::StatusCode::NOT_FOUND,
                                err.to_string(),
                            ));
                            return;
                        }
                    },
                };
                let response = tauri::http::Response::builder()
                    .header(tauri::http::header::CONTENT_TYPE, "image/jpeg")
                    // 换图靠 URL 上的版本戳失效，内容本身按文件名不可变缓存。
                    .header(
                        tauri::http::header::CACHE_CONTROL,
                        "max-age=31536000, immutable",
                    )
                    .body(bytes)
                    .expect("图片响应必定可构造");
                responder.respond(response);
            });
        })
        .setup(|app| {
            // Aurora::load() 是异步，而 setup 是同步闭包；用 Tauri 运行时 block_on 构造后放进 state。
            // 构造失败（配置损坏等）直接冒泡终止启动，避免带着半初始化的门面继续跑。
            let aurora = tauri::async_runtime::block_on(Aurora::load())?
                .with_launcher_version(env!("CARGO_PKG_VERSION"))?;
            app.manage(RwLock::new(aurora));
            // 运行中的游戏槽（launch_game 存入 kill 句柄、stop_game 取用、监控任务收尾时摘除）。初始空。
            app.manage(RunningGame(Arc::new(Mutex::new(None))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_installed,
            managed_modpack_status,
            managed_modpack_files,
            sync_managed_modpack,
            install_managed_modpack,
            current_account,
            create_offline_account,
            microsoft_login,
            authlib_login,
            list_accounts,
            set_current_account,
            remove_account,
            list_manifest,
            install_version,
            launch_game,
            stop_game,
            detect_java,
            memory_advice,
            install_java,
            update_config,
            set_game_directory,
            search_resources,
            install_mod,
            list_mods,
            set_mod_enabled,
            get_version_settings,
            set_version_settings,
            is_first_run,
            list_game_directories,
            discover_game_directories,
            add_game_directory,
            remove_game_directory,
            switch_game_directory,
            complete_first_run,
            list_mod_versions,
            match_instances,
            plan_install,
            identify_installed_mods,
            check_updates,
            list_history,
            rollback_checks,
            rollback,
            backup_size,
            diagnose_crash,
            last_crash,
            list_ledger,
            get_appearance,
            list_backgrounds,
            list_builtin_backgrounds,
            import_background,
            set_background,
            remove_background,
            set_background_veil,
            set_glass_mode
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modpack_progress_event_keeps_its_structured_payload() {
        let dto = correlated_modpack_event(
            CoreEvent::ModpackSync(ModpackSyncProgress {
                stage: aurora_core::ModpackSyncStage::DownloadingFiles,
                completed_files: 3,
                total_files: 8,
                downloaded_bytes: 4096,
                total_bytes: Some(16384),
                current_file: None,
                download_speed: Some(1048576),
            }),
            "sync-forge-47.4.16",
        );

        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            serde_json::json!({
                "kind": "modpack_sync",
                "operation_id": "sync-forge-47.4.16",
                "progress": {
                    "stage": "downloading_files",
                    "completed_files": 3,
                    "total_files": 8,
                    "downloaded_bytes": 4096,
                    "total_bytes": 16384,
                    "current_file": null,
                    "download_speed": 1048576
                }
            })
        );
    }

    /// 图库来源的简写，让下面的断言只盯路径解析本身。
    fn library(file: &str) -> Option<BackgroundSource> {
        Some(BackgroundSource::Library(file.to_owned()))
    }

    #[test]
    fn current_path_resolves_to_configured_background() {
        assert_eq!(
            background_source_for(BACKGROUND_CURRENT_PATH, Some("雪山.jpg".to_owned())),
            library("雪山.jpg")
        );
        // 没设背景时 /current 无解，协议据此回 404 而不是去读一个空文件名。
        assert_eq!(background_source_for(BACKGROUND_CURRENT_PATH, None), None);
    }

    #[test]
    fn library_path_percent_decodes_filename() {
        // WebView 发出的中文与空格都是 percent-encoded 的，不解码就读不到文件。
        assert_eq!(
            background_source_for("/library/%E9%9B%AA%E5%B1%B1.jpg", None),
            library("雪山.jpg")
        );
        assert_eq!(
            background_source_for("/library/my%20photo.jpg", None),
            library("my photo.jpg")
        );
        // 未编码的普通名字照样能过。
        assert_eq!(
            background_source_for("/library/plain.jpg", None),
            library("plain.jpg")
        );
    }

    #[test]
    fn unknown_paths_have_no_file() {
        // 三个前缀之外的路径一概无解；越界判定仍由门面的 read_background 兜底。
        for path in ["/", "/library", "/other/x.jpg", "/currentx", "/builtin"] {
            assert_eq!(
                background_source_for(path, Some("雪山.jpg".to_owned())),
                None,
                "{path}"
            );
        }
    }

    #[test]
    fn library_path_decodes_traversal_verbatim_for_facade_to_reject() {
        // 这里只还原字符串，不做安全判断——解出 `../config.json` 是对的，
        // 挡下它是 read_background 的职责（见 aurora-core 的 resolve_in_library 测试）。
        assert_eq!(
            background_source_for("/library/..%2Fconfig.json", None),
            library("../config.json")
        );
    }

    #[test]
    fn builtin_path_serves_embedded_bytes() {
        // 内置图不查配置也不碰磁盘：没设背景（current 为 None）时照样出得来，
        // 这正是「没导入过壁纸也有默认背景」赖以成立的一条。
        for id in ["master", "arena"] {
            assert_eq!(
                background_source_for(&format!("/builtin/{id}"), None),
                Some(BackgroundSource::Builtin(
                    builtin_background_bytes(id).expect("已登记的内置背景")
                )),
                "{id}"
            );
        }
    }

    /// 内置背景的 JSON 形态是前端的契约。
    ///
    /// 字段名或 plate 的嵌套一漂，前端读到的就是 undefined，而界面上只表现为
    /// 「默认背景的字色不对」——不抛错、不白屏，没人会往序列化上想。
    #[test]
    fn builtin_background_serializes_with_snake_case_fields() {
        let master = builtin_backgrounds()
            .iter()
            .find(|b| b.id == "master")
            .expect("master 必在登记表里");
        let json = serde_json::to_value(master).expect("序列化");
        assert_eq!(json["id"], "master");
        // tint 只验形态不钉色值：换张图它就变，而这里要守的是契约。
        let tint = json["tint"].as_str().expect("tint 应为字符串");
        assert!(tint.starts_with('#'), "tint 应为 #rrggbb，实际 {tint}");
        assert!(json["plate"]["p10"].is_u64() && json["plate"]["p90"].is_u64());
        // 多出字段同样算契约变更：DTO 里混进新东西时这条会挂，逼着同步前端类型。
        assert_eq!(json.as_object().expect("应为对象").len(), 3);
    }

    #[test]
    fn builtin_path_rejects_unknown_ids_and_traversal() {
        // id 只在白名单里查，从不参与路径拼接。穿越构造在这条链路上不过是查不到的名字——
        // 但仍要把它钉成用例：哪天有人图省事把 id 当文件名去拼路径，这里会先挂。
        for path in [
            "/builtin/../config.json",
            "/builtin/..%2Fconfig.json",
            "/builtin/%2E%2E%2F%2E%2E%2Fconfig.json",
            "/builtin//etc/passwd",
            "/builtin/master.jpg",
            "/builtin/Master",
            "/builtin/",
            "/builtin/nope",
        ] {
            assert_eq!(
                background_source_for(path, Some("雪山.jpg".to_owned())),
                None,
                "{path}"
            );
        }
    }
}
