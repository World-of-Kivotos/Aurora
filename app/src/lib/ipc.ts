// 前端 IPC 层：aurora-tauri 命令的类型化封装 + 进度事件订阅。
// 这里的 interface 与 aurora-tauri/src/lib.rs 里的 serde DTO 一一对应（字段名 snake_case）。
// 页面只调用本文件导出的函数，不直接 import @tauri-apps/api，保证调用点集中、可测。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---- 与后端 serde 枚举对应的字面量联合（均为 snake_case，源自各枚举的 rename_all）----
export type DownloadSourcePolicy = "auto" | "official_first" | "mirror_first";
export type IsolationPolicy =
  | "disabled"
  | "mod_loaders_only"
  | "non_release_only"
  | "mod_loaders_and_non_release"
  | "all";
export type AccountType = "microsoft" | "offline" | "authlib_injector";

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

// 进度事件负载：对应后端 CoreEventDto（serde tag = "kind"）。
export type CoreEvent =
  | { kind: "stage"; message: string }
  | { kind: "warning"; message: string }
  | { kind: "download"; total: number; finished: number; bytes: number; speed: number };

// 与后端 CORE_EVENT 常量保持一致；install/launch 页面复用同一事件名。
export const CORE_EVENT = "aurora://core-event";

// ---- 命令封装 ----
export const getConfig = (): Promise<ConfigDto> => invoke<ConfigDto>("get_config");

export const listInstalled = (): Promise<VersionScanDto> =>
  invoke<VersionScanDto>("list_installed");

export const currentAccount = (): Promise<AccountDto | null> =>
  invoke<AccountDto | null>("current_account");

export const createOfflineAccount = (name: string): Promise<AccountDto> =>
  invoke<AccountDto>("create_offline_account", { name });

// 订阅门面进度事件。后续 install/launch 页面照抄本范式：
//   const unlisten = await onCoreEvent((ev) => { ... 按 ev.kind 更新进度 ... });
//   try { await invoke("install_version", {...}); } finally { unlisten(); }
// 组件卸载或流程结束务必调用返回的 unlisten，避免监听器泄漏。
export const onCoreEvent = (handler: (event: CoreEvent) => void): Promise<UnlistenFn> =>
  listen<CoreEvent>(CORE_EVENT, (e) => handler(e.payload));
