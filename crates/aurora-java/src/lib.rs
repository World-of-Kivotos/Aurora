//! L1 Java 运行时管理：探测、按版本需求匹配、自动下载。
//!
//! 三块能力：
//! - [`detect`]：注册表 / 常见目录 / PATH 三路探测本机 Java，逐个 `java -version` 识别成
//!   [`JavaInstallation`]（[`version`] 负责把版本字符串归一成 [`JavaVersion`]）。
//! - [`select`]：按版本 JSON 的 `javaVersion.majorVersion` 挑选最合适的 Java
//!   （主版本正确 > 64 位 > 版本号高）。
//! - [`runtime`]：从 Mojang `java_runtime` 清单下载安装指定主版本的运行时，逐文件 sha1 校验，
//!   远端地址走注入的 HTTP 客户端与 aurora-base 的镜像改写。
//!
//! 错误统一归口到 [`Error`]，下游可用 `#[from] aurora_java::Error` 一处冒泡。

pub mod detect;
pub mod error;
pub mod runtime;
pub mod select;
pub mod version;

pub use detect::{DetectSource, JavaInstallation, detect_all, probe};
pub use error::{Error, Result};
pub use runtime::{
    InstalledRuntime, JavaRuntimeInstaller, MOJANG_JAVA_RUNTIME_ALL, current_platform,
};
pub use select::{rank_for_major, select_for_major};
pub use version::{JavaVersion, ProbedJava, parse_java_version_output};
