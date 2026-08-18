//! 整合包模型、快照与差集的统一错误。

/// aurora-modpack 对外错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// JSON 语法、字段类型或严格字段约束不成立。
    #[error("{document} JSON 解析失败")]
    Json {
        document: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// 文档 schema 不是当前实现明确支持的版本。
    #[error("{document} schema 不受支持: 期望 {expected}，实际 {actual}")]
    UnsupportedSchema {
        document: &'static str,
        expected: u32,
        actual: u32,
    },

    /// JSON 语法合法，但字段的业务约束不成立。
    #[error("{document} 字段 {field} 非法: {reason}")]
    InvalidField {
        document: &'static str,
        field: String,
        reason: String,
    },

    /// 同一文档内存在会落到同一个 Windows 路径的条目。
    #[error("{document} 中存在重复路径: {path}")]
    DuplicatePath {
        document: &'static str,
        path: String,
    },

    /// 不能拿其他整合包的历史快照参与删除判定。
    #[error("快照属于整合包 {snapshot_pack_id}，当前清单属于 {manifest_pack_id}")]
    SnapshotPackMismatch {
        snapshot_pack_id: String,
        manifest_pack_id: String,
    },

    /// 快照记录的文件根与本次同步解析出的实际工作根不同，不能跨根复用删除事实。
    #[error("快照工作目录为 {snapshot:?}，当前工作目录为 {current:?}")]
    SnapshotWorkingDirectoryMismatch {
        snapshot: crate::snapshot::SnapshotWorkingDirectory,
        current: crate::snapshot::SnapshotWorkingDirectory,
    },

    /// aurora-base 提供的原子文件写入或文件 IO 失败。
    #[error(transparent)]
    Base(#[from] aurora_base::Error),
}

/// crate 级结果别名。
pub type Result<T> = std::result::Result<T, Error>;
