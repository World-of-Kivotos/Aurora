// 前端 IPC 层：aurora-tauri 命令的类型化封装 + 进度/事件订阅。
// 约定（Tauri v2 官方）：invoke 命令名保持 snake_case 原样；参数键用 camelCase（映射到 Rust 的
// snake_case 形参）；返回 DTO 字段是 serde 默认的 snake_case。页面只调用本文件导出的函数，不直接
// import @tauri-apps/api，保证调用点集中、可测。

import { invoke, listen, type UnlistenFn } from "./tauri-bridge";

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
  selected_version: string | null;
}

export interface LoaderDto {
  kind: string;
  version: string | null;
}

export interface InstalledVersionDto {
  id: string;
  mc_version: string;
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
/** 游戏异常退出时后端推送的崩溃诊断；玩家主动结束游戏不会触发（见 detect_crash 的主动终止短路）。 */
export const GAME_CRASH_EVENT = "aurora://game-crash";

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
  selectedVersion?: string;
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

// ---- 版本级设置 ----

/** 版本级隔离覆盖：跟随全局档位，或让这一个实例强制开/关。 */
export type IsolationOverride = "follow_global" | "enabled" | "disabled";

/** 版本设置 + 按该设置解析出的实际工作目录状态（后端一并返回，界面据此回显"文件落在哪"）。 */
export interface VersionSettingsDto {
  description: string | null;
  icon: string | null;
  favorite: boolean;
  category: string | null;
  isolation: IsolationOverride;
  /** 该实例的游戏工作目录绝对路径；mod 装进它下面的 mods/。 */
  working_dir: string;
  /** 最终是否隔离（已综合全局档位、版本级覆盖与本地数据强制）。 */
  isolated: boolean;
  /** 因版本目录下已有 mods/saves 而被强制隔离——此时把覆盖设为「不隔离」也不会生效。 */
  forced_by_local_data: boolean;
}

/** 写入用入参：整体覆盖语义，读出完整对象改完写回（避免 patch 下无法区分"不改"与"清空"）。 */
export interface VersionSettingsInput {
  description?: string | null;
  icon?: string | null;
  favorite?: boolean;
  category?: string | null;
  isolation?: IsolationOverride;
}

export const getVersionSettings = (versionId: string): Promise<VersionSettingsDto> =>
  invoke<VersionSettingsDto>("get_version_settings", { versionId });

export const setVersionSettings = (
  versionId: string,
  settings: VersionSettingsInput,
): Promise<VersionSettingsDto> =>
  invoke<VersionSettingsDto>("set_version_settings", { versionId, settings });

// ---- 事件订阅 ----
// 组件卸载或流程结束务必调用返回的 unlisten，避免监听器泄漏。
export const onCoreEvent = (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
  listen<CoreEvent>(CORE_EVENT, (e) => handler(e.payload));

export const onDeviceCode = (handler: (code: DeviceCode) => void): Promise<UnlistenFn> =>
  listen<DeviceCode>(DEVICE_CODE_EVENT, (e) => handler(e.payload));

export const onGameLog = (handler: (line: GameLog) => void): Promise<UnlistenFn> =>
  listen<GameLog>(GAME_LOG_EVENT, (e) => handler(e.payload));

/** 游戏异常退出后的崩溃诊断推送。负载即 last_crash 的同款报告，收到时日志已归档完毕。 */
export const onGameCrash = (handler: (report: CrashReport) => void): Promise<UnlistenFn> =>
  listen<CrashReport>(GAME_CRASH_EVENT, (e) => handler(e.payload));

// ---- Mod 生态：版本列表 / 落位矩阵 / 依赖计划 / 卷宗 / 更新 / 历史 / 崩溃诊断 ----
// 这一组 DTO 由 aurora-core 直出（Tauri 层不再包一层壳），字段名即 Rust serde 的 snake_case。

/** 发布通道。beta/alpha 必须在界面上显式标注，避免玩家在不知情的前提下装到预览版。 */
export type ReleaseChannel = "release" | "beta" | "alpha";

/** 依赖关系类型。安装计划只自动收 required，其余仅作展示，不替玩家做决定。 */
export type DependencyKind = "required" | "optional" | "incompatible" | "embedded";

export interface ModDependency {
  /** Modrinth 为 project_id，CurseForge 为 modId 的十进制字符串；平台没给为 null。 */
  project_id: string | null;
  /** 依赖锁定了精确版本时才有值，否则交由依赖解析择优。 */
  version_id: string | null;
  kind: DependencyKind;
}

/** 工程版本的跨平台统一视图：Modrinth 的 version 与 CurseForge 的 file 都归一到它。 */
export interface ModVersionInfo {
  /** 安装时回传给后端的版本标识：Modrinth 为版本 id，CurseForge 为 fileId 的十进制字符串。 */
  version_id: string;
  project_id: string;
  platform: PlatformId;
  name: string;
  version_number: string;
  release_channel: ReleaseChannel;
  /** 已剥离加载器名的纯 MC 版本号。空数组表示平台没给元数据，不等于不支持。 */
  game_versions: string[];
  /** 小写加载器名，同样可能为空（平台元数据缺失）。 */
  loaders: string[];
  file_name: string;
  file_size: number | null;
  sha1: string | null;
  /** ISO 8601；平台缺失该字段时为空串。 */
  date_published: string;
  dependencies: ModDependency[];
}

/** 兼容判定。unknown 是「平台没给足够元数据」，界面上作软提示放行，不能当成不兼容拦下来。 */
export type Compatibility =
  | { kind: "match" }
  | { kind: "mismatch"; reason: string }
  | { kind: "unknown" };

/** 某个已装实例对某工程的落位判定。后端已按「完美匹配 > 可能可行 > 不兼容」排好序，界面直接按序渲染。 */
export interface InstanceMatch {
  version_id: string;
  mc_version: string;
  /** 该实例已装的加载器种类（小写名）。 */
  loaders: string[];
  compatibility: Compatibility;
  /** 该实例下最合适的版本；没有兼容版本时为 null。 */
  best_version: ModVersionInfo | null;
  /** 该工程已装在此实例里时给出文件名（卷宗与磁盘 join 的结果），据此把「安装」改成「更新/重装」。 */
  already_installed: string | null;
}

export interface PlannedItem {
  version: ModVersionInfo;
  /** 因谁被带进来（project_id）；用户主动选的那项为 null。 */
  required_by: string | null;
  /** 该实例已装同工程同版本，本次跳过下载。 */
  already_satisfied: boolean;
}

/** 一次安装的完整计划。items[0] 恒为用户主动选的那项。 */
export interface InstallPlan {
  items: PlannedItem[];
  /** 被跳过的非必需依赖、或找不到匹配版本的依赖，每条一句中文；界面必须如实展示而不是假装没有。 */
  skipped: string[];
}

/** 卷宗条目：Mod 身份的单一来源。file_name 是与磁盘 join 的键——磁盘才是权威，卷宗只补身份。 */
export interface LedgerEntry {
  file_name: string;
  platform: PlatformId;
  project_id: string;
  version_id: string;
  sha1: string | null;
  /** 安装时刻 unix 秒。 */
  installed_at: number;
  /** 作为谁的依赖被带进来（project_id）；用户主动装的为 null。 */
  installed_as_dependency_of: string | null;
}

export interface Ledger {
  entries: LedgerEntry[];
}

export interface UpdateCandidate {
  file_name: string;
  current_version_id: string;
  latest: ModVersionInfo;
}

/** 变更事件。追加式，永不改写既有条目；update 的旧文件以 `<old_file>.old` 留在原目录等待回滚。 */
export type HistoryEvent =
  | { kind: "install"; id: string; at: number; files: string[] }
  | {
      kind: "update";
      id: string;
      at: number;
      file_name: string;
      old_file: string;
      from_version: string;
      to_version: string;
    }
  | { kind: "rollback"; id: string; at: number; reverted_event: string }
  | { kind: "remove"; id: string; at: number; files: string[] };

/** 与 DOM 的全局 History 同名，模块作用域内以本定义为准；名字由跨模块契约锁定，不要另起别名。 */
export interface History {
  events: HistoryEvent[];
}

export interface RollbackCheck {
  event_id: string;
  can_rollback: boolean;
  /** 不可回滚时给出的中文原因；可回滚时为 null。 */
  reason: string | null;
}

/**
 * 一条崩溃诊断。category 取自 aurora-launch 的 CrashCategory，八种取值：
 * java_version_mismatch / out_of_memory / missing_dependency / mixin_failure /
 * duplicate_mod / native_library_missing / graphics_driver / corrupted_jar。
 */
export interface CrashDiagnosis {
  category: string;
  summary: string;
  advice: string;
  /** 正则从日志提取的附加信息（缺失的 Mod id、要求的 Java 版本等）；没提到为 null。 */
  detail: string | null;
  /** 命中的原始日志行（后端已裁剪长度）。 */
  matched: string;
}

/** 可疑 Mod。规则命中只是线索不是定论，文案一律写「日志指向 X」，不写「X 导致崩溃」。 */
export interface CrashSuspect {
  mod_id: string;
  /** 卷宗里对得上的文件名；对不上时只报 mod id。 */
  file_name: string | null;
}

export interface CrashReport {
  diagnoses: CrashDiagnosis[];
  suspects: CrashSuspect[];
  /** 归档日志路径，供「打开日志」用；没有归档为 null。 */
  log_path: string | null;
}

/** 列出工程的全部可用版本，按发布时间倒序。两个过滤条件传空数组表示不过滤。 */
export const listModVersions = (
  platform: PlatformId,
  projectId: string,
  gameVersions: string[] = [],
  loaders: ModLoader[] = [],
): Promise<ModVersionInfo[]> =>
  invoke<ModVersionInfo[]>("list_mod_versions", { platform, projectId, gameVersions, loaders });

/** 算出「全部已装实例 × 该工程最佳版本」的判定矩阵，供落位层直接铺开——不强制玩家先进实例。 */
export const matchInstances = (platform: PlatformId, projectId: string): Promise<InstanceMatch[]> =>
  invoke<InstanceMatch[]>("match_instances", { platform, projectId });

export const planInstall = (
  versionId: string,
  platform: PlatformId,
  projectId: string,
  modVersionId: string,
): Promise<InstallPlan> =>
  invoke<InstallPlan>("plan_install", { versionId, platform, projectId, modVersionId });

/** 对卷宗里没有身份的已装 Mod 做哈希反查补身份，返回补上的条数（反查不到的本地 Mod 会被跳过）。 */
export const identifyInstalledMods = (versionId: string): Promise<number> =>
  invoke<number>("identify_installed_mods", { versionId });

export const checkUpdates = (versionId: string): Promise<UpdateCandidate[]> =>
  invoke<UpdateCandidate[]>("check_updates", { versionId });

export const listHistory = (versionId: string): Promise<History> =>
  invoke<History>("list_history", { versionId });

export const rollbackChecks = (versionId: string): Promise<RollbackCheck[]> =>
  invoke<RollbackCheck[]>("rollback_checks", { versionId });

export const rollback = (versionId: string, eventId: string): Promise<void> =>
  invoke("rollback", { versionId, eventId });

/** `.old` 备份占用的总字节数，用于把磁盘代价显式告诉玩家。 */
export const backupSize = (versionId: string): Promise<number> =>
  invoke<number>("backup_size", { versionId });

export const diagnoseCrash = (versionId: string, logText: string): Promise<CrashReport> =>
  invoke<CrashReport>("diagnose_crash", { versionId, logText });

/** 读取该实例最近一次归档日志并诊断；没有归档返回 null。 */
export const lastCrash = (versionId: string): Promise<CrashReport | null> =>
  invoke<CrashReport | null>("last_crash", { versionId });

export const listLedger = (versionId: string): Promise<Ledger> =>
  invoke<Ledger>("list_ledger", { versionId });

// ---- 游戏目录与初次设定 ----

/** 一条带名字的目录记录。名字给人看，路径才是身份。 */
export interface NamedDirectory {
  name: string;
  path: string;
}

/** 已知游戏目录，含当前是否可达——盘没挂时记录仍在，只是 available 为 false。 */
export interface GameDirectoryEntry {
  name: string;
  path: string;
  /** 是否为当前正在使用的目录（安装与启动都落在它里面）。 */
  is_current: boolean;
  /** 该目录此刻是否真实存在。 */
  available: boolean;
}

/** 配置文件是否还没落过盘；为真表示这是第一次启动，该走初次设定。 */
export const isFirstRun = (): Promise<boolean> => invoke<boolean>("is_first_run");

export const listGameDirectories = (): Promise<GameDirectoryEntry[]> =>
  invoke<GameDirectoryEntry[]>("list_game_directories");

/** 探测机器上尚未记录的其它 .minecraft（官方启动器、PCL2 等）；只报告，不写入配置。 */
export const discoverGameDirectories = (): Promise<NamedDirectory[]> =>
  invoke<NamedDirectory[]>("discover_game_directories");

export const addGameDirectory = (name: string, path: string): Promise<void> =>
  invoke("add_game_directory", { name, path });

export const removeGameDirectory = (path: string): Promise<boolean> =>
  invoke<boolean>("remove_game_directory", { path });

/** 切换当前游戏目录；原当前目录会自动转入「其它文件夹」，不会丢失。 */
export const switchGameDirectory = (path: string, name: string): Promise<void> =>
  invoke("switch_game_directory", { path, name });

/** 走完初次设定：定下游戏目录、收下选中的其它文件夹，并把配置落盘。 */
export const completeFirstRun = (gameDir: string, extras: NamedDirectory[]): Promise<void> =>
  invoke("complete_first_run", { gameDir, extras });
