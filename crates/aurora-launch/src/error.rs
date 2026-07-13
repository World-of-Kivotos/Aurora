//! 本 crate 的错误枚举。
//!
//! 启动链路只在少数几处真正产生错误：classpath 组装时坐标非法、版本缺主类、账户无可用令牌、
//! 子进程 spawn/等待失败，以及委派给 [`aurora_instance`] 的隔离解析冒泡上来的 IO 错误。其余全部
//! 是纯拼装逻辑（不失败）。错误自然向上冒泡，兜底展示归 CLI/前端层。

use std::path::PathBuf;

/// 启动链路错误。
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// 库坐标无法解析为 maven 路径，无法定位其 classpath 条目。
    #[error("库坐标非法，无法定位 classpath 条目：{name}")]
    InvalidLibraryCoordinate {
        /// 出问题的库 name（原始 maven 坐标）。
        name: String,
    },

    /// 版本 JSON 缺少 mainClass，无从确定启动入口。
    #[error("版本 {version} 缺少主类（mainClass），无法确定启动入口")]
    MissingMainClass {
        /// 版本 id。
        version: String,
    },

    /// 账户缺少可用的 Minecraft 访问令牌（微软账户令牌未缓存或已过期）。
    #[error("账户 {name} 缺少可用的 Minecraft 访问令牌，请先完成登录或刷新")]
    MissingAccessToken {
        /// 账户名。
        name: String,
    },

    /// 启动子进程失败。
    #[error("启动游戏进程失败：{program}")]
    Spawn {
        /// 尝试执行的程序（java 可执行文件）路径。
        program: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },

    /// 等待游戏进程结束失败。
    #[error("等待游戏进程结束失败")]
    Wait(#[source] std::io::Error),

    /// 隔离解析（探盘 mods/saves）冒泡上来的实例层错误。
    #[error(transparent)]
    Instance(#[from] aurora_instance::Error),
}

/// 本 crate 的结果别名。
pub type Result<T> = std::result::Result<T, LaunchError>;
