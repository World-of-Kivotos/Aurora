/**
 * 整合包 UI 的前端契约。
 *
 * 字段沿用 aurora-core 经 Tauri 序列化后的 snake_case，视图层无需重复改名。
 */

export interface ModpackSubscription {
  pack_id: string;
  pointer_url: string;
}

export interface ModpackRelease {
  pack_id: string;
  version: string;
  manifest_url: string;
  released_at: string;
  note: string | null;
  min_launcher_version: string;
}

export interface KnownModpackVersions {
  installed_version: string | null;
  latest: ModpackRelease;
}

export type ManagedModpackStatus =
  | {
      kind: "checking";
      subscription: ModpackSubscription;
      last_known: KnownModpackVersions | null;
    }
  | {
      kind: "ready";
      subscription: ModpackSubscription;
      versions: KnownModpackVersions;
      source: "network" | "cache";
      /** 检查完成时刻的 Unix 秒数字符串。 */
      checked_at: string;
    }
  | {
      kind: "unavailable";
      subscription: ModpackSubscription;
      last_known: KnownModpackVersions | null;
      detail: string;
    };

export type CheckedManagedModpackStatus = Exclude<ManagedModpackStatus, { kind: "checking" }>;

export type ModpackSyncStage =
  | "resolving_manifest"
  | "installing_minecraft"
  | "installing_loader"
  | "downloading_files"
  | "deleting_files"
  | "writing_snapshot";

export interface ModpackSyncProgress {
  stage: ModpackSyncStage;
  completed_files: number;
  total_files: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  current_file: string | null;
}

interface FileFailure {
  file_path: string;
}

export type ModpackSyncFailure =
  | (FileFailure & {
      kind: "network";
      detail: string;
    })
  | (FileFailure & {
      kind: "checksum_mismatch";
      expected_sha1: string;
      actual_sha1: string;
    })
  | (FileFailure & {
      kind: "disk_full";
      required_bytes: number | null;
      available_bytes: number | null;
    })
  | (FileFailure & {
      kind: "permission_denied";
      detail: string;
    })
  | (FileFailure & {
      kind: "snapshot_write";
      detail: string;
    })
  | {
      kind: "invalid_metadata";
      detail: string;
    }
  | {
      kind: "launcher_too_old";
      current: string;
      required: string;
    }
  | {
      kind: "conflict";
      detail: string;
    }
  | (FileFailure & {
      kind: "filesystem";
      detail: string;
    });

export interface ModpackSyncError {
  target_version: string;
  stage: ModpackSyncStage;
  failure: ModpackSyncFailure;
}

export interface ModpackSyncOutcome {
  installed_version: string;
  downloaded_files: number;
  deleted_files: number;
  kept_files: number;
}

export interface ModpackInstallOutcome {
  instance_id: string;
  installed_version: string;
}

export type ModpackSyncState =
  | { kind: "idle" }
  | { kind: "running"; target_version: string; progress: ModpackSyncProgress }
  | {
      kind: "failed";
      target_version: string;
      stage: ModpackSyncStage;
      failure: ModpackSyncFailure;
    }
  | { kind: "complete"; installed_version: string };

export type ModpackFileOwner = "modpack" | "player";

export function canRemoveModpackFile(owner: ModpackFileOwner): boolean {
  return owner === "player";
}

export interface SyncFailurePresentation {
  title: string;
  reason: string;
  action: string;
}

const SYNC_STAGES: ReadonlySet<string> = new Set<ModpackSyncStage>([
  "resolving_manifest",
  "installing_minecraft",
  "installing_loader",
  "downloading_files",
  "deleting_files",
  "writing_snapshot",
]);

function recordOf(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : null;
}

function isNullableNumber(value: unknown): value is number | null {
  return value === null || typeof value === "number";
}

function isModpackSyncFailure(value: unknown): value is ModpackSyncFailure {
  const failure = recordOf(value);
  if (!failure || typeof failure.kind !== "string") return false;

  const hasPath = typeof failure.file_path === "string";
  const hasDetail = typeof failure.detail === "string";
  switch (failure.kind) {
    case "network":
    case "permission_denied":
    case "snapshot_write":
    case "filesystem":
      return hasPath && hasDetail;
    case "checksum_mismatch":
      return hasPath && typeof failure.expected_sha1 === "string" && typeof failure.actual_sha1 === "string";
    case "disk_full":
      return hasPath && isNullableNumber(failure.required_bytes) && isNullableNumber(failure.available_bytes);
    case "invalid_metadata":
    case "conflict":
      return hasDetail;
    case "launcher_too_old":
      return typeof failure.current === "string" && typeof failure.required === "string";
    default:
      return false;
  }
}

