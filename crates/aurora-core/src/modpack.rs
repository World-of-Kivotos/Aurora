//! 受管整合包的远端元数据缓存、三方差集采集与安全同步编排。

use std::collections::{BTreeMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use aurora_download::{DownloadProgress, DownloadTask, TaskFailure};
use aurora_install::{LoaderInstaller, forge_installer_url, neoforge_installer_url};
use aurora_instance::VERSIONS_DIR;
use aurora_modpack::{
    AppliedSnapshot, DiskFile, FilePolicy, ManagedDeletion, PackManifest, PackPointer,
    SafeRelativePath, Sha1Digest, SnapshotStore, SnapshotWorkingDirectory, diff,
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::error::{CoreError, Result};
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::{Aurora, make_context};
use crate::install::LoaderChoice;
use crate::subscription::ModpackSubscription;

const POINTER_CACHE_FILE: &str = "modpack-latest-cache.json";
const MANIFEST_CACHE_FILE: &str = "modpack-manifest-cache.json";
const INSTALL_RESERVATION_DIR: &str = ".aurora/modpack-install-reservations";
const INSTALL_RESERVATION_SCHEMA: u32 = 2;
const INSTALL_LOCK_FILE: &str = ".aurora/install.lock";
const INSTALL_STAGING_DIR: &str = ".aurora/install-staging";
const INSTALLER_CACHE_DIR: &str = ".aurora/installer-cache";
const INSTALL_OWNER_FILE: &str = ".aurora/install-owner.json";
const MAX_INSTALL_PROFILE_BYTES: u64 = 4 * 1024 * 1024;
static ACTIVE_MANAGED_INSTALLS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// 已成功应用的版本与服务端当前发布版本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KnownModpackVersions {
    pub installed_version: Option<String>,
    pub latest: PackPointer,
}

/// 指针来源。缓存只在网络请求失败时使用；不兼容或非法的新响应不会被旧缓存掩盖。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModpackCacheSource {
    Network,
    Cache,
}

/// 一次受管实例版本检查的结构化结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManagedModpackStatus {
    Ready {
        subscription: ModpackSubscription,
        versions: KnownModpackVersions,
        source: ModpackCacheSource,
        checked_at: String,
    },
    Unavailable {
        subscription: ModpackSubscription,
        last_known: Option<KnownModpackVersions>,
        detail: String,
    },
}

/// 同步执行阶段。字段名与前端事件契约保持 snake_case。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModpackSyncStage {
    ResolvingManifest,
    InstallingMinecraft,
    InstallingLoader,
    DownloadingFiles,
    DeletingFiles,
    WritingSnapshot,
}

/// 同步进度。下载引擎不虚构“当前文件”，并发下载期间该字段为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModpackSyncProgress {
    pub stage: ModpackSyncStage,
    pub completed_files: u64,
    pub total_files: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub current_file: Option<String>,
}

/// 可直接交给 UI 呈现的失败事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModpackSyncFailure {
    Network {
        file_path: String,
        detail: String,
    },
    ChecksumMismatch {
        file_path: String,
        expected_sha1: String,
        actual_sha1: String,
    },
    DiskFull {
        file_path: String,
        required_bytes: Option<u64>,
        available_bytes: Option<u64>,
    },
    PermissionDenied {
        file_path: String,
        detail: String,
    },
    SnapshotWrite {
        file_path: String,
        detail: String,
    },
    InvalidMetadata {
        detail: String,
    },
    LauncherTooOld {
        current: String,
        required: String,
    },
    Conflict {
        detail: String,
    },
    Filesystem {
        file_path: String,
        detail: String,
    },
}

/// 带目标版本与阶段的同步错误，避免 Tauri 只能拿到一条无上下文字符串。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("整合包 {target_version} 同步在 {stage:?} 阶段失败: {failure:?}")]
pub struct ModpackSyncError {
    pub target_version: String,
    pub stage: ModpackSyncStage,
    pub failure: ModpackSyncFailure,
}

/// 同步成功摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModpackSyncOutcome {
    pub installed_version: String,
    pub downloaded_files: usize,
    pub deleted_files: usize,
    pub kept_files: usize,
}

/// 首次安装成功摘要。完整安装入口复用 [`Aurora::install`]，不复制原版或加载器安装逻辑。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModpackInstallOutcome {
    pub instance_id: String,
    pub installed_version: String,
}

/// 成功快照中的文件归属。只有 `managed` 条目必须由界面禁止单独开关或移除。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedModpackFile {
    pub path: String,
    pub policy: FilePolicy,
}

#[derive(Debug)]
struct RemoteUnavailable {
    url: String,
    detail: String,
}

#[derive(Debug)]
enum RemoteDocumentError {
    Unavailable(RemoteUnavailable),
    Invalid(CoreError),
}

#[derive(Debug)]
struct PlannedInstall {
    instance_id: String,
    prepared_installer: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModpackInstallReservation {
    schema: u32,
    operation_id: String,
    instance_id: String,
    pack_id: String,
    pointer_url: String,
    pack_version: String,
    manifest_url: String,
    minecraft: String,
    loader_kind: aurora_modpack::LoaderKind,
    loader_version: String,
    install_minecraft: bool,
}

impl ModpackInstallReservation {
    fn new(
        instance_id: &str,
        subscription: &ModpackSubscription,
        pointer: &PackPointer,
        manifest: &PackManifest,
        install_minecraft: bool,
    ) -> Self {
        Self {
            schema: INSTALL_RESERVATION_SCHEMA,
            operation_id: new_install_operation_id(),
            instance_id: instance_id.to_owned(),
            pack_id: subscription.pack_id.clone(),
            pointer_url: subscription.pointer_url.clone(),
            pack_version: pointer.version.clone(),
            manifest_url: pointer.manifest_url.clone(),
            minecraft: manifest.minecraft.clone(),
            loader_kind: manifest.loader.kind,
            loader_version: manifest.loader.version.clone(),
            install_minecraft,
        }
    }

    fn matches_environment(&self, other: &Self) -> bool {
        self.schema == other.schema
            && self.instance_id == other.instance_id
            && self.pack_id == other.pack_id
            && self.pointer_url == other.pointer_url
            && self.minecraft == other.minecraft
            && self.loader_kind == other.loader_kind
            && self.loader_version == other.loader_version
    }

    fn validate(&self) -> Result<()> {
        if self.schema != INSTALL_RESERVATION_SCHEMA {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "install reservation schema {} is unsupported; expected {}",
                    self.schema, INSTALL_RESERVATION_SCHEMA
                ),
            });
        }
        validate_instance_id(&self.instance_id)?;
        validate_runtime_component("operation_id", &self.operation_id)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionDirectoryOwner {
    schema: u32,
    operation_id: String,
    instance_id: String,
    directory_id: String,
}

