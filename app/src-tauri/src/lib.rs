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

use std::path::PathBuf;
use std::sync::Arc;

use aurora_core::{
    detect_crash, Account, AccountType, AggregateResult, Aurora, CoreEvent, CrashReport,
    DetectSource, DeviceCodeResponse, DownloadSourcePolicy, GameSession, History, InstallPlan,
    InstalledMod, InstanceMatch, IsolationOverride, IsolationPolicy, JavaInstallation, JavaVersion,
    LaunchOptions, Ledger, LoaderChoice, LogLine, LogStream, MemorySettings, ModLoader,
    ModVersionInfo, Platform, ResolvedIsolation, ResourceType, RollbackCheck, SearchHit,
    SearchQuery, SortField, UpdateCandidate, VersionManifest, VersionScan, VersionSettings,
};
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
        }
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

/// 读取当前选中账户。Windows 走加密账户库；非 Windows 无凭据库实现，返回 None 以保证跨平台可编译。
#[cfg(windows)]
fn read_current_account(aurora: &Aurora) -> Result<Option<AccountDto>, String> {
    let current = aurora.current_account().map_err(|e| e.to_string())?;
    Ok(current.as_ref().map(account_dto))
}

#[cfg(not(windows))]
fn read_current_account(_aurora: &Aurora) -> Result<Option<AccountDto>, String> {
    Ok(None)
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
// 微软/authlib 登录与账户读写在门面上均 `#[cfg(windows)]`。这里照 read_current_account 的兜底范式做
// 跨平台缝：非 Windows 下读操作返回空，写/登录操作明确报“平台不受支持”，绝不静默假装成功。

#[cfg(not(windows))]
const WINDOWS_ONLY: &str = "该操作在当前平台不受支持（账户凭据加密仅限 Windows）";

#[cfg(windows)]
fn list_accounts_impl(aurora: &Aurora) -> Result<Vec<AccountDto>, String> {
    let accounts = aurora.accounts().map_err(|e| e.to_string())?;
    Ok(accounts.iter().map(account_dto).collect())
}

#[cfg(not(windows))]
fn list_accounts_impl(_aurora: &Aurora) -> Result<Vec<AccountDto>, String> {
    Ok(Vec::new())
}

#[cfg(windows)]
fn set_current_account_impl(aurora: &Aurora, uuid: &str) -> Result<(), String> {
    aurora.set_current_account(uuid).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn set_current_account_impl(_aurora: &Aurora, _uuid: &str) -> Result<(), String> {
    Err(WINDOWS_ONLY.to_owned())
}

#[cfg(windows)]
fn remove_account_impl(aurora: &Aurora, uuid: &str) -> Result<(), String> {
    aurora.remove_account(uuid).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn remove_account_impl(_aurora: &Aurora, _uuid: &str) -> Result<(), String> {
    Err(WINDOWS_ONLY.to_owned())
}

/// 按 uuid 取出完整账户（含令牌，仅供内部传给 launch_account，绝不过 IPC）。
#[cfg(windows)]
fn find_account_impl(aurora: &Aurora, uuid: &str) -> Result<Account, String> {
    aurora
        .accounts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.uuid == uuid)
        .ok_or_else(|| format!("账户 {uuid} 不存在"))
}

#[cfg(not(windows))]
fn find_account_impl(_aurora: &Aurora, _uuid: &str) -> Result<Account, String> {
    Err(WINDOWS_ONLY.to_owned())
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

/// 扫描游戏目录下已安装的版本（含损坏版本单列）。
#[tauri::command]
async fn list_installed(state: State<'_, RwLock<Aurora>>) -> Result<VersionScanDto, String> {
    let aurora = state.read().await;
    let scan = aurora.list_installed().await.map_err(|e| e.to_string())?;
    Ok(scan_dto(scan))
}

/// 读取当前选中账户（可能没有）。
#[tauri::command]
async fn current_account(state: State<'_, RwLock<Aurora>>) -> Result<Option<AccountDto>, String> {
    let aurora = state.read().await;
    read_current_account(&aurora)
}

/// 创建一个离线账户，并示范“进度事件流”范式。
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
            .create_offline_account(&name, Some(&tx))
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
    list_accounts_impl(&aurora)
}

/// 切换当前选中账户。
#[tauri::command]
async fn set_current_account(uuid: String, state: State<'_, RwLock<Aurora>>) -> Result<(), String> {
    let aurora = state.read().await;
    set_current_account_impl(&aurora, &uuid)
}

/// 删除账户。
#[tauri::command]
async fn remove_account(uuid: String, state: State<'_, RwLock<Aurora>>) -> Result<(), String> {
    let aurora = state.read().await;
    remove_account_impl(&aurora, &uuid)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Aurora::load() 是异步，而 setup 是同步闭包；用 Tauri 运行时 block_on 构造后放进 state。
            // 构造失败（配置损坏等）直接冒泡终止启动，避免带着半初始化的门面继续跑。
            let aurora = tauri::async_runtime::block_on(Aurora::load())?;
            app.manage(RwLock::new(aurora));
            // 运行中的游戏槽（launch_game 存入 kill 句柄、stop_game 取用、监控任务收尾时摘除）。初始空。
            app.manage(RunningGame(Arc::new(Mutex::new(None))));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_installed,
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
            install_java,
            update_config,
            set_game_directory,
            search_resources,
            install_mod,
            list_mods,
            set_mod_enabled,
            get_version_settings,
            set_version_settings,
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
            list_ledger
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