/** Tauri 会以结构化 rejection 返回同步错误；只接受完整契约，避免把普通 Error 误当成可重试同步失败。 */
export function parseModpackSyncError(value: unknown): ModpackSyncError | null {
  const error = recordOf(value);
  if (
    !error ||
    typeof error.target_version !== "string" ||
    typeof error.stage !== "string" ||
    !SYNC_STAGES.has(error.stage) ||
    !isModpackSyncFailure(error.failure)
  ) {
    return null;
  }
  return error as unknown as ModpackSyncError;
}

export const SYNC_STAGE_LABEL: Record<ModpackSyncStage, string> = {
  resolving_manifest: "读取整合包清单",
  installing_minecraft: "安装 Minecraft",
  installing_loader: "安装加载器",
  downloading_files: "下载并校验文件",
  deleting_files: "移除旧的受管文件",
  writing_snapshot: "保存同步快照",
};

export function formatModpackBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    throw new RangeError("字节数必须是非负有限数");
  }
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

export function syncProgressRatio(progress: ModpackSyncProgress): number {
  const raw =
    progress.total_bytes !== null && progress.total_bytes > 0
      ? progress.downloaded_bytes / progress.total_bytes
      : progress.total_files > 0
        ? progress.completed_files / progress.total_files
        : 0;
  return Math.min(1, Math.max(0, raw));
}

function diskSpaceReason(failure: Extract<ModpackSyncFailure, { kind: "disk_full" }>): string {
  if (failure.required_bytes === null || failure.available_bytes === null) {
    return "目标磁盘没有足够空间写入该文件。";
  }
  return `本次写入需要 ${formatModpackBytes(failure.required_bytes)}，当前可用 ${formatModpackBytes(failure.available_bytes)}。`;
}

export function presentSyncFailure(failure: ModpackSyncFailure): SyncFailurePresentation {
  switch (failure.kind) {
    case "network":
      return {
        title: `下载失败：${failure.file_path}`,
        reason: failure.detail,
        action: "检查网络连接或代理设置后重试。已校验完成的文件不会重复下载。",
      };
    case "checksum_mismatch":
      return {
        title: `校验不符：${failure.file_path}`,
        reason: `期望 SHA-1 ${failure.expected_sha1}，实际为 ${failure.actual_sha1}。`,
        action: "重试下载；若仍然不符，请把文件名和校验值发给整合包维护者。",
      };
    case "disk_full":
      return {
        title: `磁盘空间不足：${failure.file_path}`,
        reason: diskSpaceReason(failure),
        action: "释放实例所在磁盘的空间后重试。同步尚未删除任何旧文件。",
      };
    case "permission_denied":
      return {
        title: `无法写入：${failure.file_path}`,
        reason: failure.detail,
        action: "关闭正在占用该文件的程序，确认目录可写后重试。",
      };
    case "snapshot_write":
      return {
        title: `无法保存同步快照：${failure.file_path}`,
        reason: failure.detail,
        action: "确认实例目录可写后重试。快照未写入前，本次同步不会被标记为完成。",
      };
    case "invalid_metadata":
      return {
        title: "整合包元数据无效",
        reason: failure.detail,
        action: "请把错误详情发给整合包维护者，修复发布数据后再重新检查。",
      };
    case "launcher_too_old":
      return {
        title: "启动器版本过低",
        reason: `当前 Aurora 为 ${failure.current}，整合包要求 ${failure.required} 或更高版本。`,
        action: "先更新 Aurora，然后重新检查并同步整合包。",
      };
    case "conflict":
      return {
        title: "整合包状态已变化",
        reason: failure.detail,
        action: "当前实例不能安全地原地应用此次变更。请作为新实例安装，现有实例与玩家数据会保留。",
      };
    case "filesystem":
      return {
        title: `文件系统操作失败：${failure.file_path}`,
        reason: failure.detail,
        action: "确认该路径位于实例目录内且没有被其它程序占用，处理后再重试。",
      };
  }
}

export function modpackUpdateAvailable(versions: KnownModpackVersions): boolean {
  return versions.installed_version !== versions.latest.version;
}

export function validateModpackPointerUrl(value: string): string | null {
  const trimmed = value.trim();
  if (trimmed === "") return "请输入整合包地址";

  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return "整合包地址不是有效 URL";
  }

  if (url.protocol !== "https:" && url.protocol !== "http:") {
    return "整合包地址只支持 HTTP 或 HTTPS";
  }
  if (url.hostname === "") {
    return "整合包地址必须包含主机名";
  }
  if (url.username !== "" || url.password !== "") {
    return "整合包地址不能包含账号或密码";
  }
  if (url.hash !== "") {
    return "整合包地址不能包含片段标识";
  }
  return null;
}