impl VersionDirectoryOwner {
    fn new(reservation: &ModpackInstallReservation, directory_id: &str) -> Self {
        Self {
            schema: INSTALL_RESERVATION_SCHEMA,
            operation_id: reservation.operation_id.clone(),
            instance_id: reservation.instance_id.clone(),
            directory_id: directory_id.to_owned(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema != INSTALL_RESERVATION_SCHEMA {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "install owner schema {} is unsupported; expected {}",
                    self.schema, INSTALL_RESERVATION_SCHEMA
                ),
            });
        }
        validate_runtime_component("operation_id", &self.operation_id)?;
        validate_instance_id(&self.instance_id)?;
        validate_instance_id(&self.directory_id)?;
        Ok(())
    }

    fn matches(&self, reservation: &ModpackInstallReservation, directory_id: &str) -> bool {
        self.schema == reservation.schema
            && self.operation_id == reservation.operation_id
            && self.instance_id == reservation.instance_id
            && self.directory_id == directory_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallReservationState {
    Created,
    Existing,
}

#[derive(Debug)]
pub(crate) struct ActiveManagedInstall {
    game_dir: PathBuf,
    _file: File,
}

impl Drop for ActiveManagedInstall {
    fn drop(&mut self) {
        let Some(active) = ACTIVE_MANAGED_INSTALLS.get() else {
            return;
        };
        let mut active = active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        active.remove(&self.game_dir);
    }
}

#[derive(Debug, Deserialize)]
struct InstallerProfileIdentity {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    json: Option<String>,
    #[serde(default)]
    minecraft: Option<String>,
    #[serde(default)]
    processors: Vec<serde_json::Value>,
    #[serde(default)]
    install: Option<serde_json::Value>,
    #[serde(rename = "versionInfo", default)]
    version_info: Option<InstallerVersionIdentity>,
}

#[derive(Debug, Deserialize)]
struct InstallerVersionIdentity {
    id: String,
    #[serde(rename = "inheritsFrom", default)]
    inherits_from: Option<String>,
}

impl Aurora {
    /// 检查受管实例的当前发布版本。普通实例返回 `None`。
    ///
    /// 网络不可用时回退到上次成功缓存；服务端返回非法文档、身份冲突或要求更高启动器版本时明确
    /// 返回 `unavailable`，绝不拿旧缓存伪装成这次检查成功。
    pub async fn managed_modpack_status(
        &self,
        version_id: &str,
    ) -> Result<Option<ManagedModpackStatus>> {
        if let Some(subscription) = self
            .pending_managed_install_subscription(version_id)
            .await?
        {
            return Ok(Some(ManagedModpackStatus::Unavailable {
                subscription,
                last_known: None,
                detail: "managed modpack installation is incomplete and must be resumed before this instance can be used".to_owned(),
            }));
        }
        let (version_dir, subscription) = self.checked_modpack_subscription(version_id).await?;
        let Some(subscription) = subscription else {
            return Ok(None);
        };
        let installed_version = match self
            .checked_managed_snapshot(version_id, &version_dir, &subscription)
            .await
        {
            Ok(snapshot) => snapshot.map(|snapshot| snapshot.version),
            Err(err) => {
                return Ok(Some(ManagedModpackStatus::Unavailable {
                    subscription,
                    last_known: None,
                    detail: err.to_string(),
                }));
            }
        };
        match self.fetch_pointer(&subscription.pointer_url).await {
            Ok(pointer) => {
                if let Err(err) = validate_pointer(&pointer, &subscription, self.launcher_version())
                {
                    let (last_known, cache_detail) = cached_versions(
                        self.game_dir(),
                        &version_dir,
                        &subscription,
                        installed_version,
                        self.launcher_version(),
                    )
                    .await;
                    return Ok(Some(ManagedModpackStatus::Unavailable {
                        subscription,
                        last_known,
                        detail: append_cache_detail(err.to_string(), cache_detail),
                    }));
                }
                save_instance_cache(self.game_dir(), &version_dir, POINTER_CACHE_FILE, &pointer)
                    .await?;
                Ok(Some(ManagedModpackStatus::Ready {
                    subscription,
                    versions: KnownModpackVersions {
                        installed_version,
                        latest: pointer,
                    },
                    source: ModpackCacheSource::Network,
                    checked_at: checked_at()?,
                }))
            }
            Err(RemoteDocumentError::Unavailable(remote)) => {
                let (cached, cache_detail) = cached_versions(
                    self.game_dir(),
                    &version_dir,
                    &subscription,
                    installed_version,
                    self.launcher_version(),
                )
                .await;
                match cached {
                    Some(versions) => Ok(Some(ManagedModpackStatus::Ready {
                        subscription,
                        versions,
                        source: ModpackCacheSource::Cache,
                        checked_at: checked_at()?,
                    })),
                    None => Ok(Some(ManagedModpackStatus::Unavailable {
                        subscription,
                        last_known: None,
                        detail: append_cache_detail(
                            format!("{}: {}", remote.url, remote.detail),
                            cache_detail,
                        ),
                    })),
                }
            }
            Err(RemoteDocumentError::Invalid(err)) => {
                let (last_known, cache_detail) = cached_versions(
                    self.game_dir(),
                    &version_dir,
                    &subscription,
                    installed_version,
                    self.launcher_version(),
                )
                .await;
                Ok(Some(ManagedModpackStatus::Unavailable {
                    subscription,
                    last_known,
                    detail: append_cache_detail(err.to_string(), cache_detail),
                }))
            }
        }
    }

    /// 读取成功快照中的文件归属。普通实例返回 `None`，尚未成功同步的受管实例返回空列表。
    pub async fn managed_modpack_files(
        &self,
        version_id: &str,
    ) -> Result<Option<Vec<ManagedModpackFile>>> {
        if self
            .pending_managed_install_subscription(version_id)
            .await?
            .is_some()
        {
            return Ok(Some(Vec::new()));
        }
        let (version_dir, subscription) = self.checked_modpack_subscription(version_id).await?;
        let Some(subscription) = subscription else {
            return Ok(None);
        };
        let Some(snapshot) = self
            .checked_managed_snapshot(version_id, &version_dir, &subscription)
            .await?
        else {
            return Ok(Some(Vec::new()));
        };
        if snapshot.pack_id != subscription.pack_id {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "实例订阅 {}，成功快照却属于 {}",
                    subscription.pack_id, snapshot.pack_id
                ),
            });
        }
        Ok(Some(
            snapshot
                .files
                .into_iter()
                .map(|entry| ManagedModpackFile {
                    path: entry.path.to_string(),
                    policy: entry.policy,
                })
                .collect(),
        ))
    }

    /// 从 latest 指针完成原版、加载器、订阅与空快照同步的一键安装。
    ///
    /// 原版和加载器安装分别复用 [`Aurora::install_vanilla_component`] 与
    /// [`Aurora::install_loader_component`]；若文件同步失败，已创建的受管实例会保留，调用方可按返回
    /// 的结构化失败刷新实例列表后重试同步。
    pub async fn install_managed_modpack(
        &self,
        pointer_url: &str,
        events: Option<&EventSink>,
    ) -> std::result::Result<ModpackInstallOutcome, ModpackSyncError> {
        const UNKNOWN_TARGET: &str = "latest";
        emit_stage(events, ModpackSyncStage::ResolvingManifest);

        let pointer = self
            .fetch_pointer(pointer_url)
            .await
            .map_err(remote_document_error)
            .map_err(|err| {
                sync_core_error(UNKNOWN_TARGET, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let subscription = ModpackSubscription {
            pack_id: pointer.pack_id.clone(),
            pointer_url: pointer_url.to_owned(),
        };
        validate_pointer(&pointer, &subscription, self.launcher_version()).map_err(|err| {
            sync_core_error(&pointer.version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let target_version = pointer.version.clone();
        let manifest = self
            .fetch_manifest(&pointer.manifest_url)
            .await
            .map_err(remote_document_error)
            .and_then(|manifest| {
                validate_manifest(&manifest, &pointer)?;
                Ok(manifest)
            })
            .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let loader = install_loader_choice(manifest.loader.kind).map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let _active_install = acquire_install_gate(self.game_dir()).await.map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let planned = self.planned_install(&manifest).await.map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let planned_instance_id = planned.instance_id;
        self.checked_version_dir(&planned_instance_id)
            .await
            .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        if loader.is_some() {
            self.checked_version_dir(&manifest.minecraft)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
                })?;
        }
        let reservation_identity = ModpackInstallReservation::new(
            &planned_instance_id,
            &subscription,
            &pointer,
            &manifest,
            false,
        );
        let reservations = load_all_install_reservations(self.game_dir())
            .await
            .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let existing_reservation = match reservations.as_slice() {
            [] => None,
            [reservation]
                if reservation.instance_id == planned_instance_id
                    && reservation.matches_environment(&reservation_identity) =>
            {
                Some(reservation.clone())
            }
            [reservation] => {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "managed installation {} ({}) must be resumed before installing {planned_instance_id}",
                            reservation.instance_id, reservation.pack_id
                        ),
                    },
                ));
            }
            _ => {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail:
                            "multiple pending managed install reservations require manual recovery"
                                .to_owned(),
                    },
                ));
            }
        };
        let recovering = existing_reservation.is_some();
        let existing = self.list_installed().await.map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        if let Some(version) = existing
            .versions
            .iter()
            .find(|version| version.id == planned_instance_id)
        {
            let (_, existing_subscription) = self
                .checked_modpack_subscription(&version.id)
                .await
                .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
            if !version_matches_manifest(version, &manifest) {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "planned instance id {planned_instance_id} has incompatible version metadata"
                        ),
                    },
                ));
            }
            if existing_subscription.as_ref() == Some(&subscription) {
                self.finish_managed_install(
                    &version.id,
                    &subscription,
                    &pointer,
                    &manifest,
                    false,
                    events,
                )
                .await?;
                return Ok(ModpackInstallOutcome {
                    instance_id: version.id.clone(),
                    installed_version: target_version,
                });
            }
            if existing_subscription.is_some() || !recovering {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "planned instance id {planned_instance_id} is already occupied by an unrelated instance"
                        ),
                    },
                ));
            }
        }
        if existing
            .broken
            .iter()
            .any(|version| version.id == planned_instance_id)
            && !recovering
        {
            return Err(sync_core_error(
                &target_version,
                ModpackSyncStage::ResolvingManifest,
                CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "planned instance id {planned_instance_id} is occupied by a broken instance"
                    ),
                },
            ));
        }

        let reservation_owns_base = existing_reservation
            .as_ref()
            .is_some_and(|reservation| reservation.install_minecraft);
        let reuse_vanilla = if loader.is_some() {
            if planned_instance_id == manifest.minecraft {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "loader profile attempts to replace the base Minecraft instance {}",
                            manifest.minecraft
                        ),
                    },
                ));
            }
            let base_owner = load_version_directory_owner(self.game_dir(), &manifest.minecraft)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
                })?;
            if let Some(owner) = &base_owner
                && !existing_reservation.as_ref().is_some_and(|reservation| {
                    reservation.install_minecraft && owner.matches(reservation, &manifest.minecraft)
                })
            {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "base Minecraft instance {} is owned by another incomplete installation",
                            manifest.minecraft
                        ),
                    },
                ));
            }
            let compatible_base = existing
                .versions
                .iter()
                .find(|version| version.id == manifest.minecraft)
                .filter(|version| {
                    version.json.id == manifest.minecraft
                        && version.json.inherits_from.is_none()
                        && version.loaders.is_empty()
                });
            if recovering {
                if reservation_owns_base {
                    false
                } else if compatible_base.is_some() {
                    true
                } else {
                    return Err(sync_core_error(
                        &target_version,
                        ModpackSyncStage::ResolvingManifest,
                        CoreError::ModpackMetadataConflict {
                            detail: format!(
                                "base Minecraft instance {} is missing or incompatible with the install reservation",
                                manifest.minecraft
                            ),
                        },
                    ));
                }
            } else if compatible_base.is_some() {
                true
            } else if existing
                .versions
                .iter()
                .any(|version| version.id == manifest.minecraft)
                || existing
                    .broken
                    .iter()
                    .any(|version| version.id == manifest.minecraft)
            {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "base Minecraft id {} is occupied by an incompatible instance",
                            manifest.minecraft
                        ),
                    },
                ));
            } else {
                false
            }
        } else {
            false
        };

        let active_reservation = if let Some(reservation) = &existing_reservation {
            reservation.clone()
        } else {
            let reservation = ModpackInstallReservation::new(
                &planned_instance_id,
                &subscription,
                &pointer,
                &manifest,
                !reuse_vanilla,
            );
            let state = acquire_install_reservation(self.game_dir(), &reservation)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
                })?;
            if state != InstallReservationState::Created {
                return Err(sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "planned instance id {planned_instance_id} was reserved concurrently; retry after the other installation finishes"
                        ),
                    },
                ));
            }
            reservation
        };
        prepare_reserved_version_directories(
            self.game_dir(),
            &active_reservation,
            if loader.is_some() && !reuse_vanilla {
                Some(manifest.minecraft.as_str())
            } else {
                None
            },
        )
        .await
        .map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;

        let vanilla_id = if reuse_vanilla {
            manifest.minecraft.clone()
        } else {
            emit_stage(events, ModpackSyncStage::InstallingMinecraft);
            self.install_vanilla_component(&manifest.minecraft, events)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::InstallingMinecraft, err)
                })?
                .id
        };
        let installed_loader = if let Some(choice) = loader {
            emit_stage(events, ModpackSyncStage::InstallingLoader);
            if let Some(installer) = planned.prepared_installer.as_deref() {
                let relative = installer.strip_prefix(self.game_dir()).map_err(|_| {
                    sync_core_error(
                        &target_version,
                        ModpackSyncStage::InstallingLoader,
                        CoreError::UnsafeModpackPath {
                            path: installer.display().to_string(),
                            reason: "prepared installer cache escaped the game directory"
                                .to_owned(),
                        },
                    )
                })?;
                ensure_no_link_components(self.game_dir(), relative)
                    .await
                    .map_err(|err| {
                        sync_core_error(&target_version, ModpackSyncStage::InstallingLoader, err)
                    })?;
            }
            let installed = match planned.prepared_installer.as_deref() {
                Some(installer) => self
                    .install_loader_component_from_installer(
                        &manifest.minecraft,
                        choice,
                        &manifest.loader.version,
                        installer,
                        events,
                    )
                    .await
                    .map(Some),
                None => {
                    self.install_loader_component(
                        &manifest.minecraft,
                        Some(choice),
                        Some(&manifest.loader.version),
                        events,
                    )
                    .await
                }
            };
            installed.map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::InstallingLoader, err)
            })?
        } else {
            None
        };
        let instance_id = installed_loader
            .as_ref()
            .map(|summary| summary.id.clone())
            .unwrap_or(vanilla_id);
        validate_instance_id(&instance_id).map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::InstallingLoader, err)
        })?;
        if instance_id != planned_instance_id {
            return Err(sync_core_error(
                &target_version,
                ModpackSyncStage::InstallingLoader,
                CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "installer produced instance id {instance_id}, but preflight metadata planned {planned_instance_id}"
                    ),
                },
            ));
        }

        self.finish_managed_install(
            &instance_id,
            &subscription,
            &pointer,
            &manifest,
            true,
            events,
        )
        .await?;

        Ok(ModpackInstallOutcome {
            instance_id,
            installed_version: target_version,
        })
    }

    async fn finish_managed_install(
        &self,
        instance_id: &str,
        subscription: &ModpackSubscription,
        pointer: &PackPointer,
        manifest: &PackManifest,
        claim_reservation: bool,
        events: Option<&EventSink>,
    ) -> std::result::Result<(), ModpackSyncError> {
        let target_version = &manifest.version;
        let version_dir = self.checked_version_dir(instance_id).await.map_err(|err| {
            sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        if claim_reservation {
            let mut settings = self.version_settings(instance_id).await.map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
            settings.isolation = aurora_instance::IsolationOverride::Enabled;
            self.set_initial_managed_version_settings(instance_id, &settings)
                .await
                .map_err(|err| {
                    sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
                })?;
            self.set_modpack_subscription(instance_id, subscription)
                .await
                .map_err(|err| {
                    sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
                })?;
        }
        save_instance_cache(self.game_dir(), &version_dir, POINTER_CACHE_FILE, pointer)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        save_instance_cache(self.game_dir(), &version_dir, MANIFEST_CACHE_FILE, manifest)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        self.apply_manifest(instance_id, manifest, events).await?;
        complete_install_reservation(self.game_dir(), instance_id)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::WritingSnapshot, err)
            })?;
        Ok(())
    }

    /// 把受管实例同步到调用方刚检查到的目标版本。
    ///
    /// 若服务端指针已变化则返回冲突，要求 UI 重新检查；网络失效时可使用此前验证并缓存的不可变
    /// 指针与清单。执行顺序固定为：全部下载成功、删除旧 managed 文件、原子写新快照。
    pub async fn sync_managed_modpack(
        &self,
        version_id: &str,
        target_version: &str,
        events: Option<&EventSink>,
    ) -> std::result::Result<ModpackSyncOutcome, ModpackSyncError> {
        emit_progress(
            events,
            ModpackSyncProgress {
                stage: ModpackSyncStage::ResolvingManifest,
                completed_files: 0,
                total_files: 0,
                downloaded_bytes: 0,
                total_bytes: None,
                current_file: None,
            },
        );

        let (version_dir, subscription) = self
            .checked_modpack_subscription(version_id)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let subscription = subscription.ok_or_else(|| ModpackSyncError {
            target_version: target_version.to_owned(),
            stage: ModpackSyncStage::ResolvingManifest,
            failure: ModpackSyncFailure::Conflict {
                detail: CoreError::ModpackNotManaged {
                    version_id: version_id.to_owned(),
                }
                .to_string(),
            },
        })?;
        let pointer = self
            .resolve_pointer(&version_dir, &subscription)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        if pointer.version != target_version {
            return Err(ModpackSyncError {
                target_version: target_version.to_owned(),
                stage: ModpackSyncStage::ResolvingManifest,
                failure: ModpackSyncFailure::Conflict {
                    detail: format!(
                        "服务端当前版本已从 {target_version} 变为 {}，请重新检查后再同步",
                        pointer.version
                    ),
                },
            });
        }
        let manifest = self
            .resolve_manifest(&version_dir, &pointer)
            .await
            .map_err(|err| {
                sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let installed = self.list_installed().await.map_err(|err| {
            sync_core_error(target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let compatible = installed
            .versions
            .iter()
            .find(|version| version.id == version_id)
            .is_some_and(|version| version_matches_manifest(version, &manifest));
        if !compatible {
            return Err(sync_core_error(
                target_version,
                ModpackSyncStage::ResolvingManifest,
                CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "installed instance {version_id} does not match the manifest runtime identity"
                    ),
                },
            ));
        }

        self.apply_manifest(version_id, &manifest, events).await
    }

    /// 启动链路只从校验通过的成功快照取得门控版本。无订阅或无快照均返回 `None`。
    pub(crate) async fn managed_pack_version_for_launch(
        &self,
        version_id: &str,
    ) -> Result<Option<String>> {
        self.ensure_instance_not_pending_managed_install(version_id)
            .await?;
        let (version_dir, subscription) = self.checked_modpack_subscription(version_id).await?;
        let Some(subscription) = subscription else {
            return Ok(None);
        };
        let Some(snapshot) = self
            .checked_managed_snapshot(version_id, &version_dir, &subscription)
            .await?
        else {
            return Ok(None);
        };
        if snapshot.pack_id != subscription.pack_id {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "实例订阅 {}，成功快照却属于 {}",
                    subscription.pack_id, snapshot.pack_id
                ),
            });
        }
        Ok(Some(snapshot.version))
    }

    async fn resolve_pointer(
        &self,
        version_dir: &Path,
        subscription: &ModpackSubscription,
    ) -> Result<PackPointer> {
        match self.fetch_pointer(&subscription.pointer_url).await {
            Ok(pointer) => {
                validate_pointer(&pointer, subscription, self.launcher_version())?;
                save_instance_cache(self.game_dir(), version_dir, POINTER_CACHE_FILE, &pointer)
                    .await?;
                Ok(pointer)
            }
            Err(RemoteDocumentError::Unavailable(remote)) => load_cached_pointer(
                self.game_dir(),
                version_dir,
                subscription,
                self.launcher_version(),
            )
            .await?
            .ok_or_else(|| CoreError::ModpackRemoteUnavailable {
                url: remote.url,
                detail: remote.detail,
            }),
            Err(RemoteDocumentError::Invalid(err)) => Err(err),
        }
    }

    async fn resolve_manifest(
        &self,
        version_dir: &Path,
        pointer: &PackPointer,
    ) -> Result<PackManifest> {
        match self.fetch_manifest(&pointer.manifest_url).await {
            Ok(manifest) => {
                validate_manifest(&manifest, pointer)?;
                save_instance_cache(self.game_dir(), version_dir, MANIFEST_CACHE_FILE, &manifest)
                    .await?;
                Ok(manifest)
            }
            Err(RemoteDocumentError::Unavailable(remote)) => {
                load_cached_manifest(self.game_dir(), version_dir, pointer)
                    .await?
                    .ok_or_else(|| CoreError::ModpackRemoteUnavailable {
                        url: remote.url,
                        detail: remote.detail,
                    })
            }
            Err(RemoteDocumentError::Invalid(err)) => Err(err),
        }
    }

    async fn fetch_pointer(
        &self,
        url: &str,
    ) -> std::result::Result<PackPointer, RemoteDocumentError> {
        let bytes = self.fetch_document(url).await?;
        PackPointer::from_json_slice(&bytes).map_err(|err| RemoteDocumentError::Invalid(err.into()))
    }

    async fn fetch_manifest(
        &self,
        url: &str,
    ) -> std::result::Result<PackManifest, RemoteDocumentError> {
        let bytes = self.fetch_document(url).await?;
        PackManifest::from_json_slice(&bytes)
            .map_err(|err| RemoteDocumentError::Invalid(err.into()))
    }

    async fn fetch_document(&self, url: &str) -> std::result::Result<Vec<u8>, RemoteDocumentError> {
        let response = self.http().get(url).send().await.map_err(|source| {
            RemoteDocumentError::Unavailable(RemoteUnavailable {
                url: url.to_owned(),
                detail: source.to_string(),
            })
        })?;
        let status = response.status();
        if status.is_server_error() {
            return Err(RemoteDocumentError::Unavailable(RemoteUnavailable {
                url: url.to_owned(),
                detail: format!("HTTP {}", status.as_u16()),
            }));
        }
        if !status.is_success() {
            return Err(RemoteDocumentError::Invalid(
                CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "remote metadata endpoint {url} rejected the request with HTTP {}",
                        status.as_u16()
                    ),
                },
            ));
        }
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|source| {
                RemoteDocumentError::Unavailable(RemoteUnavailable {
                    url: url.to_owned(),
                    detail: source.to_string(),
                })
            })
    }

    async fn planned_install(&self, manifest: &PackManifest) -> Result<PlannedInstall> {
        validate_runtime_component("minecraft", &manifest.minecraft)?;
        if manifest.loader.kind != aurora_modpack::LoaderKind::Vanilla {
            validate_runtime_component("loader.version", &manifest.loader.version)?;
        }

        let mut prepared_installer = None;
        let instance_id = match manifest.loader.kind {
            aurora_modpack::LoaderKind::Vanilla => manifest.minecraft.clone(),
            aurora_modpack::LoaderKind::Fabric | aurora_modpack::LoaderKind::Quilt => {
                let layout = self.layout();
                let pool = self.download_pool();
                let policy = self.retry_policy();
                let http = self.http();
                let context = make_context(&http, &pool, &layout, self.runtime(), &policy);
                let raw = match manifest.loader.kind {
                    aurora_modpack::LoaderKind::Fabric => {
                        LoaderInstaller::fabric(context)
                            .with_base_url(self.fabric_base())
                            .fetch_profile_json(&manifest.minecraft, &manifest.loader.version)
                            .await?
                    }
                    aurora_modpack::LoaderKind::Quilt => {
                        LoaderInstaller::quilt(context)
                            .with_base_url(self.quilt_base())
                            .fetch_profile_json(&manifest.minecraft, &manifest.loader.version)
                            .await?
                    }
                    _ => unreachable!("仅 Fabric/Quilt 进入 profile 分支"),
                };
                let profile =
                    aurora_version::VersionJson::from_json_str(&String::from_utf8_lossy(&raw))?;
                if profile.inherits_from.as_deref() != Some(manifest.minecraft.as_str()) {
                    return Err(CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "loader profile {} does not inherit from manifest Minecraft {}",
                            profile.id, manifest.minecraft
                        ),
                    });
                }
                profile.id
            }
            aurora_modpack::LoaderKind::Forge | aurora_modpack::LoaderKind::NeoForge => {
                let url = if manifest.loader.kind == aurora_modpack::LoaderKind::Forge {
                    forge_installer_url(&manifest.minecraft, &manifest.loader.version)
                } else {
                    neoforge_installer_url(&manifest.loader.version)
                };
                let (instance_id, installer) = self
                    .prepare_installer_plan(&url, &manifest.minecraft)
                    .await?;
                prepared_installer = Some(installer);
                instance_id
            }
            aurora_modpack::LoaderKind::LiteLoader => {
                return Err(CoreError::UnsupportedModpackLoader {
                    loader: "liteloader",
                });
            }
            aurora_modpack::LoaderKind::OptiFine => {
                return Err(CoreError::UnsupportedModpackLoader { loader: "optifine" });
            }
        };
        validate_instance_id(&instance_id)?;
        Ok(PlannedInstall {
            instance_id,
            prepared_installer,
        })
    }

    async fn prepare_installer_plan(
        &self,
        url: &str,
        expected_minecraft: &str,
    ) -> Result<(String, PathBuf)> {
        let installer = self.prepare_installer_cache(url).await?;
        let bytes = tokio::fs::read(&installer)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: installer.clone(),
                source,
            })?;
        let instance_id = inspect_installer_instance_id(&bytes, expected_minecraft)?;
        Ok((instance_id, installer))
    }

    async fn prepare_installer_cache(&self, url: &str) -> Result<PathBuf> {
        let file_name =
            url.rsplit('/')
                .next()
                .ok_or_else(|| CoreError::ModpackMetadataConflict {
                    detail: format!("installer URL has no file name: {url}"),
                })?;
        let safe_file_name =
            SafeRelativePath::new(file_name).map_err(|source| CoreError::UnsafeModpackPath {
                path: file_name.to_owned(),
                reason: format!("unsafe installer cache file name: {source}"),
            })?;
        let relative_path = Path::new(INSTALLER_CACHE_DIR).join(safe_file_name.as_str());
        ensure_no_link_components(self.game_dir(), &relative_path).await?;
        let installer = self.game_dir().join(&relative_path);
        let report = self
            .download_pool()
            .download_all(vec![DownloadTask::new(url, &installer)], None)
            .await?;
        if let Some(failure) = report.failures.first() {
            return Err(CoreError::ModpackRemoteUnavailable {
                url: url.to_owned(),
                detail: failure.error.to_string(),
            });
        }
        ensure_no_link_components(self.game_dir(), &relative_path).await?;
        Ok(installer)
    }

    async fn apply_manifest(
        &self,
        version_id: &str,
        manifest: &PackManifest,
        events: Option<&EventSink>,
    ) -> std::result::Result<ModpackSyncOutcome, ModpackSyncError> {
        let target_version = manifest.version.clone();
        let version_dir = self.checked_version_dir(version_id).await.map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let snapshot_store = SnapshotStore::for_version_dir(&version_dir);
        let previous = load_snapshot(self.game_dir(), &version_dir)
            .await
            .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        if let Some(snapshot) = &previous
            && snapshot.pack_id != manifest.pack_id
        {
            return Err(sync_core_error(
                &target_version,
                ModpackSyncStage::ResolvingManifest,
                CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "snapshot belongs to pack {}, but manifest belongs to {}",
                        snapshot.pack_id, manifest.pack_id
                    ),
                },
            ));
        }
        ensure_version_relative_path(
            self.game_dir(),
            &version_dir,
            Path::new(".aurora/settings.json"),
        )
        .await
        .map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let working_dir = self.resolve_working_dir(version_id).await.map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let working_directory = if working_dir.isolated {
            SnapshotWorkingDirectory::IsolatedVersionDirectory
        } else {
            SnapshotWorkingDirectory::SharedGameDirectory
        };
        if let Some(snapshot) = &previous
            && snapshot.working_directory != working_directory
        {
            return Err(sync_core_error(
                &target_version,
                ModpackSyncStage::ResolvingManifest,
                aurora_modpack::Error::SnapshotWorkingDirectoryMismatch {
                    snapshot: snapshot.working_directory,
                    current: working_directory,
                }
                .into(),
            ));
        }
        ensure_path_under_game_dir(self.game_dir(), &working_dir.working_dir)
            .await
            .map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
            })?;
        let disk = collect_disk_files(
            self.game_dir(),
            &working_dir.working_dir,
            manifest,
            previous.as_ref(),
        )
        .await
        .map_err(|err| {
            sync_core_error(&target_version, ModpackSyncStage::ResolvingManifest, err)
        })?;
        let plan = diff(manifest, previous.as_ref(), &disk, working_directory).map_err(|err| {
            sync_core_error(
                &target_version,
                ModpackSyncStage::ResolvingManifest,
                err.into(),
            )
        })?;

        let total_bytes = plan
            .to_download
            .iter()
            .try_fold(0_u64, |total, action| total.checked_add(action.file.size))
            .ok_or_else(|| {
                sync_core_error(
                    &target_version,
                    ModpackSyncStage::ResolvingManifest,
                    CoreError::ModpackMetadataConflict {
                        detail: "manifest download size exceeds u64".to_owned(),
                    },
                )
            })?;
        let mut tasks = Vec::with_capacity(plan.to_download.len());
        for action in &plan.to_download {
            let file = &action.file;
            ensure_execution_path(self.game_dir(), &working_dir.working_dir, &file.path)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::DownloadingFiles, err)
                })?;
            tasks.push(
                DownloadTask::new(
                    file.urls[0].clone(),
                    file.path.resolve_under(&working_dir.working_dir),
                )
                .with_urls(file.urls.clone())
                .with_sha1(file.sha1.to_string())
                .with_size(file.size),
            );
        }

        emit_progress(
            events,
            ModpackSyncProgress {
                stage: ModpackSyncStage::DownloadingFiles,
                completed_files: 0,
                total_files: tasks.len() as u64,
                downloaded_bytes: 0,
                total_bytes: Some(total_bytes),
                current_file: None,
            },
        );
        let (progress_tx, progress_task) = download_progress_bridge(events, total_bytes);
        let report = self
            .download_pool()
            .download_all(tasks, progress_tx)
            .await
            .map_err(|err| ModpackSyncError {
                target_version: target_version.clone(),
                stage: ModpackSyncStage::DownloadingFiles,
                failure: ModpackSyncFailure::Network {
                    file_path: "<download-batch>".to_owned(),
                    detail: err.to_string(),
                },
            })?;
        if let Some(task) = progress_task {
            task.await.map_err(|err| {
                sync_core_error(
                    &target_version,
                    ModpackSyncStage::DownloadingFiles,
                    err.into(),
                )
            })?;
        }
        if let Some(failure) = report.failures.first() {
            return Err(ModpackSyncError {
                target_version,
                stage: ModpackSyncStage::DownloadingFiles,
                failure: classify_download_failure(
                    failure,
                    report.failures.len(),
                    &working_dir.working_dir,
                ),
            });
        }

        let deletion_snapshot =
            load_snapshot(self.game_dir(), &version_dir)
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::DeletingFiles, err)
                })?;
        if deletion_snapshot != previous {
            return Err(ModpackSyncError {
                target_version,
                stage: ModpackSyncStage::DeletingFiles,
                failure: ModpackSyncFailure::Conflict {
                    detail: "the applied snapshot changed while files were downloading; retry after checking the instance".to_owned(),
                },
            });
        }

        let delete_total = plan.to_delete.len() as u64;
        for (index, deletion) in plan.to_delete.iter().enumerate() {
            validate_managed_deletion(previous.as_ref(), deletion).map_err(|err| {
                sync_core_error(&target_version, ModpackSyncStage::DeletingFiles, err)
            })?;
            ensure_execution_path(self.game_dir(), &working_dir.working_dir, deletion.path())
                .await
                .map_err(|err| {
                    sync_core_error(&target_version, ModpackSyncStage::DeletingFiles, err)
                })?;
            emit_progress(
                events,
                ModpackSyncProgress {
                    stage: ModpackSyncStage::DeletingFiles,
                    completed_files: index as u64,
                    total_files: delete_total,
                    downloaded_bytes: total_bytes,
                    total_bytes: Some(total_bytes),
                    current_file: Some(deletion.path().to_string()),
                },
            );
            let path = deletion.path().resolve_under(&working_dir.working_dir);
            if let Err(source) = tokio::fs::remove_file(&path).await
                && source.kind() != std::io::ErrorKind::NotFound
            {
                return Err(ModpackSyncError {
                    target_version,
                    stage: ModpackSyncStage::DeletingFiles,
                    failure: classify_io(deletion.path().as_str(), source, None),
                });
            }
        }

        emit_progress(
            events,
            ModpackSyncProgress {
                stage: ModpackSyncStage::WritingSnapshot,
                completed_files: 0,
                total_files: 1,
                downloaded_bytes: total_bytes,
                total_bytes: Some(total_bytes),
                current_file: Some(snapshot_store.path().display().to_string()),
            },
        );
        ensure_version_relative_path(
            self.game_dir(),
            &version_dir,
            Path::new(".aurora")
                .join(aurora_modpack::APPLIED_SNAPSHOT_FILE)
                .as_path(),
        )
        .await
        .map_err(|err| sync_core_error(&target_version, ModpackSyncStage::WritingSnapshot, err))?;
        snapshot_store
            .save(&plan.next_snapshot)
            .await
            .map_err(|err| ModpackSyncError {
                target_version: target_version.clone(),
                stage: ModpackSyncStage::WritingSnapshot,
                failure: ModpackSyncFailure::SnapshotWrite {
                    file_path: snapshot_store.path().display().to_string(),
                    detail: err.to_string(),
                },
            })?;
        emit_progress(
            events,
            ModpackSyncProgress {
                stage: ModpackSyncStage::WritingSnapshot,
                completed_files: 1,
                total_files: 1,
                downloaded_bytes: total_bytes,
                total_bytes: Some(total_bytes),
                current_file: None,
            },
        );

        Ok(ModpackSyncOutcome {
            installed_version: target_version,
            downloaded_files: report.succeeded,
            deleted_files: plan.to_delete.len(),
            kept_files: plan.to_keep.len(),
        })
    }

    pub(crate) async fn checked_version_dir(&self, version_id: &str) -> Result<PathBuf> {
        validate_instance_id(version_id)?;
        let relative = Path::new(VERSIONS_DIR).join(version_id);
        ensure_no_link_components(self.game_dir(), &relative).await?;
        Ok(self.game_dir().join(relative))
    }

    pub(crate) async fn ensure_instance_not_pending_managed_install(
        &self,
        version_id: &str,
    ) -> Result<()> {
        if self
            .pending_managed_install_subscription(version_id)
            .await?
            .is_some()
        {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "instance {version_id} belongs to an incomplete managed modpack installation; resume that installation before using the instance"
                ),
            });
        }
        Ok(())
    }

    async fn pending_managed_install_subscription(
        &self,
        version_id: &str,
    ) -> Result<Option<ModpackSubscription>> {
        self.checked_version_dir(version_id).await?;
        let owner = load_version_directory_owner(self.game_dir(), version_id).await?;
        let direct_reservation = load_install_reservation(self.game_dir(), version_id).await?;

        let reservation = match owner {
            Some(owner) => {
                let reservation = load_install_reservation(self.game_dir(), &owner.instance_id)
                    .await?
                    .ok_or_else(|| CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "instance {version_id} has an install owner sentinel but its reservation is missing"
                        ),
                    })?;
                if !owner.matches(&reservation, version_id) {
                    return Err(CoreError::ModpackMetadataConflict {
                        detail: format!(
                            "instance {version_id} has an install owner sentinel that does not match its reservation"
                        ),
                    });
                }
                Some(reservation)
            }
            None => direct_reservation,
        };

        Ok(reservation.map(|reservation| ModpackSubscription {
            pack_id: reservation.pack_id,
            pointer_url: reservation.pointer_url,
        }))
    }

    pub(crate) async fn managed_mod_paths_for_player_write(
        &self,
        version_id: &str,
    ) -> Result<HashSet<String>> {
        self.ensure_instance_not_pending_managed_install(version_id)
            .await?;
        let (version_dir, subscription) = self.checked_modpack_subscription(version_id).await?;
        let Some(subscription) = subscription else {
            return Ok(HashSet::new());
        };
        let snapshot = self
            .checked_managed_snapshot(version_id, &version_dir, &subscription)
            .await?
            .ok_or_else(|| CoreError::ModpackMetadataConflict {
                detail: format!(
                    "managed instance {version_id} has no successful applied snapshot; player mod writes are locked"
                ),
            })?;
        Ok(snapshot
            .files
            .into_iter()
            .filter(|entry| entry.policy == FilePolicy::Managed)
            .map(|entry| entry.path.as_str().replace('\\', "/").to_ascii_lowercase())
            .collect())
    }

    async fn checked_modpack_subscription(
        &self,
        version_id: &str,
    ) -> Result<(PathBuf, Option<ModpackSubscription>)> {
        let version_dir = self.checked_version_dir(version_id).await?;
        ensure_version_relative_path(
            self.game_dir(),
            &version_dir,
            Path::new(".aurora/modpack-subscription.json"),
        )
        .await?;
        let subscription = self.modpack_subscription(version_id).await?;
        Ok((version_dir, subscription))
    }

    async fn checked_managed_snapshot(
        &self,
        version_id: &str,
        version_dir: &Path,
        subscription: &ModpackSubscription,
    ) -> Result<Option<AppliedSnapshot>> {
        let Some(snapshot) = load_snapshot(self.game_dir(), version_dir).await? else {
            return Ok(None);
        };
        if snapshot.pack_id != subscription.pack_id {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "subscription belongs to pack {}, but snapshot belongs to {}",
                    subscription.pack_id, snapshot.pack_id
                ),
            });
        }
        let working_dir = self.resolve_working_dir(version_id).await?;
        let current = snapshot_working_directory(working_dir.isolated);
        if snapshot.working_directory != current {
            return Err(aurora_modpack::Error::SnapshotWorkingDirectoryMismatch {
                snapshot: snapshot.working_directory,
                current,
            }
            .into());
        }
        Ok(Some(snapshot))
    }
}

