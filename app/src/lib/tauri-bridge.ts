// IPC 桥：在 Tauri 里走真命令；在纯浏览器（pnpm dev 看 UI）里走 mock 假数据。
// 侦测 Tauri v2 注入的全局 __TAURI_INTERNALS__；缺失即视为浏览器开发环境。

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { mockInvoke, mockListen } from "./ipc-mock";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const invoke = inTauri ? tauriInvoke : mockInvoke;
export const listen = inTauri ? tauriListen : mockListen;
export type { UnlistenFn };
