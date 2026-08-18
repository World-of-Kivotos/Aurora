//! 远端清单、上次快照与磁盘实测状态的纯三方差集。

use std::collections::{BTreeMap, BTreeSet};

use crate::error::{Error, Result};
use crate::model::{FilePolicy, ManifestFile, PackManifest, Sha1Digest};
use crate::path::SafeRelativePath;
use crate::snapshot::{AppliedSnapshot, SnapshotEntry, SnapshotWorkingDirectory};

const DISK_INDEX_DOCUMENT: &str = "磁盘文件索引";

/// 对候选路径实测 SHA-1 后得到的磁盘事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFile {
    pub path: SafeRelativePath,
    pub sha1: Sha1Digest,
}

impl DiskFile {
    pub fn new(path: SafeRelativePath, sha1: Sha1Digest) -> Self {
        Self { path, sha1 }
    }
}

/// 下载原因，供 UI 给出准确进度与冲突说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadReason {
    /// 清单新增且磁盘尚无此文件。
    NewFile,
    /// 快照或清单有记录，但磁盘文件缺失。
    MissingFile,
    /// managed 文件的实测摘要与远端不一致，必须恢复服务端版本。
    ManagedHashMismatch,
}

/// 一项待下载文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadAction {
    pub file: ManifestFile,
    pub reason: DownloadReason,
}

/// 不改写磁盘文件的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepReason {
    /// 磁盘已经等于远端目标。
    AlreadyCurrent,
    /// 空快照下发现磁盘文件已等于远端，仅把它收编进新快照。
    Adopted,
    /// seeded 或 optional 已被玩家修改，保留玩家版本。
    PreserveUserModified,
    /// 文件已从远端移除，但其旧策略不允许同步器删除。
    RetiredUserOwned,
}

/// 一项明确保留的磁盘文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepAction {
    pub path: SafeRelativePath,
    pub reason: KeepReason,
}

/// 只从下次快照移除的条目；磁盘本来就不存在，无需执行文件操作。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgetAction {
    pub path: SafeRelativePath,
}

/// 由纯 diff 产生的受管删除候选。
///
/// 字段和构造器均不公开，防止调用方直接把远端路径当作删除项。快照本身是公开 DTO，因此真正
/// 执行删除的边界仍必须从固定 `SnapshotStore` 复核路径、策略和旧摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedDeletion {
    snapshot_entry: SnapshotEntry,
}

impl ManagedDeletion {
    pub fn path(&self) -> &SafeRelativePath {
        &self.snapshot_entry.path
    }

    /// 旧快照摘要仅用于日志和诊断；判定表规定删除不以磁盘仍匹配旧摘要为前提。
    pub fn previous_sha1(&self) -> &Sha1Digest {
        &self.snapshot_entry.sha1
    }

    fn from_snapshot(entry: &SnapshotEntry) -> Self {
        debug_assert_eq!(entry.policy, FilePolicy::Managed);
        Self {
            snapshot_entry: entry.clone(),
        }
    }
}

/// 全部文件操作以及成功后应原子落盘的目标快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlan {
    pub to_download: Vec<DownloadAction>,
    pub to_delete: Vec<ManagedDeletion>,
    pub to_keep: Vec<KeepAction>,
    pub to_forget: Vec<ForgetAction>,
    pub next_snapshot: AppliedSnapshot,
}

impl SyncPlan {
    /// 是否完全不需要改动磁盘文件。即使为真，版本变化时仍可能需要写入 `next_snapshot`。
    pub fn filesystem_is_current(&self) -> bool {
        self.to_download.is_empty() && self.to_delete.is_empty()
    }
}