fn snapshot_working_directory(isolated: bool) -> SnapshotWorkingDirectory {
    if isolated {
        SnapshotWorkingDirectory::IsolatedVersionDirectory
    } else {
        SnapshotWorkingDirectory::SharedGameDirectory
    }
}

fn remote_document_error(error: RemoteDocumentError) -> CoreError {
    match error {
        RemoteDocumentError::Unavailable(remote) => CoreError::ModpackRemoteUnavailable {
            url: remote.url,
            detail: remote.detail,
        },
        RemoteDocumentError::Invalid(error) => error,
    }
}

fn install_loader_choice(kind: aurora_modpack::LoaderKind) -> Result<Option<LoaderChoice>> {
    match kind {
        aurora_modpack::LoaderKind::Vanilla => Ok(None),
        aurora_modpack::LoaderKind::Fabric => Ok(Some(LoaderChoice::Fabric)),
        aurora_modpack::LoaderKind::Quilt => Ok(Some(LoaderChoice::Quilt)),
        aurora_modpack::LoaderKind::Forge => Ok(Some(LoaderChoice::Forge)),
        aurora_modpack::LoaderKind::NeoForge => Ok(Some(LoaderChoice::NeoForge)),
        aurora_modpack::LoaderKind::LiteLoader => Err(CoreError::UnsupportedModpackLoader {
            loader: "liteloader",
        }),
        aurora_modpack::LoaderKind::OptiFine => {
            Err(CoreError::UnsupportedModpackLoader { loader: "optifine" })
        }
    }
}

