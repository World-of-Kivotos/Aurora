//! aurora-core（L4 门面）
//!
//! 组合下层各 crate，向 iced 前端与 CLI 暴露一套粗粒度异步 API：版本清单/发现、版本安装（原版 +
//! Fabric/Quilt）、微软/离线账户、离线启动、资源聚合搜索，并持有全局配置（config.json）与统一的
//! 进度/事件通道。目标是让上层只依赖本 crate 即可完成端到端流程。
//!
//! 结构：
//! - [`config`]：全局配置与下载源三档策略。
//! - [`error`]：门面统一错误 [`CoreError`]，各下层错误 `#[from]` 冒泡。
//! - [`event`]：进度/事件模型（[`CoreEvent`] / [`EventSink`] / [`DownloadProgress`]）。
//! - [`facade`]：门面结构 [`Aurora`] 与共享装配；各操作分散在 `versions`/`install`/`auth`/`launch`/`search`。
//!
//! Mod 治理这一组（下载页落位、依赖、更新、回滚、崩溃归因）共用一条地基：`ledger` 是 Mod 身份的
//! 单一来源，但磁盘始终是权威、卷宗只是索引。围绕它展开的是
//! [`modversions`]（跨平台版本列表）、[`compat`]（兼容判定与实例匹配）、[`deps`]（依赖图与安装计划）、
//! [`updates`]（哈希反查与更新检查）、[`history`]（变更历史与回滚）、[`crashdiag`]（崩溃归因）。
//!
//! ```no_run
//! # async fn demo() -> aurora_core::Result<()> {
//! use aurora_core::Aurora;
//! let aurora = Aurora::load().await?;
//! let manifest = aurora.list_manifest().await?;
//! println!("最新正式版：{}", manifest.latest.release);
//! # Ok(())
//! # }
//! ```

pub mod auth;
pub mod background;
pub mod compat;
pub mod config;
pub mod crashdiag;
pub mod deps;
pub mod error;
pub mod event;
pub mod facade;
pub mod history;
pub mod folders;
pub mod install;
pub mod instance;
pub mod java;
pub mod launch;
pub mod ledger;
pub mod mods;
pub mod modversions;
pub mod search;
pub mod updates;
pub mod versions;

// ---- 门面自身的公开类型 ----
pub use background::{BACKGROUNDS_DIR, BackgroundEntry};
pub use compat::{Compatibility, InstanceMatch, classify};
pub use config::{
    AppearanceSettings, AuroraConfig, BackgroundRef, ConfigStore, DownloadSourcePolicy,
    MAX_BACKGROUND_VEIL, MemorySettings, NamedDirectory,
};
pub use crashdiag::{CrashReport, CrashSuspect};
pub use deps::{InstallPlan, PlannedItem};
pub use error::{CoreError, Result};
pub use event::{CoreEvent, DownloadProgress, EventSink};
pub use facade::Aurora;
pub use history::{History, HistoryEvent, HistoryStore, RollbackCheck};
pub use install::{InstallOutcome, LoaderChoice};
pub use launch::LaunchOptions;
pub use ledger::{Ledger, LedgerEntry, LedgerStore};
pub use mods::ModInstallOutcome;
pub use modversions::sort_by_published_desc;
pub use updates::UpdateCandidate;

pub use auth::{MSA_CLIENT_ID_ENV, perform_microsoft_login};

// ---- 透传下层类型，让上层只依赖本 crate ----
pub use aurora_auth::{Account, AccountType, DeviceCodeResponse, GameProfile};
pub use aurora_install::{LoaderSummary, VanillaSummary};
pub use aurora_instance::{
    BrokenReason, BrokenVersion, DiscoveredVersion, GameDirectory, GameDirectorySource,
    IsolationOverride, IsolationPolicy, ResolvedIsolation, VersionScan, VersionSettings,
};
pub use aurora_java::{DetectSource, InstalledRuntime, JavaInstallation, JavaVersion};
pub use aurora_launch::{
    CrashCategory, CrashDiagnosis, ExitReport, GameSession, LogLine, LogStream, analyze,
    detect_crash,
};
pub use aurora_modplatform::{
    AggregateResult, DependencyKind, InstalledMod, MetadataFormat, ModDependency, ModLoader,
    ModMetadata, ModVersionInfo, Platform, PlatformError, ReleaseChannel, ResourceType, SearchHit,
    SearchQuery, SortField, parse_loader_name,
};
pub use aurora_version::{LoaderInfo, LoaderKind, ManifestVersion, VersionManifest};
