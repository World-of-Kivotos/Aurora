//! Aurora Tauri 外壳的 IPC 层。
//!
//! 只承担一件事：把 aurora-core 门面（[`Aurora`]）经 `#[tauri::command]` 暴露给 React 前端。
//! 三条纪律贯穿全文件：
//! 1. 门面放进 managed state，用 `tokio::sync::Mutex` 包裹——`Aurora` 兼有 `&self` 异步方法与
//!    `set_client_id`/`set_game_dir` 这类 `&mut self` 方法，异步 Mutex 才能把两类方法都用上。
//! 2. 绝不把 aurora-core 原始类型（尤其含登录令牌的 [`Account`]）整体过 IPC；命令一律返回本文件
//!    定义的瘦 DTO，只映射前端需要的安全字段。
//! 3. 进度/事件走一条固定范式：命令内建一个 `tokio::mpsc` 通道作为门面的 [`EventSink`]，另起一个
//!    转发任务把 [`CoreEvent`] 逐条 `emit` 成 Tauri 事件推给前端（见 `create_offline_account`）。

use aurora_core::{
    Account, AccountType, Aurora, CoreEvent, DownloadSourcePolicy, IsolationPolicy, MemorySettings,
    VersionScan,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

/// 前端订阅进度事件的统一事件名。install/launch 等后续长任务照抄本范式时复用同一事件名，
/// 前端按负载里的 `kind` 区分阶段/告警/下载进度。
const CORE_EVENT: &str = "aurora://core-event";

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

// ===== IPC 命令 =====
//
// 全部为 async：借用 managed state 的命令必须返回 Result（Tauri 对借用 State 的异步命令的硬性要求），
// 内部一律 `state.lock().await` 取门面。CoreError 经 `to_string()` 转成字符串上抛，让前端能显示；
// 不在命令里 try/catch 生吞。

/// 读取全局配置（含游戏目录、内存、下载源策略、是否已配 client_id 等）。
#[tauri::command]
async fn get_config(state: State<'_, Mutex<Aurora>>) -> Result<ConfigDto, String> {
    let aurora = state.lock().await;
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
    })
}

/// 扫描游戏目录下已安装的版本（含损坏版本单列）。
#[tauri::command]
async fn list_installed(state: State<'_, Mutex<Aurora>>) -> Result<VersionScanDto, String> {
    let aurora = state.lock().await;
    let scan = aurora.list_installed().await.map_err(|e| e.to_string())?;
    Ok(scan_dto(scan))
}

/// 读取当前选中账户（可能没有）。
#[tauri::command]
async fn current_account(state: State<'_, Mutex<Aurora>>) -> Result<Option<AccountDto>, String> {
    let aurora = state.lock().await;
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
    state: State<'_, Mutex<Aurora>>,
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
        let aurora = state.lock().await;
        aurora
            .create_offline_account(&name, Some(&tx))
            .map_err(|e| e.to_string())?
    };

    // 丢弃唯一的 sender，让桥接任务排空剩余事件后退出，再等它 join。
    drop(tx);
    let _ = forward_task.await;

    Ok(account_dto(&account))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Aurora::load() 是异步，而 setup 是同步闭包；用 Tauri 运行时 block_on 构造后放进 state。
            // 构造失败（配置损坏等）直接冒泡终止启动，避免带着半初始化的门面继续跑。
            let aurora = tauri::async_runtime::block_on(Aurora::load())?;
            app.manage(Mutex::new(aurora));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            list_installed,
            current_account,
            create_offline_account
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