fn version_matches_manifest(
    version: &aurora_instance::DiscoveredVersion,
    manifest: &PackManifest,
) -> bool {
    if version.json.id != version.id {
        return false;
    }
    if manifest.loader.kind == aurora_modpack::LoaderKind::Vanilla {
        return version.id == manifest.minecraft
            && version.json.inherits_from.is_none()
            && version.loaders.is_empty();
    }
    if version.json.inherits_from.as_deref() != Some(manifest.minecraft.as_str()) {
        return false;
    }
    if version.loaders.len() != 1 {
        return false;
    }
    version.loaders.first().is_some_and(|loader| {
        let kind_matches = matches!(
            (manifest.loader.kind, loader.kind),
            (
                aurora_modpack::LoaderKind::Fabric,
                aurora_version::LoaderKind::Fabric
            ) | (
                aurora_modpack::LoaderKind::Quilt,
                aurora_version::LoaderKind::Quilt
            ) | (
                aurora_modpack::LoaderKind::Forge,
                aurora_version::LoaderKind::Forge
            ) | (
                aurora_modpack::LoaderKind::NeoForge,
                aurora_version::LoaderKind::NeoForge
            )
        );
        kind_matches && loader.version.as_deref() == Some(manifest.loader.version.as_str())
    })
}

fn new_install_operation_id() -> String {
    format!("{:016x}{:016x}", fastrand::u64(..), fastrand::u64(..))
}

