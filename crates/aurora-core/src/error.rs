//! 门面层错误枚举。
//!
//! aurora-core 组合各下层 crate，故其错误既向下透传各 crate 的独立错误（`#[from]` 冒泡，不吞不掩），
//! 也补充门面自身的失败：配置读写/解析、缺失微软 client_id、目标版本未安装、无匹配 Java 等。
//! 统一收口后由更上层（aurora-cli / 未来前端）做兜底展示。

use std::path::PathBuf;

/// 门面层统一错误。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 下层公共设施错误（HTTP 构建、镜像改写、文件校验、目录定位）。
    #[error(transparent)]
    Base(#[from] aurora_base::Error),
    /// 版本 JSON 解析/继承合并错误。
    #[error(transparent)]
    Version(#[from] aurora_version::Error),
    /// 下载引擎错误。
    #[error(transparent)]
    Download(#[from] aurora_download::Error),
    /// 安装（原版/加载器补全）错误。
    #[error(transparent)]
    Install(#[from] aurora_install::Error),
    /// 实例（目录/版本发现/隔离）错误。
    #[error(transparent)]
    Instance(#[from] aurora_instance::Error),
    /// Java 探测/自动下载错误。
    #[error(transparent)]
    Java(#[from] aurora_java::Error),
    /// 账户/登录错误。
    #[error(transparent)]
    Auth(#[from] aurora_auth::AuthError),
    /// 启动链路错误。
    #[error(transparent)]
    Launch(#[from] aurora_launch::LaunchError),
    /// Mod 平台错误（Modrinth/CurseForge 客户端、本地模组扫描与启禁）。
    #[error(transparent)]
    ModPlatform(#[from] aurora_modplatform::Error),
    /// 整合包清单、路径、差集或快照错误。
    #[error(transparent)]
    Modpack(#[from] aurora_modpack::Error),

    /// 读取配置文件失败。
    #[error("读取配置文件 {path} 失败")]
    ConfigIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 配置文件内容非法。
    #[error("解析配置文件 {path} 失败")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    /// 配置序列化失败。
    #[error("序列化配置失败")]
    ConfigSerialize(#[source] serde_json::Error),
    /// 整合包订阅字段非法。路径用于区分多实例下具体是哪份订阅损坏。
    #[error("整合包订阅非法: {path}: {reason}")]
    InvalidModpackSubscription { path: PathBuf, reason: &'static str },
    /// 整合包元数据端点暂时不可用，且没有可用缓存。
    #[error("整合包元数据不可用: {url}: {detail}")]
    ModpackRemoteUnavailable { url: String, detail: String },
    /// 指针、订阅、清单或快照之间的身份/版本契约不一致。
    #[error("整合包元数据冲突: {detail}")]
    ModpackMetadataConflict { detail: String },
    /// 调用产品注入的启动器版本不是合法 semver。
    #[error("启动器版本 {version} 无法按 semver 解析: {detail}")]
    InvalidLauncherVersion { version: String, detail: String },
    /// 服务端要求的启动器版本高于当前版本，必须明确拒绝解析后续清单。
    #[error("整合包要求 Aurora >= {required}，当前版本为 {current}，请先升级启动器")]
    ModpackLauncherTooOld { current: String, required: String },
    /// 对普通实例调用了仅受管实例可用的整合包操作。
    #[error("实例 {version_id} 未订阅整合包")]
    ModpackNotManaged { version_id: String },
    /// 受管实例的快照绑定实际工作根，不能在仍受管时切换隔离策略。
    #[error("受管整合包实例 {version_id} 的隔离设置已锁定；请先解除受管状态或重新安装实例")]
    ManagedModpackIsolationLocked { version_id: String },
    /// 清单声明了当前安装门面尚不支持的加载器。
    #[error("整合包加载器 {loader} 暂不支持一键安装")]
    UnsupportedModpackLoader { loader: &'static str },
    /// 清单路径虽通过词法校验，但命中了玩家/启动器保留域或既有符号链接/reparse point。
    #[error("拒绝整合包路径 {path}: {reason}")]
    UnsafeModpackPath { path: String, reason: String },

    /// 未配置微软登录 client_id。
    #[error(
        "未配置微软登录 client_id：请在 config.json 设置 msa_client_id 或提供环境变量 AURORA_MSA_CLIENT_ID"
    )]
    MissingClientId,
    /// 请求启动/操作的版本本地未安装。
    #[error("本地未安装版本 {id}")]
    VersionNotInstalled { id: String },
    /// 请求安装的模组版本/文件在平台上不存在。
    #[error("平台 {platform} 上工程 {project_id} 未找到版本 {version_id}")]
    ModVersionNotFound {
        platform: &'static str,
        project_id: String,
        version_id: String,
    },
    /// 找不到匹配主版本的 Java，且自动下载被关闭。
    #[error(
        "未找到匹配 Java {major} 的运行时，且自动下载已关闭（开启 auto_download_java 或手动安装对应 Java）"
    )]
    NoJava { major: u32 },
    /// 启动前检查存在阻断项，已中止启动。
    #[error("启动前检查未通过：{0}")]
    PrecheckFailed(String),
    /// 后台阻塞任务异常结束（如 Java 探测子任务 panic）。
    #[error("后台任务异常结束")]
    TaskJoin(#[from] tokio::task::JoinError),
    /// 该操作在当前平台不受支持（微软登录凭据加密仅限 Windows）。
    #[error("该操作在当前平台不受支持（微软登录的凭据加密仅限 Windows）")]
    PlatformUnsupported,

    /// 背景图库的文件读写失败。
    #[error("读写背景图 {path} 失败")]
    BackgroundIo {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// 背景图无法解码（格式不支持或文件损坏）。
    #[error("无法解析背景图 {path}：{reason}")]
    BackgroundDecode { path: PathBuf, reason: String },
    /// 背景文件名越界。文件名来自 WebView，越界即视为攻击面而不是笔误。
    #[error("背景文件名 {file} 不合法：只接受图库目录下的单个文件名")]
    BackgroundName { file: String },
    /// 指定的背景不在图库里。
    #[error("图库里没有背景 {file}")]
    BackgroundNotFound { file: String },
}

/// 门面层 `Result` 别名，下游用 `#[from] aurora_core::CoreError` 冒泡。
pub type Result<T> = std::result::Result<T, CoreError>;
