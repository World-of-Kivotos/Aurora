import type { ManagedModpackFile } from "./ipc";
import type { ManagedModpackStatus, ModpackFileOwner } from "./modpack-ui";

const DISABLED_SUFFIX = ".disabled";

function enabledFileName(fileName: string): string {
  return fileName.endsWith(DISABLED_SUFFIX)
    ? fileName.slice(0, -DISABLED_SUFFIX.length)
    : fileName;
}

function comparisonKey(path: string): string {
  return path.replace(/\\/g, "/").toLowerCase();
}

function installedVersionOf(status: ManagedModpackStatus): string | null {
  if (status.kind === "ready") return status.versions.installed_version;
  return status.last_known?.installed_version ?? null;
}

/** undefined 表示仍在读取，status/files 同为 null 才表示普通实例。 */
export function modpackOwnerOf(
  status: ManagedModpackStatus | null | undefined,
  files: readonly ManagedModpackFile[] | null | undefined,
  fileName: string,
): ModpackFileOwner | null {
  if (status === undefined || files === undefined || status?.kind === "checking") return null;
  if (status === null || files === null) {
    return status === null && files === null ? "player" : null;
  }
  if (installedVersionOf(status) === null) return null;
  const relativePath = comparisonKey(`mods/${enabledFileName(fileName)}`);
  return files.some(
    (file) => file.policy === "managed" && comparisonKey(file.path) === relativePath,
  )
    ? "modpack"
    : "player";
}
