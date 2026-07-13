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

    /// 未配置微软登录 client_id。
    #[error("未配置微软登录 client_id：请在 config.json 设置 msa_client_id 或提供环境变量 AURORA_MSA_CLIENT_ID")]
    MissingClientId,
    /// 请求启动/操作的版本本地未安装。
    #[error("本地未安装版本 {id}")]
    VersionNotInstalled { id: String },
    /// 找不到匹配主版本的 Java，且自动下载被关闭。
    #[error("未找到匹配 Java {major} 的运行时，且自动下载已关闭（开启 auto_download_java 或手动安装对应 Java）")]
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
}

/// 门面层 `Result` 别名，下游用 `#[from] aurora_core::CoreError` 冒泡。
pub type Result<T> = std::result::Result<T, CoreError>;
