// 前端 IPC 层：aurora-tauri 命令的类型化封装 + 进度/事件订阅。
// 约定（Tauri v2 官方）：invoke 命令名保持 snake_case 原样；参数键用 camelCase（映射到 Rust 的
// snake_case 形参）；返回 DTO 字段是 serde 默认的 snake_case。页面只调用本文件导出的函数，不直接
// import @tauri-apps/api，保证调用点集中、可测。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---- 与后端 serde 枚举对应的字面量联合（均为 snake_case）----
export type DownloadSourcePolicy = "auto" | "official_first" | "mirror_first";
export type IsolationPolicy =
  | "disabled"
  | "mod_loaders_only"
  | "non_release_only"
  | "mod_loaders_and_non_release"
  | "all";
export type AccountType = "microsoft" | "offline" | "authlib_injector";
export type LoaderChoice = "fabric" | "quilt" | "forge" | "neoforge";
export type PlatformId = "modrinth" | "curseforge";
export type ResourceType = "mod" | "modpack" | "resource_pack" | "shader" | "data_pack" | "plugin";
export type ModLoader = "fabric" | "quilt" | "forge" | "neoforge" | "liteloader";
export type SortField = "relevance" | "downloads" | "follows" | "newest" | "updated";

export interface MemorySettings {
  max_mb: number;
  min_mb: number | null;
}

export interface ConfigDto {
  game_dir: string;
  data_dir: string;
  download_source: DownloadSourcePolicy;
  version_list_source: DownloadSourcePolicy;
  download_concurrency: number;
  memory: MemorySettings;
  isolation_policy: IsolationPolicy;
  has_client_id: boolean;
  auto_download_java: boolean;
}

export interface LoaderDto {
  kind: string;
  version: string | null;
}

export interface InstalledVersionDto {
  id: string;
  is_release: boolean;
  has_mod_loader: boolean;
  loaders: LoaderDto[];
}

export interface BrokenVersionDto {
  id: string;
  reason: string;
}

export interface VersionScanDto {
  versions: InstalledVersionDto[];
  broken: BrokenVersionDto[];
}

export interface AccountDto {
  uuid: string;
  name: string;
  account_type: AccountType;
}

export interface ManifestVersionDto {
  id: string;
  release_type: string;
  url: string;
  time: string;
  release_time: string;
  sha1: string | null;
  compliance_level: number | null;
}

export interface ManifestDto {
  latest: { release: string; snapshot: string };
  versions: ManifestVersionDto[];
}

export interface InstallOutcomeDto {
  vanilla: { id: string; libraries: number; assets: number; natives: number };
  loader: { id: string; loader_version: string; libraries: number } | null;
}

export interface JavaVersionDto {
  major: number;
  minor: number;
  security: number;
  build: number;
  raw: string;
}

export interface JavaInstallationDto {
  path: string;
  version: JavaVersionDto;
  is_64bit: boolean;
  vendor: string;
  source: "registry" | "common_dir" | "path" | "managed";
}

export interface InstalledRuntimeDto {
  component: string;
  version: JavaVersionDto;
  java_executable: string;
}

export interface LaunchedDto {
  pid: number | null;
}

export interface SearchHit {
  platform: PlatformId;
  project_id: string;
  slug: string | null;
  title: string;
  description: string;
  author: string | null;
  downloads: number;
  follows: number | null;
  icon_url: string | null;
  categories: string[];
  resource_type: ResourceType;
  date_modified: string | null;
  page_url: string | null;
}

export interface SearchResultDto {
  hits: SearchHit[];
  errors: { platform: PlatformId; message: string }[];
}

export interface ModInstallOutcomeDto {
  file_name: string;
  path: string;
  platform: PlatformId;
}

export interface ModMetadata {
  mod_id: string;
  name: string | null;
  version: string | null;
  description: string | null;
  authors: string[];
  loader: ModLoader;
  format: string;
}

export interface InstalledMod {
  path: string;
  file_name: string;
  enabled: boolean;
  metadata: ModMetadata | null;
}

// ---- 事件负载 ----
export type CoreEvent =
  | { kind: "stage"; message: string }
  | { kind: "warning"; message: string }
  | { kind: "download"; total: number; finished: number; bytes: number; speed: number };

export interface DeviceCode {
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
  message: string;
}

export interface GameLog {
  stream: "stdout" | "stderr";
  text: string;
}

// 与后端常量保持一致。
export const CORE_EVENT = "aurora://core-event";
export const DEVICE_CODE_EVENT = "aurora://device-code";
export const GAME_LOG_EVENT = "aurora://game-log";

// ---- 命令封装（参数键 camelCase）----
export const getConfig = (): Promise<ConfigDto> => invoke<ConfigDto>("get_config");

export const listInstalled = (): Promise<VersionScanDto> =>
  invoke<VersionScanDto>("list_installed");

export const currentAccount = (): Promise<AccountDto | null> =>
  invoke<AccountDto | null>("current_account");