/// 按设计规格判定表计算同步计划。函数不读取文件、不访问网络，也不修改输入。
pub fn diff(
    manifest: &PackManifest,
    snapshot: Option<&AppliedSnapshot>,
    disk_files: &[DiskFile],
    working_directory: SnapshotWorkingDirectory,
) -> Result<SyncPlan> {
    manifest.validate()?;
    if let Some(snapshot) = snapshot {
        snapshot.validate()?;
        if snapshot.pack_id != manifest.pack_id {
            return Err(Error::SnapshotPackMismatch {
                snapshot_pack_id: snapshot.pack_id.clone(),
                manifest_pack_id: manifest.pack_id.clone(),
            });
        }
        if snapshot.working_directory != working_directory {
            return Err(Error::SnapshotWorkingDirectoryMismatch {
                snapshot: snapshot.working_directory,
                current: working_directory,
            });
        }
    }

    let remote = manifest
        .files
        .iter()
        .map(|file| (file.path.comparison_key(), file))
        .collect::<BTreeMap<_, _>>();
    let previous = snapshot
        .into_iter()
        .flat_map(|snapshot| &snapshot.files)
        .map(|entry| (entry.path.comparison_key(), entry))
        .collect::<BTreeMap<_, _>>();
    let disk = index_disk(disk_files)?;

    let keys = remote
        .keys()
        .chain(previous.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut plan = SyncPlan {
        to_download: Vec::new(),
        to_delete: Vec::new(),
        to_keep: Vec::new(),
        to_forget: Vec::new(),
        next_snapshot: AppliedSnapshot::from_manifest(manifest, working_directory),
    };

    for key in keys {
        classify_path(
            remote.get(&key).copied(),
            previous.get(&key).copied(),
            disk.get(&key).copied(),
            &mut plan,
        );
    }
    Ok(plan)
}

fn index_disk(disk_files: &[DiskFile]) -> Result<BTreeMap<String, &DiskFile>> {
    let mut disk = BTreeMap::new();
    for file in disk_files {
        let key = file.path.comparison_key();
        if disk.insert(key, file).is_some() {
            return Err(Error::DuplicatePath {
                document: DISK_INDEX_DOCUMENT,
                path: file.path.to_string(),
            });
        }
    }
    Ok(disk)
}

fn classify_path(
    remote: Option<&ManifestFile>,
    previous: Option<&SnapshotEntry>,
    disk: Option<&DiskFile>,
    plan: &mut SyncPlan,
) {
    match (remote, previous, disk) {
        (Some(remote), None, None) => download(plan, remote, DownloadReason::NewFile),
        (Some(remote), Some(_), None) => download(plan, remote, DownloadReason::MissingFile),
        (Some(remote), previous, Some(disk)) if disk.sha1 == remote.sha1 => {
            keep(
                plan,
                &remote.path,
                if previous.is_some() {
                    KeepReason::AlreadyCurrent
                } else {
                    KeepReason::Adopted
                },
            );
        }
        (Some(remote), _, Some(_)) if remote.policy == FilePolicy::Managed => {
            download(plan, remote, DownloadReason::ManagedHashMismatch);
        }
        (Some(remote), _, Some(_)) => {
            keep(plan, &remote.path, KeepReason::PreserveUserModified);
        }
        (None, Some(previous), Some(_)) if previous.policy == FilePolicy::Managed => {
            plan.to_delete
                .push(ManagedDeletion::from_snapshot(previous));
        }
        (None, Some(previous), Some(_)) => {
            keep(plan, &previous.path, KeepReason::RetiredUserOwned);
        }
        (None, Some(previous), None) => plan.to_forget.push(ForgetAction {
            path: previous.path.clone(),
        }),
        (None, None, _) => unreachable!("候选键只来自远端清单与旧快照"),
    }
}

fn download(plan: &mut SyncPlan, file: &ManifestFile, reason: DownloadReason) {
    plan.to_download.push(DownloadAction {
        file: file.clone(),
        reason,
    });
}

fn keep(plan: &mut SyncPlan, path: &SafeRelativePath, reason: KeepReason) {
    plan.to_keep.push(KeepAction {
        path: path.clone(),
        reason,
    });
}