pub(crate) fn validate_instance_id(instance_id: &str) -> Result<()> {
    let invalid = instance_id.trim().is_empty()
        || instance_id != instance_id.trim()
        || matches!(instance_id, "." | "..")
        || instance_id.ends_with(['.', ' '])
        || instance_id
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':' | '\0'));
    if invalid {
        return Err(CoreError::UnsafeModpackPath {
            path: instance_id.to_owned(),
            reason: "invalid instance id".to_owned(),
        });
    }
    SafeRelativePath::new(instance_id).map_err(|source| CoreError::UnsafeModpackPath {
        path: instance_id.to_owned(),
        reason: format!("unsafe instance id: {source}"),
    })?;
    Ok(())
}

fn validate_runtime_component(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "manifest field {field} is not a safe runtime version component: {value:?}"
            ),
        });
    }
    Ok(())
}

fn inspect_installer_instance_id(installer: &[u8], expected_minecraft: &str) -> Result<String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(installer)).map_err(|source| {
        CoreError::ModpackMetadataConflict {
            detail: format!("loader installer is not a valid ZIP archive: {source}"),
        }
    })?;
    let mut entry = archive.by_name("install_profile.json").map_err(|source| {
        CoreError::ModpackMetadataConflict {
            detail: format!("loader installer has no readable install_profile.json: {source}"),
        }
    })?;
    if entry.size() > MAX_INSTALL_PROFILE_BYTES {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "install_profile.json is {} bytes, exceeding the {} byte safety limit",
                entry.size(),
                MAX_INSTALL_PROFILE_BYTES
            ),
        });
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut bytes)
        .map_err(|source| CoreError::ModpackMetadataConflict {
            detail: format!("failed to read install_profile.json: {source}"),
        })?;
    let profile: InstallerProfileIdentity =
        serde_json::from_slice(&bytes).map_err(|source| CoreError::ModpackMetadataConflict {
            detail: format!("invalid install_profile.json: {source}"),
        })?;

    let instance_id = if profile.install.is_some() && profile.processors.is_empty() {
        let version = profile
            .version_info
            .ok_or_else(|| CoreError::ModpackMetadataConflict {
                detail: "legacy install_profile is missing versionInfo".to_owned(),
            })?;
        if version.inherits_from.as_deref() != Some(expected_minecraft) {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "legacy loader profile inherits from {:?}, but manifest requires {expected_minecraft}",
                    version.inherits_from
                ),
            });
        }
        version.id
    } else if profile.json.is_some() {
        if profile.minecraft.as_deref() != Some(expected_minecraft) {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "loader installer targets Minecraft {:?}, but manifest requires {expected_minecraft}",
                    profile.minecraft
                ),
            });
        }
        profile
            .version
            .ok_or_else(|| CoreError::ModpackMetadataConflict {
                detail: "modern install_profile is missing version".to_owned(),
            })?
    } else {
        return Err(CoreError::ModpackMetadataConflict {
            detail: "unsupported install_profile shape".to_owned(),
        });
    };
    validate_instance_id(&instance_id)?;
    Ok(instance_id)
}

fn validate_pointer(
    pointer: &PackPointer,
    subscription: &ModpackSubscription,
    current: &semver::Version,
) -> Result<()> {
    if pointer.pack_id != subscription.pack_id {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "订阅要求整合包 {}，指针却返回 {}",
                subscription.pack_id, pointer.pack_id
            ),
        });
    }
    let required = semver::Version::parse(&pointer.min_launcher_version).map_err(|source| {
        CoreError::ModpackMetadataConflict {
            detail: format!("min_launcher_version 无法按 semver 解析: {source}"),
        }
    })?;
    if current < &required {
        return Err(CoreError::ModpackLauncherTooOld {
            current: current.to_string(),
            required: required.to_string(),
        });
    }
    Ok(())
}

fn validate_manifest(manifest: &PackManifest, pointer: &PackPointer) -> Result<()> {
    if manifest.pack_id != pointer.pack_id || manifest.version != pointer.version {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "指针要求 {}/{}, 清单实际为 {}/{}",
                pointer.pack_id, pointer.version, manifest.pack_id, manifest.version
            ),
        });
    }
    Ok(())
}

async fn load_cached_pointer(
    game_dir: &Path,
    version_dir: &Path,
    subscription: &ModpackSubscription,
    launcher_version: &semver::Version,
) -> Result<Option<PackPointer>> {
    let Some(bytes) = read_instance_cache(game_dir, version_dir, POINTER_CACHE_FILE).await? else {
        return Ok(None);
    };
    let pointer = PackPointer::from_json_slice(&bytes)?;
    validate_pointer(&pointer, subscription, launcher_version)?;
    Ok(Some(pointer))
}

async fn cached_versions(
    game_dir: &Path,
    version_dir: &Path,
    subscription: &ModpackSubscription,
    installed_version: Option<String>,
    launcher_version: &semver::Version,
) -> (Option<KnownModpackVersions>, Option<String>) {
    match load_cached_pointer(game_dir, version_dir, subscription, launcher_version).await {
        Ok(pointer) => (
            pointer.map(|latest| KnownModpackVersions {
                installed_version,
                latest,
            }),
            None,
        ),
        Err(err) => (None, Some(err.to_string())),
    }
}

fn append_cache_detail(primary: String, cache_detail: Option<String>) -> String {
    match cache_detail {
        Some(detail) => format!("{primary}；本地缓存也不可用: {detail}"),
        None => primary,
    }
}

async fn load_cached_manifest(
    game_dir: &Path,
    version_dir: &Path,
    pointer: &PackPointer,
) -> Result<Option<PackManifest>> {
    let Some(bytes) = read_instance_cache(game_dir, version_dir, MANIFEST_CACHE_FILE).await? else {
        return Ok(None);
    };
    let manifest = PackManifest::from_json_slice(&bytes)?;
    validate_manifest(&manifest, pointer)?;
    Ok(Some(manifest))
}

async fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(aurora_base::Error::Io {
            path: path.to_owned(),
            source,
        }
        .into()),
    }
}

pub(crate) async fn acquire_install_gate(game_dir: &Path) -> Result<ActiveManagedInstall> {
    use fs2::FileExt;

    let absolute = std::path::absolute(game_dir).map_err(|source| aurora_base::Error::Io {
        path: game_dir.to_owned(),
        source,
    })?;
    ensure_no_link_components(&absolute, Path::new(".aurora")).await?;
    let metadata_dir = absolute.join(".aurora");
    tokio::fs::create_dir_all(&metadata_dir)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: metadata_dir,
            source,
        })?;
    ensure_no_link_components(&absolute, Path::new(".aurora")).await?;
    let game_dir = match std::fs::canonicalize(&absolute) {
        Ok(path) => path,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => absolute,
        Err(source) => {
            return Err(aurora_base::Error::Io {
                path: absolute,
                source,
            }
            .into());
        }
    };
    let lock_path = game_dir.join(INSTALL_LOCK_FILE);
    ensure_no_link_components(&game_dir, Path::new(INSTALL_LOCK_FILE)).await?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .map_err(|source| aurora_base::Error::Io {
            path: lock_path.clone(),
            source,
        })?;
    ensure_no_link_components(&game_dir, Path::new(INSTALL_LOCK_FILE)).await?;
    file.try_lock_exclusive().map_err(|source| {
        if install_lock_is_contended(&source) {
            CoreError::ModpackMetadataConflict {
                detail: format!(
                    "another installation process is already active for {}",
                    game_dir.display()
                ),
            }
        } else {
            aurora_base::Error::Io {
                path: lock_path,
                source,
            }
            .into()
        }
    })?;
    let active = ACTIVE_MANAGED_INSTALLS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut active = active
        .lock()
        .map_err(|_| CoreError::ModpackMetadataConflict {
            detail: "managed install lock is poisoned".to_owned(),
        })?;
    if !active.insert(game_dir.clone()) {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "another managed installation is already active for {}",
                game_dir.display()
            ),
        });
    }
    Ok(ActiveManagedInstall {
        game_dir,
        _file: file,
    })
}

fn install_lock_is_contended(source: &std::io::Error) -> bool {
    source.kind() == std::io::ErrorKind::WouldBlock
        || matches!(source.raw_os_error(), Some(11 | 33 | 35))
}

fn install_reservation_relative(instance_id: &str) -> Result<PathBuf> {
    validate_instance_id(instance_id)?;
    Ok(Path::new(INSTALL_RESERVATION_DIR).join(format!("{instance_id}.json")))
}

async fn load_install_reservation(
    game_dir: &Path,
    instance_id: &str,
) -> Result<Option<ModpackInstallReservation>> {
    let relative = install_reservation_relative(instance_id)?;
    ensure_no_link_components(game_dir, &relative).await?;
    let path = game_dir.join(&relative);
    let Some(bytes) = read_optional(&path).await? else {
        return Ok(None);
    };
    let reservation: ModpackInstallReservation =
        serde_json::from_slice(&bytes).map_err(|source| CoreError::ConfigParse {
            path: path.clone(),
            source,
        })?;
    reservation.validate()?;
    if reservation.instance_id != instance_id {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "install reservation at {} belongs to instance {} instead of {}",
                path.display(),
                reservation.instance_id,
                instance_id
            ),
        });
    }
    Ok(Some(reservation))
}

