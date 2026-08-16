/**
 * 整合包 UI 的前端契约。
 *
 * 这些类型刻意不声明 Tauri command：阶段 4 的后端 IPC 尚未确定，组件只消费数据与回调。
 * 字段沿用服务端 JSON 的 snake_case，真实 DTO 到位后无需在视图层重复改名。
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
      checked_at: string;
    }
  | {
      kind: "unavailable";
      subscription: ModpackSubscription;
      last_known: KnownModpackVersions | null;
      detail: string;
    };

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
    });

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
  if (url.username !== "" || url.password !== "") {
    return "整合包地址不能包含账号或密码";
  }
  return null;
}
