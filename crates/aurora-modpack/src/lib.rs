//! Aurora 受管整合包的纯模型与同步判定。
//!
//! 本阶段刻意把网络、磁盘扫描与下载执行留在边界之外：输入已经实测的磁盘摘要后，三方 diff
//! 是可重复、无副作用的纯函数。唯一的文件 IO 是应用快照存储，并统一复用 aurora-base 原子写入。

pub mod diff;
pub mod error;
pub mod model;
pub mod path;
pub mod snapshot;

pub use diff::{
    DiskFile, DownloadAction, DownloadReason, ForgetAction, KeepAction, KeepReason,
    ManagedDeletion, SyncPlan, diff,
};
pub use error::{Error, Result};
pub use model::{
    FilePolicy, LoaderKind, LoaderSpec, ManifestFile, PackManifest, PackPointer, SCHEMA_VERSION,
    Sha1Digest, Sha1ValidationError,
};
pub use path::{PathValidationError, SafeRelativePath, validate_relative_path};
pub use snapshot::{
    APPLIED_SNAPSHOT_FILE, AppliedSnapshot, SnapshotEntry, SnapshotStore, SnapshotWorkingDirectory,
};