async fn load_all_install_reservations(game_dir: &Path) -> Result<Vec<ModpackInstallReservation>> {
    let relative = Path::new(INSTALL_RESERVATION_DIR);
    ensure_no_link_components(game_dir, relative).await?;
    let directory = game_dir.join(relative);
    let mut entries = match tokio::fs::read_dir(&directory).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(aurora_base::Error::Io {
                path: directory,
                source,
            }
            .into());
        }
    };
    let mut reservations = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: directory.clone(),
            source,
        })?
    {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: path.clone(),
                source,
            })?;
        if !file_type.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "unexpected entry in install reservation directory: {}",
                    path.display()
                ),
            });
        }
        let bytes = tokio::fs::read(&path)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: path.clone(),
                source,
            })?;
        let reservation: ModpackInstallReservation =
            serde_json::from_slice(&bytes).map_err(|source| CoreError::ConfigParse {
                path: path.clone(),
                source,
            })?;
        reservation.validate()?;
        let expected_name = format!("{}.json", reservation.instance_id);
        if path.file_name().and_then(|value| value.to_str()) != Some(expected_name.as_str()) {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "install reservation file {} does not match instance {}",
                    path.display(),
                    reservation.instance_id
                ),
            });
        }
        reservations.push(reservation);
    }
    reservations.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
    Ok(reservations)
}

pub(crate) async fn ensure_no_pending_managed_install(game_dir: &Path) -> Result<()> {
    let reservations = load_all_install_reservations(game_dir).await?;
    if let Some(reservation) = reservations.first() {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "managed installation {} ({}) must be resumed before another installation can start",
                reservation.instance_id, reservation.pack_id
            ),
        });
    }
    Ok(())
}

async fn acquire_install_reservation(
    game_dir: &Path,
    reservation: &ModpackInstallReservation,
) -> Result<InstallReservationState> {
    use tokio::io::AsyncWriteExt;

    reservation.validate()?;
    let relative = install_reservation_relative(&reservation.instance_id)?;
    let parent_relative = relative
        .parent()
        .expect("reservation path always has a parent");
    ensure_no_link_components(game_dir, parent_relative).await?;
    let parent = game_dir.join(parent_relative);
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: parent.clone(),
            source,
        })?;
    ensure_no_link_components(game_dir, parent_relative).await?;

    let bytes = serde_json::to_vec_pretty(reservation).map_err(CoreError::ConfigSerialize)?;
    let target = game_dir.join(&relative);
    match tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .await
    {
        Ok(mut file) => {
            if let Err(source) = file.write_all(&bytes).await {
                drop(file);
                return cleanup_failed_reservation_write(&target, source).await;
            }
            if let Err(source) = file.sync_all().await {
                drop(file);
                return cleanup_failed_reservation_write(&target, source).await;
            }
            drop(file);
            ensure_no_link_components(game_dir, &relative).await?;
            Ok(InstallReservationState::Created)
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = load_install_reservation(game_dir, &reservation.instance_id)
                .await?
                .ok_or_else(|| CoreError::ModpackMetadataConflict {
                    detail: "install reservation disappeared during atomic acquisition".to_owned(),
                })?;
            if !existing.matches_environment(reservation) {
                return Err(CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "instance {} is reserved by another managed installation",
                        reservation.instance_id
                    ),
                });
            }
            Ok(InstallReservationState::Existing)
        }
        Err(source) => Err(aurora_base::Error::Io {
            path: target,
            source,
        }
        .into()),
    }
}

async fn cleanup_failed_reservation_write(
    path: &Path,
    write_error: std::io::Error,
) -> Result<InstallReservationState> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Err(aurora_base::Error::Io {
            path: path.to_owned(),
            source: write_error,
        }
        .into()),
        Err(cleanup_error) => Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "failed to write install reservation {}: {}; cleanup also failed: {}",
                path.display(),
                write_error,
                cleanup_error
            ),
        }),
    }
}

async fn remove_install_reservation(game_dir: &Path, instance_id: &str) -> Result<()> {
    let relative = install_reservation_relative(instance_id)?;
    ensure_no_link_components(game_dir, &relative).await?;
    let path = game_dir.join(relative);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(aurora_base::Error::Io { path, source }.into()),
    }
}

async fn complete_install_reservation(game_dir: &Path, instance_id: &str) -> Result<()> {
    let Some(reservation) = load_install_reservation(game_dir, instance_id).await? else {
        return Ok(());
    };
    if reservation.install_minecraft && reservation.minecraft != reservation.instance_id {
        remove_version_directory_owner(game_dir, &reservation, &reservation.minecraft).await?;
    }
    remove_version_directory_owner(game_dir, &reservation, &reservation.instance_id).await?;
    remove_install_reservation(game_dir, instance_id).await
}

async fn remove_version_directory_owner(
    game_dir: &Path,
    reservation: &ModpackInstallReservation,
    directory_id: &str,
) -> Result<()> {
    let Some(owner) = load_version_directory_owner(game_dir, directory_id).await? else {
        return Ok(());
    };
    if !owner.matches(reservation, directory_id) {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "refusing to remove owner sentinel for instance {directory_id}: operation mismatch"
            ),
        });
    }
    let relative = Path::new(VERSIONS_DIR)
        .join(directory_id)
        .join(INSTALL_OWNER_FILE);
    ensure_no_link_components(game_dir, &relative).await?;
    let path = game_dir.join(relative);
    tokio::fs::remove_file(&path)
        .await
        .map_err(|source| aurora_base::Error::Io { path, source })?;
    Ok(())
}

async fn prepare_reserved_version_directories(
    game_dir: &Path,
    reservation: &ModpackInstallReservation,
    base_minecraft: Option<&str>,
) -> Result<()> {
    reservation.validate()?;
    if let Some(base_minecraft) = base_minecraft {
        validate_instance_id(base_minecraft)?;
    }
    let versions_relative = Path::new(VERSIONS_DIR);
    ensure_no_link_components(game_dir, versions_relative).await?;
    let versions_dir = game_dir.join(versions_relative);
    tokio::fs::create_dir_all(&versions_dir)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: versions_dir.clone(),
            source,
        })?;
    ensure_no_link_components(game_dir, versions_relative).await?;

    claim_version_directory(game_dir, reservation, &reservation.instance_id).await?;
    if let Some(base_minecraft) = base_minecraft
        && base_minecraft != reservation.instance_id
    {
        claim_version_directory(game_dir, reservation, base_minecraft).await?;
    }
    Ok(())
}

async fn claim_version_directory(
    game_dir: &Path,
    reservation: &ModpackInstallReservation,
    directory_id: &str,
) -> Result<()> {
    validate_instance_id(directory_id)?;
    let target_relative = Path::new(VERSIONS_DIR).join(directory_id);
    ensure_no_link_components(game_dir, &target_relative).await?;
    let target = game_dir.join(&target_relative);
    match tokio::fs::symlink_metadata(&target).await {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "reserved instance path {} is not a directory",
                    target.display()
                ),
            });
        }
        Ok(_) => {
            let owner = load_version_directory_owner(game_dir, directory_id).await?;
            if owner
                .as_ref()
                .is_some_and(|owner| owner.matches(reservation, directory_id))
            {
                return Ok(());
            }
            return Err(CoreError::ModpackMetadataConflict {
                detail: format!(
                    "instance path {} is not owned by reservation operation {}",
                    target.display(),
                    reservation.operation_id
                ),
            });
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(aurora_base::Error::Io {
                path: target,
                source,
            }
            .into());
        }
    }

    let staging_parent_relative = Path::new(INSTALL_STAGING_DIR);
    ensure_no_link_components(game_dir, staging_parent_relative).await?;
    let staging_parent = game_dir.join(staging_parent_relative);
    tokio::fs::create_dir_all(&staging_parent)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: staging_parent.clone(),
            source,
        })?;
    ensure_no_link_components(game_dir, staging_parent_relative).await?;
    let role = if directory_id == reservation.instance_id {
        "target"
    } else {
        "base"
    };
    let staging_relative =
        staging_parent_relative.join(format!("{}-{role}", reservation.operation_id));
    let staging = game_dir.join(&staging_relative);
    match tokio::fs::create_dir(&staging).await {
        Ok(()) => {
            ensure_no_link_components(game_dir, &staging_relative).await?;
            let owner = VersionDirectoryOwner::new(reservation, directory_id);
            let owner_parent_relative = staging_relative.join(".aurora");
            ensure_no_link_components(game_dir, &owner_parent_relative).await?;
            let owner_parent = game_dir.join(&owner_parent_relative);
            tokio::fs::create_dir_all(&owner_parent)
                .await
                .map_err(|source| aurora_base::Error::Io {
                    path: owner_parent,
                    source,
                })?;
            ensure_no_link_components(game_dir, &owner_parent_relative).await?;
            let owner_relative = staging_relative.join(INSTALL_OWNER_FILE);
            ensure_no_link_components(game_dir, &owner_relative).await?;
            let bytes = serde_json::to_vec_pretty(&owner).map_err(CoreError::ConfigSerialize)?;
            aurora_base::fs::atomic_write(game_dir.join(&owner_relative), &bytes).await?;
            ensure_no_link_components(game_dir, &owner_relative).await?;
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let owner = load_directory_owner(game_dir, &staging_relative)
                .await?
                .ok_or_else(|| CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "install staging directory {} has no owner sentinel",
                        staging.display()
                    ),
                })?;
            if !owner.matches(reservation, directory_id) {
                return Err(CoreError::ModpackMetadataConflict {
                    detail: format!(
                        "install staging directory {} belongs to another operation",
                        staging.display()
                    ),
                });
            }
        }
        Err(source) => {
            return Err(aurora_base::Error::Io {
                path: staging,
                source,
            }
            .into());
        }
    }

    ensure_no_link_components(game_dir, &staging_relative).await?;
    ensure_no_link_components(game_dir, &target_relative).await?;
    tokio::fs::rename(&staging, &target)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: target.clone(),
            source,
        })?;
    ensure_no_link_components(game_dir, &target_relative).await?;
    let owner = load_version_directory_owner(game_dir, directory_id)
        .await?
        .ok_or_else(|| CoreError::ModpackMetadataConflict {
            detail: format!(
                "claimed instance path {} lost its owner sentinel",
                target.display()
            ),
        })?;
    if !owner.matches(reservation, directory_id) {
        return Err(CoreError::ModpackMetadataConflict {
            detail: format!(
                "claimed instance path {} has a mismatched owner sentinel",
                target.display()
            ),
        });
    }
    Ok(())
}

async fn load_version_directory_owner(
    game_dir: &Path,
    directory_id: &str,
) -> Result<Option<VersionDirectoryOwner>> {
    validate_instance_id(directory_id)?;
    load_directory_owner(game_dir, &Path::new(VERSIONS_DIR).join(directory_id)).await
}

async fn load_directory_owner(
    game_dir: &Path,
    directory_relative: &Path,
) -> Result<Option<VersionDirectoryOwner>> {
    let owner_relative = directory_relative.join(INSTALL_OWNER_FILE);
    ensure_no_link_components(game_dir, &owner_relative).await?;
    let path = game_dir.join(&owner_relative);
    let Some(bytes) = read_optional(&path).await? else {
        return Ok(None);
    };
    let owner: VersionDirectoryOwner =
        serde_json::from_slice(&bytes).map_err(|source| CoreError::ConfigParse {
            path: path.clone(),
            source,
        })?;
    owner.validate()?;
    Ok(Some(owner))
}

async fn load_snapshot(game_dir: &Path, version_dir: &Path) -> Result<Option<AppliedSnapshot>> {
    let relative = Path::new(".aurora").join(aurora_modpack::APPLIED_SNAPSHOT_FILE);
    ensure_version_relative_path(game_dir, version_dir, &relative).await?;
    Ok(SnapshotStore::for_version_dir(version_dir).load().await?)
}

async fn read_instance_cache(
    game_dir: &Path,
    version_dir: &Path,
    file_name: &str,
) -> Result<Option<Vec<u8>>> {
    let relative = Path::new(".aurora").join(file_name);
    ensure_version_relative_path(game_dir, version_dir, &relative).await?;
    read_optional(&version_dir.join(relative)).await
}

