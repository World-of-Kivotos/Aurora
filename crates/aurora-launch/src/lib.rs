//! aurora-launch（L3 启动链路）
//!
//! 把「账户 + 已合并版本 JSON + Java + 路径」拼装成一条可执行的启动命令并驱动游戏进程。本 crate 只做拼装与
//! 进程编排，不发网络请求、不下载文件（补全交给 aurora-install，探测交给 aurora-java，登录交给 aurora-auth）。
//!
//! 模块划分：
//! - [`placeholder`]：`${}` 占位符替换引擎（ArgumentReplace）。
//! - [`classpath`]：按合并后 libraries 顺序拼 classpath，客户端主 jar 垫最后。
//! - [`memory`]：内存自动分配、手动滑块换算，产出 `-Xmx`/`-Xms`。
//! - [`args`]：安全/编码防御参数、GC 策略、旧版 JVM 基座、新式条件参数展开、参数分割与去重/覆盖合并。
//! - [`account`]：账户鉴权值摊平与 Authlib-Injector javaagent 拼装。
//! - [`command`]：门面构造器 [`command::CommandBuilder`] -> [`command::LaunchCommand`]。
//! - [`process`]：进程 spawn、stdout/stderr 流式捕获、退出码回报、崩溃触发判定。
//! - [`crash`]：崩溃基础检测规则表与结构化诊断。
//! - [`precheck`]：启动前检查编排。
//! - [`workspace`]：游戏工作目录（版本隔离）解析。
//!
//! 错误统一归口到 [`LaunchError`]，下游可 `#[from] aurora_launch::LaunchError` 冒泡。

pub mod account;
pub mod args;
pub mod classpath;
pub mod command;
pub mod crash;
pub mod error;
pub mod memory;
pub mod placeholder;
pub mod precheck;
pub mod process;
pub mod workspace;

pub use account::{AuthValues, AuthlibInjector, OFFLINE_ACCESS_TOKEN};
pub use args::{GcPolicy, dedup_jvm_args, merge_game_args, split_args};
pub use classpath::{classpath_entries, classpath_separator, classpath_string};
pub use command::{CommandBuilder, GamePaths, LaunchCommand};
pub use crash::{CrashCategory, CrashDiagnosis, analyze, has_crash_marker, primary_cause};
pub use error::{LaunchError, Result};
pub use memory::{MemoryConfig, MemoryTier, auto_allocate, slider_to_mb};
pub use placeholder::Placeholders;
pub use precheck::{CheckItem, CheckStatus, PreLaunchInput, PreLaunchReport};
pub use process::{ExitReport, GameSession, LogLine, LogStream, detect_crash, spawn};
pub use workspace::resolve_game_directory;
