//! 上次成功应用清单的最小快照及原子存储。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{
    FilePolicy, ManifestFile, PackManifest, SCHEMA_VERSION, Sha1Digest, validate_required_text,
    validate_schema,
};
use crate::path::SafeRelativePath;

/// 与实例 ledger 并列的快照文件名。
pub const APPLIED_SNAPSHOT_FILE: &str = "modpack-applied.json";

const SNAPSHOT_DOCUMENT: &str = "整合包应用快照";

/// 快照所描述文件的实际工作目录。
///
/// 只记录稳定的根类型而非绝对路径：快照本身已经位于对应实例目录，实例 id 与当前游戏目录共同
/// 决定实际路径。该字段必须存在，旧格式快照会拒绝解析，避免把旧根下的删除计划套到新根。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotWorkingDirectory {
    SharedGameDirectory,
    IsolatedVersionDirectory,
}

/// 上次成功同步完成后记录的远端事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedSnapshot {
    pub schema: u32,
    pub pack_id: String,
    pub version: String,
    pub working_directory: SnapshotWorkingDirectory,
    pub files: Vec<SnapshotEntry>,
}

impl AppliedSnapshot {
    /// 从目标清单生成同步成功后应写入的快照。
    pub fn from_manifest(
        manifest: &PackManifest,
        working_directory: SnapshotWorkingDirectory,
    ) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            pack_id: manifest.pack_id.clone(),
            version: manifest.version.clone(),
            working_directory,
            files: manifest.files.iter().map(SnapshotEntry::from).collect(),
        }
    }

    /// 严格解析快照。损坏或未来 schema 必须冒泡，不能静默当成首次安装。
    pub fn from_json_slice(bytes: &[u8]) -> Result<Self> {
        let snapshot: Self = serde_json::from_slice(bytes).map_err(|source| Error::Json {
            document: SNAPSHOT_DOCUMENT,
            source,
        })?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn from_json_str(json: &str) -> Result<Self> {
        Self::from_json_slice(json.as_bytes())
    }

    /// 校验手工构造的快照，尤其防止 Windows 路径别名重复。
    pub fn validate(&self) -> Result<()> {
        validate_schema(SNAPSHOT_DOCUMENT, self.schema)?;
        validate_required_text(SNAPSHOT_DOCUMENT, "pack_id", &self.pack_id)?;
        validate_required_text(SNAPSHOT_DOCUMENT, "version", &self.version)?;

        let mut paths = BTreeSet::new();
        for entry in &self.files {
            let key = entry.path.comparison_key();
            if !paths.insert(key) {
                return Err(Error::DuplicatePath {
                    document: SNAPSHOT_DOCUMENT,
                    path: entry.path.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// 快照只保留 diff 与安全删除所需的字段，不缓存已过期下载地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEntry {
    pub path: SafeRelativePath,
    pub sha1: Sha1Digest,
    pub policy: FilePolicy,
}

impl From<&ManifestFile> for SnapshotEntry {
    fn from(file: &ManifestFile) -> Self {
        Self {
            path: file.path.clone(),
            sha1: file.sha1.clone(),
            policy: file.policy,
        }
    }
}

/// `versions/<id>/.aurora/modpack-applied.json` 的读写句柄。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotStore {
    path: PathBuf,
}

impl SnapshotStore {
    /// 从实例版本目录定位快照。
    pub fn for_version_dir(version_dir: &Path) -> Self {
        Self {
            path: version_dir.join(".aurora").join(APPLIED_SNAPSHOT_FILE),
        }
    }

    /// 指定文件路径，供测试与嵌入方注入。
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 文件缺失表示从未成功同步，返回 `None`；其他 IO 或解析错误完整冒泡。
    pub async fn load(&self) -> Result<Option<AppliedSnapshot>> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => AppliedSnapshot::from_json_slice(&bytes).map(Some),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(aurora_base::Error::Io {
                path: self.path.clone(),
                source,
            }
            .into()),
        }
    }

    /// 验证后序列化，并经 aurora-base 在同目录原子替换。
    pub async fn save(&self, snapshot: &AppliedSnapshot) -> Result<()> {
        snapshot.validate()?;
        let bytes = serde_json::to_vec_pretty(snapshot).map_err(|source| Error::Json {
            document: SNAPSHOT_DOCUMENT,
            source,
        })?;
        aurora_base::fs::atomic_write(&self.path, &bytes).await?;
        Ok(())
    }
}