export const createOfflineAccount = (name: string): Promise<AccountDto> =>
  invoke<AccountDto>("create_offline_account", { name });

// 微软设备码登录：invoke 会在整个登录完成时 resolve；期间订阅 onDeviceCode 展示待输入短码。
export const microsoftLogin = (): Promise<AccountDto> => invoke<AccountDto>("microsoft_login");

export const authlibLogin = (
  serverUrl: string,
  username: string,
  password: string,
): Promise<AccountDto> => invoke<AccountDto>("authlib_login", { serverUrl, username, password });

export const listAccounts = (): Promise<AccountDto[]> => invoke<AccountDto[]>("list_accounts");

export const setCurrentAccount = (uuid: string): Promise<void> =>
  invoke("set_current_account", { uuid });

export const removeAccount = (uuid: string): Promise<void> => invoke("remove_account", { uuid });

export const listManifest = (): Promise<ManifestDto> => invoke<ManifestDto>("list_manifest");

export const installVersion = (
  id: string,
  loader?: LoaderChoice,
  loaderVersion?: string,
): Promise<InstallOutcomeDto> =>
  invoke<InstallOutcomeDto>("install_version", { id, loader: loader ?? null, loaderVersion: loaderVersion ?? null });

export interface LaunchArgs {
  versionId: string;
  accountUuid?: string;
  offlineName?: string;
  maxMemoryMb?: number;
  minMemoryMb?: number;
  fullscreen?: boolean;
  extraJvmArgs?: string[];
  extraGameArgs?: string[];
  resolution?: [number, number];
  demo?: boolean;
}

export const launchGame = (args: LaunchArgs): Promise<LaunchedDto> =>
  invoke<LaunchedDto>("launch_game", {
    versionId: args.versionId,
    accountUuid: args.accountUuid ?? null,
    offlineName: args.offlineName ?? null,
    maxMemoryMb: args.maxMemoryMb ?? null,
    minMemoryMb: args.minMemoryMb ?? null,
    fullscreen: args.fullscreen ?? false,
    extraJvmArgs: args.extraJvmArgs ?? [],
    extraGameArgs: args.extraGameArgs ?? [],
    resolution: args.resolution ?? null,
    demo: args.demo ?? false,
  });

export const stopGame = (): Promise<void> => invoke("stop_game");

export const detectJava = (): Promise<JavaInstallationDto[]> =>
  invoke<JavaInstallationDto[]>("detect_java");

export const installJava = (requiredMajor: number): Promise<InstalledRuntimeDto> =>
  invoke<InstalledRuntimeDto>("install_java", { requiredMajor });

export interface ConfigPatch {
  downloadSource?: DownloadSourcePolicy;
  versionListSource?: DownloadSourcePolicy;
  downloadConcurrency?: number;
  memory?: MemorySettings;
  isolationPolicy?: IsolationPolicy;
  autoDownloadJava?: boolean;
  cacheDirectory?: string;
  clientId?: string;
}

export const updateConfig = (patch: ConfigPatch): Promise<void> => invoke("update_config", { ...patch });

export const setGameDirectory = (path: string): Promise<void> =>
  invoke("set_game_directory", { path });

export interface SearchArgs {
  query?: string;
  resourceType: ResourceType;
  loaders?: ModLoader[];
  gameVersions?: string[];
  sort?: SortField;
  limit?: number;
  offset?: number;
}

export const searchResources = (args: SearchArgs): Promise<SearchResultDto> =>
  invoke<SearchResultDto>("search_resources", {
    query: args.query ?? null,
    resourceType: args.resourceType,
    loaders: args.loaders ?? [],
    gameVersions: args.gameVersions ?? [],
    sort: args.sort ?? "relevance",
    limit: args.limit ?? 20,
    offset: args.offset ?? 0,
  });

export const installMod = (
  versionId: string,
  platform: PlatformId,
  projectId: string,
  modVersionId: string,
): Promise<ModInstallOutcomeDto> =>
  invoke<ModInstallOutcomeDto>("install_mod", { versionId, platform, projectId, modVersionId });

export const listMods = (versionId: string): Promise<InstalledMod[]> =>
  invoke<InstalledMod[]>("list_mods", { versionId });

export const setModEnabled = (
  versionId: string,
  fileName: string,
  enabled: boolean,
): Promise<string> => invoke<string>("set_mod_enabled", { versionId, fileName, enabled });

// ---- 事件订阅 ----
// 组件卸载或流程结束务必调用返回的 unlisten，避免监听器泄漏。
export const onCoreEvent = (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
  listen<CoreEvent>(CORE_EVENT, (e) => handler(e.payload));

export const onDeviceCode = (handler: (code: DeviceCode) => void): Promise<UnlistenFn> =>
  listen<DeviceCode>(DEVICE_CODE_EVENT, (e) => handler(e.payload));

export const onGameLog = (handler: (line: GameLog) => void): Promise<UnlistenFn> =>
  listen<GameLog>(GAME_LOG_EVENT, (e) => handler(e.payload));