async fn save_instance_cache(
    game_dir: &Path,
    version_dir: &Path,
    file_name: &str,
    value: &impl Serialize,
) -> Result<()> {
    let relative = Path::new(".aurora").join(file_name);
    ensure_version_relative_path(game_dir, version_dir, &relative).await?;
    let bytes = serde_json::to_vec_pretty(value).map_err(CoreError::ConfigSerialize)?;
    aurora_base::fs::atomic_write(&version_dir.join(relative), &bytes).await?;
    Ok(())
}

fn validate_managed_deletion(
    snapshot: Option<&AppliedSnapshot>,
    deletion: &ManagedDeletion,
) -> Result<()> {
    let entry = snapshot
        .into_iter()
        .flat_map(|snapshot| &snapshot.files)
        .find(|entry| entry.path == *deletion.path())
        .ok_or_else(|| CoreError::UnsafeModpackPath {
            path: deletion.path().to_string(),
            reason: "delete candidate is absent from the applied snapshot".to_owned(),
        })?;
    if entry.policy != FilePolicy::Managed || entry.sha1 != *deletion.previous_sha1() {
        return Err(CoreError::UnsafeModpackPath {
            path: deletion.path().to_string(),
            reason: "delete candidate does not match the managed path, policy, and SHA-1 in the applied snapshot".to_owned(),
        });
    }
    Ok(())
}

async fn collect_disk_files(
    game_dir: &Path,
    root: &Path,
    manifest: &PackManifest,
    snapshot: Option<&AppliedSnapshot>,
) -> Result<Vec<DiskFile>> {
    let mut candidates = BTreeMap::<String, SafeRelativePath>::new();
    for path in manifest.files.iter().map(|file| &file.path).chain(
        snapshot
            .into_iter()
            .flat_map(|snapshot| snapshot.files.iter().map(|file| &file.path)),
    ) {
        validate_reserved_namespace(path)?;
        candidates
            .entry(path.as_str().to_lowercase())
            .or_insert_with(|| path.clone());
    }

    let mut disk = Vec::with_capacity(candidates.len());
    for path in candidates.into_values() {
        ensure_execution_path(game_dir, root, &path).await?;
        let resolved = path.resolve_under(root);
        match tokio::fs::metadata(&resolved).await {
            Ok(_) => {
                let actual = aurora_base::fs::sha1_hex(&resolved).await?;
                let digest = Sha1Digest::new(actual).map_err(|source| {
                    CoreError::ModpackMetadataConflict {
                        detail: format!("本地 SHA-1 计算结果非法: {source}"),
                    }
                })?;
                disk.push(DiskFile::new(path, digest));
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(aurora_base::Error::Io {
                    path: resolved,
                    source,
                }
                .into());
            }
        }
    }
    Ok(disk)
}

fn download_progress_bridge(
    events: Option<&EventSink>,
    total_bytes: u64,
) -> (
    Option<watch::Sender<DownloadProgress>>,
    Option<tokio::task::JoinHandle<()>>,
) {
    let Some(events) = events.cloned() else {
        return (None, None);
    };
    let (tx, mut rx) = watch::channel(DownloadProgress::default());
    let task = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let progress = *rx.borrow_and_update();
            let _ = events.send(CoreEvent::ModpackSync(ModpackSyncProgress {
                stage: ModpackSyncStage::DownloadingFiles,
                completed_files: progress.finished,
                total_files: progress.total,
                downloaded_bytes: progress.bytes,
                total_bytes: Some(total_bytes),
                current_file: None,
            }));
        }
    });
    (Some(tx), Some(task))
}

fn emit_progress(events: Option<&EventSink>, progress: ModpackSyncProgress) {
    emit(events, CoreEvent::ModpackSync(progress));
}

fn emit_stage(events: Option<&EventSink>, stage: ModpackSyncStage) {
    emit_progress(
        events,
        ModpackSyncProgress {
            stage,
            completed_files: 0,
            total_files: 0,
            downloaded_bytes: 0,
            total_bytes: None,
            current_file: None,
        },
    );
}

fn checked_at() -> Result<String> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|source| CoreError::ModpackMetadataConflict {
            detail: format!("系统时钟早于 Unix epoch: {source}"),
        })?
        .as_secs();
    Ok(seconds.to_string())
}

fn sync_core_error(
    target_version: &str,
    stage: ModpackSyncStage,
    error: CoreError,
) -> ModpackSyncError {
    let failure = match error {
        CoreError::ModpackLauncherTooOld { current, required } => {
            ModpackSyncFailure::LauncherTooOld { current, required }
        }
        CoreError::ModpackMetadataConflict { detail } => ModpackSyncFailure::Conflict { detail },
        CoreError::UnsafeModpackPath { path, reason } => ModpackSyncFailure::Filesystem {
            file_path: path,
            detail: reason,
        },
        CoreError::ModpackRemoteUnavailable { url, detail } => ModpackSyncFailure::Network {
            file_path: url,
            detail,
        },
        CoreError::Base(aurora_base::Error::Io { path, source }) => {
            classify_io(&path.display().to_string(), source, None)
        }
        CoreError::Modpack(err) => ModpackSyncFailure::InvalidMetadata {
            detail: err.to_string(),
        },
        other => ModpackSyncFailure::InvalidMetadata {
            detail: other.to_string(),
        },
    };
    ModpackSyncError {
        target_version: target_version.to_owned(),
        stage,
        failure,
    }
}

fn classify_download_failure(
    failure: &TaskFailure,
    failure_count: usize,
    root: &Path,
) -> ModpackSyncFailure {
    let file_path = failure
        .task
        .dest
        .strip_prefix(root)
        .unwrap_or(&failure.task.dest)
        .to_string_lossy()
        .replace('\\', "/");
    if let Some((expected, actual)) = hash_mismatch(&failure.error) {
        return ModpackSyncFailure::ChecksumMismatch {
            file_path,
            expected_sha1: expected,
            actual_sha1: actual,
        };
    }
    if let Some(source) = download_io(&failure.error) {
        return classify_io(&file_path, clone_io(source), failure.task.size);
    }
    ModpackSyncFailure::Network {
        file_path,
        detail: if failure_count == 1 {
            failure.error.to_string()
        } else {
            format!(
                "{} 个文件下载失败；首个错误: {}",
                failure_count, failure.error
            )
        },
    }
}

fn hash_mismatch(error: &aurora_download::Error) -> Option<(String, String)> {
    match error {
        aurora_download::Error::Base(aurora_base::Error::HashMismatch {
            expected, actual, ..
        }) => Some((expected.clone(), actual.clone())),
        aurora_download::Error::AllSourcesExhausted { last, .. } => hash_mismatch(last),
        _ => None,
    }
}

fn download_io(error: &aurora_download::Error) -> Option<&std::io::Error> {
    match error {
        aurora_download::Error::Base(aurora_base::Error::Io { source, .. }) => Some(source),
        aurora_download::Error::AllSourcesExhausted { last, .. } => download_io(last),
        _ => None,
    }
}

fn clone_io(source: &std::io::Error) -> std::io::Error {
    match source.raw_os_error() {
        Some(code) => std::io::Error::from_raw_os_error(code),
        None => std::io::Error::new(source.kind(), source.to_string()),
    }
}

fn classify_io(
    file_path: &str,
    source: std::io::Error,
    required_bytes: Option<u64>,
) -> ModpackSyncFailure {
    if source.kind() == std::io::ErrorKind::PermissionDenied {
        ModpackSyncFailure::PermissionDenied {
            file_path: file_path.to_owned(),
            detail: source.to_string(),
        }
    } else if source.kind() == std::io::ErrorKind::StorageFull
        || matches!(source.raw_os_error(), Some(28 | 112))
    {
        ModpackSyncFailure::DiskFull {
            file_path: file_path.to_owned(),
            required_bytes,
            available_bytes: None,
        }
    } else {
        ModpackSyncFailure::Filesystem {
            file_path: file_path.to_owned(),
            detail: source.to_string(),
        }
    }
}

fn validate_reserved_namespace(path: &SafeRelativePath) -> Result<()> {
    let first = path
        .as_str()
        .split('/')
        .next()
        .expect("安全路径至少有一个组件");
    if matches!(
        first.to_ascii_lowercase().as_str(),
        ".aurora" | "saves" | "screenshots" | "logs"
    ) {
        return Err(CoreError::UnsafeModpackPath {
            path: path.to_string(),
            reason: format!("顶层目录 {first} 属于启动器或玩家保留域"),
        });
    }
    Ok(())
}

async fn ensure_execution_path(
    game_dir: &Path,
    root: &Path,
    path: &SafeRelativePath,
) -> Result<()> {
    validate_reserved_namespace(path)?;
    let relative = path
        .as_str()
        .split('/')
        .fold(PathBuf::new(), |current, component| current.join(component));
    ensure_version_relative_path(game_dir, root, &relative)
        .await
        .map_err(|err| match err {
            CoreError::UnsafeModpackPath { reason, .. } => CoreError::UnsafeModpackPath {
                path: path.to_string(),
                reason,
            },
            other => other,
        })
}

async fn ensure_path_under_game_dir(game_dir: &Path, path: &Path) -> Result<()> {
    let relative = relative_to_game_dir(game_dir, path, Path::new(""))?;
    ensure_no_link_components(game_dir, &relative).await
}

pub(crate) async fn ensure_version_relative_path(
    game_dir: &Path,
    version_dir: &Path,
    relative: &Path,
) -> Result<()> {
    let relative = relative_to_game_dir(game_dir, version_dir, relative)?;
    ensure_no_link_components(game_dir, &relative).await
}

fn relative_to_game_dir(game_dir: &Path, base: &Path, suffix: &Path) -> Result<PathBuf> {
    let base_relative = base
        .strip_prefix(game_dir)
        .map_err(|_| CoreError::UnsafeModpackPath {
            path: base.display().to_string(),
            reason: format!(
                "path is outside the trusted game directory {}",
                game_dir.display()
            ),
        })?;
    let mut relative = base_relative.to_path_buf();
    relative.push(suffix);
    if relative.components().any(|component| {
        !matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    }) {
        return Err(CoreError::UnsafeModpackPath {
            path: relative.display().to_string(),
            reason: "path contains a component that can escape the game directory".to_owned(),
        });
    }
    Ok(relative)
}

async fn ensure_no_link_components(root: &Path, relative: &Path) -> Result<()> {
    let canonical_root = match tokio::fs::canonicalize(root).await {
        Ok(path) => Some(path),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(aurora_base::Error::Io {
                path: root.to_owned(),
                source,
            }
            .into());
        }
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if is_link_or_reparse(&metadata) => {
                return Err(CoreError::UnsafeModpackPath {
                    path: relative.display().to_string(),
                    reason: format!(
                        "既有路径组件 {} 是符号链接或 reparse point",
                        current.display()
                    ),
                });
            }
            Ok(_) => {
                if let Some(canonical_root) = &canonical_root {
                    let canonical_current =
                        tokio::fs::canonicalize(&current).await.map_err(|source| {
                            aurora_base::Error::Io {
                                path: current.clone(),
                                source,
                            }
                        })?;
                    if !canonical_current.starts_with(canonical_root) {
                        return Err(CoreError::UnsafeModpackPath {
                            path: relative.display().to_string(),
                            reason: format!(
                                "existing path component {} resolves outside the trusted root",
                                current.display()
                            ),
                        });
                    }
                }
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(aurora_base::Error::Io {
                    path: current,
                    source,
                }
                .into());
            }
        }
    }
    Ok(())
}

fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::config::AuroraConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn manifest(loader: &str, loader_version: &str) -> PackManifest {
        PackManifest::from_json_str(&format!(
            r#"{{
                "schema":1,
                "pack_id":"wok",
                "version":"2.0.0",
                "minecraft":"test",
                "loader":{{"kind":"{loader}","version":"{loader_version}"}},
                "files":[]
            }}"#
        ))
        .unwrap()
    }

    fn installer_zip(profile: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let cursor = Cursor::new(&mut bytes);
            let mut writer = zip::ZipWriter::new(cursor);
            writer
                .start_file(
                    "install_profile.json",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(profile.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    fn reservation(instance_id: &str, install_minecraft: bool) -> ModpackInstallReservation {
        let pointer = PackPointer::from_json_str(
            r#"{
                "pack_id":"wok",
                "version":"2.0.0",
                "manifest_url":"https://example.com/manifest.json",
                "released_at":"2026-08-17T12:00:00Z",
                "min_launcher_version":"0.1.0"
            }"#,
        )
        .unwrap();
        let subscription = ModpackSubscription {
            pack_id: "wok".to_owned(),
            pointer_url: "https://example.com/latest".to_owned(),
        };
        ModpackInstallReservation::new(
            instance_id,
            &subscription,
            &pointer,
            &manifest("fabric", "0.16.0"),
            install_minecraft,
        )
    }

    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) {
        match std::os::windows::fs::symlink_dir(target, link) {
            Ok(()) => {}
            Err(source) if source.raw_os_error() == Some(1314) => {
                let status = std::process::Command::new("cmd")
                    .args(["/c", "mklink", "/J"])
                    .arg(link)
                    .arg(target)
                    .status()
                    .expect("调用 mklink 创建测试 junction");
                assert!(status.success(), "创建测试 junction 失败: {status}");
            }
            Err(source) => panic!("创建目录符号链接失败: {source}"),
        }
    }

    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("测试需要创建目录符号链接");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fabric_and_quilt_preflight_use_profile_ids_without_writing_versions() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/versions/loader/test/0.16.0/profile/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"id":"fabric-loader-0.16.0-test","inheritsFrom":"test"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v3/versions/loader/test/0.27.1/profile/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"id":"quilt-loader-0.27.1-test","inheritsFrom":"test"}"#),
            )
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join(".minecraft");
        let aurora = Aurora::for_test(
            AuroraConfig::default(),
            tmp.path().to_path_buf(),
            game_dir.clone(),
        )
        .with_fabric_base(server.uri())
        .with_quilt_base(server.uri());

        let fabric = aurora
            .planned_install(&manifest("fabric", "0.16.0"))
            .await
            .unwrap();
        let quilt = aurora
            .planned_install(&manifest("quilt", "0.27.1"))
            .await
            .unwrap();

        assert_eq!(fabric.instance_id, "fabric-loader-0.16.0-test");
        assert!(fabric.prepared_installer.is_none());
        assert_eq!(quilt.instance_id, "quilt-loader-0.27.1-test");
        assert!(quilt.prepared_installer.is_none());
        assert!(!game_dir.join("versions").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn forge_and_neoforge_preflight_cache_and_inspect_exact_installer_ids() {
        let server = MockServer::start().await;
        let forge =
            installer_zip(r#"{"version":"forge-test","json":"/version.json","minecraft":"test"}"#);
        let neoforge = installer_zip(
            r#"{"version":"neoforge-test","json":"/version.json","minecraft":"test"}"#,
        );
        Mock::given(method("GET"))
            .and(path("/forge-installer.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(forge.clone()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/neoforge-installer.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(neoforge.clone()))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join(".minecraft");
        let aurora = Aurora::for_test(
            AuroraConfig::default(),
            tmp.path().to_path_buf(),
            game_dir.clone(),
        );

        let (forge_id, forge_path) = aurora
            .prepare_installer_plan(&format!("{}/forge-installer.jar", server.uri()), "test")
            .await
            .unwrap();
        let (neoforge_id, neoforge_path) = aurora
            .prepare_installer_plan(&format!("{}/neoforge-installer.jar", server.uri()), "test")
            .await
            .unwrap();

        assert_eq!(forge_id, "forge-test");
        assert_eq!(neoforge_id, "neoforge-test");
        assert_eq!(tokio::fs::read(&forge_path).await.unwrap(), forge);
        assert_eq!(tokio::fs::read(&neoforge_path).await.unwrap(), neoforge);
        assert_eq!(
            forge_path.parent(),
            Some(game_dir.join(".aurora").join("installer-cache").as_path())
        );
        assert_eq!(
            neoforge_path.parent(),
            Some(game_dir.join(".aurora").join("installer-cache").as_path())
        );
        assert!(!game_dir.join("versions").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn installer_cache_reparse_point_is_rejected_before_download() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/forge-installer.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(installer_zip(
                r#"{"version":"forge-test","json":"/version.json","minecraft":"test"}"#,
            )))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join(".minecraft");
        let outside = tmp.path().join("outside-cache");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(game_dir.join(".aurora")).unwrap();
        symlink_dir(&outside, &game_dir.join(".aurora").join("installer-cache"));
        let aurora = Aurora::for_test(AuroraConfig::default(), tmp.path().to_path_buf(), game_dir);

        let error = aurora
            .prepare_installer_plan(&format!("{}/forge-installer.jar", server.uri()), "test")
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::UnsafeModpackPath { ref reason, .. }
                if reason.contains("符号链接或 reparse point")
        ));
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn install_gate_is_exclusive_across_independent_file_handles() {
        use fs2::FileExt;

        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        let first = acquire_install_gate(&game_dir).await.unwrap();
        let second_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(game_dir.join(INSTALL_LOCK_FILE))
            .unwrap();
        assert!(install_lock_is_contended(
            &second_file.try_lock_exclusive().unwrap_err()
        ));
        assert!(matches!(
            acquire_install_gate(&game_dir).await.unwrap_err(),
            CoreError::ModpackMetadataConflict { .. }
        ));

        drop(first);
        second_file.try_lock_exclusive().unwrap();
    }

    #[tokio::test]
    async fn staging_owner_resumes_after_interruption_and_claims_target_and_base() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        let reservation = reservation("fabric-test", true);
        assert_eq!(
            acquire_install_reservation(&game_dir, &reservation)
                .await
                .unwrap(),
            InstallReservationState::Created
        );

        let staging_relative =
            Path::new(INSTALL_STAGING_DIR).join(format!("{}-target", reservation.operation_id));
        let owner_relative = staging_relative.join(INSTALL_OWNER_FILE);
        let owner = VersionDirectoryOwner::new(&reservation, &reservation.instance_id);
        aurora_base::fs::atomic_write(
            game_dir.join(&owner_relative),
            &serde_json::to_vec_pretty(&owner).unwrap(),
        )
        .await
        .unwrap();

        prepare_reserved_version_directories(&game_dir, &reservation, Some("test"))
            .await
            .unwrap();
        assert!(!game_dir.join(staging_relative).exists());
        assert!(
            load_version_directory_owner(&game_dir, "fabric-test")
                .await
                .unwrap()
                .is_some_and(|owner| owner.matches(&reservation, "fabric-test"))
        );
        assert!(
            load_version_directory_owner(&game_dir, "test")
                .await
                .unwrap()
                .is_some_and(|owner| owner.matches(&reservation, "test"))
        );

        complete_install_reservation(&game_dir, "fabric-test")
            .await
            .unwrap();
        assert!(
            load_install_reservation(&game_dir, "fabric-test")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            load_version_directory_owner(&game_dir, "fabric-test")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            load_version_directory_owner(&game_dir, "test")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn reservation_never_claims_an_unmarked_directory_that_appears_after_preflight() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        let reservation = reservation("fabric-test", true);
        acquire_install_reservation(&game_dir, &reservation)
            .await
            .unwrap();
        let foreign = game_dir.join("versions").join("fabric-test");
        tokio::fs::create_dir_all(&foreign).await.unwrap();
        tokio::fs::write(foreign.join("foreign.json"), b"foreign")
            .await
            .unwrap();

        let error = prepare_reserved_version_directories(&game_dir, &reservation, Some("test"))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            CoreError::ModpackMetadataConflict { ref detail }
                if detail.contains("not owned by reservation operation")
        ));
        assert_eq!(
            tokio::fs::read(foreign.join("foreign.json")).await.unwrap(),
            b"foreign"
        );
        assert!(
            load_version_directory_owner(&game_dir, "fabric-test")
                .await
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            ensure_no_pending_managed_install(&game_dir)
                .await
                .unwrap_err(),
            CoreError::ModpackMetadataConflict { .. }
        ));
    }

    #[tokio::test]
    async fn incomplete_owned_instance_is_locked_and_the_same_reservation_can_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        let reservation = reservation("fabric-test", true);
        acquire_install_reservation(&game_dir, &reservation)
            .await
            .unwrap();
        prepare_reserved_version_directories(&game_dir, &reservation, Some("test"))
            .await
            .unwrap();
        let target = game_dir.join("versions").join("fabric-test");
        tokio::fs::write(
            target.join("fabric-test.json"),
            br#"{"id":"fabric-test","inheritsFrom":"test","type":"release","mainClass":"m"}"#,
        )
        .await
        .unwrap();
        let aurora = Aurora::for_test(
            AuroraConfig::default(),
            tmp.path().to_path_buf(),
            game_dir.clone(),
        );

        let scan = aurora_instance::discover_versions(&game_dir).await.unwrap();
        assert!(
            scan.versions
                .iter()
                .any(|version| version.id == "fabric-test")
        );
        let status = aurora
            .managed_modpack_status("fabric-test")
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            status,
            ManagedModpackStatus::Unavailable {
                ref subscription,
                last_known: None,
                ref detail,
            } if subscription.pack_id == "wok" && detail.contains("incomplete")
        ));
        assert_eq!(
            aurora.managed_modpack_files("fabric-test").await.unwrap(),
            Some(Vec::new())
        );
        assert!(matches!(
            aurora
                .ensure_instance_not_pending_managed_install("fabric-test")
                .await
                .unwrap_err(),
            CoreError::ModpackMetadataConflict { ref detail }
                if detail.contains("incomplete managed modpack installation")
        ));
        assert!(matches!(
            aurora
                .launch_offline(
                    "fabric-test",
                    "Player",
                    &crate::launch::LaunchOptions::default(),
                    None,
                    None,
                )
                .await,
            Err(CoreError::ModpackMetadataConflict { .. })
        ));
        assert!(matches!(
            aurora
                .set_version_settings("fabric-test", &aurora_instance::VersionSettings::default(),)
                .await
                .unwrap_err(),
            CoreError::ModpackMetadataConflict { .. }
        ));
        assert!(!target.join(".aurora/settings.json").exists());

        assert_eq!(
            acquire_install_reservation(&game_dir, &reservation)
                .await
                .unwrap(),
            InstallReservationState::Existing
        );
        prepare_reserved_version_directories(&game_dir, &reservation, Some("test"))
            .await
            .unwrap();
    }

    #[test]
    fn legacy_forge_preflight_uses_version_info_id() {
        let installer = installer_zip(
            r#"{"install":{},"versionInfo":{"id":"1.7.10-Forge10.13.4.1614-1.7.10","inheritsFrom":"1.7.10"}}"#,
        );
        let id = inspect_installer_instance_id(&installer, "1.7.10").unwrap();
        assert_eq!(id, "1.7.10-Forge10.13.4.1614-1.7.10");
    }

    #[test]
    fn legacy_forge_preflight_requires_exact_minecraft_identity() {
        for profile in [
            r#"{"install":{},"versionInfo":{"id":"forge-test"}}"#,
            r#"{"install":{},"versionInfo":{"id":"forge-test","inheritsFrom":"1.12.2"}}"#,
        ] {
            let error =
                inspect_installer_instance_id(&installer_zip(profile), "1.7.10").unwrap_err();
            assert!(matches!(
                error,
                CoreError::ModpackMetadataConflict { ref detail }
                    if detail.contains("manifest requires 1.7.10")
            ));
        }
    }
}
