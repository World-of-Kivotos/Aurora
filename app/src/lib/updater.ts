// 启动器自更新的薄封装。
//
// 更新插件只在 Tauri 里存在，浏览器（mock 开发环境）里 import 它会直接抛。这里把入口收在一处：
// 不在 Tauri 就返回「不可用」，让界面照常渲染而不是白屏——前端开发大部分时间都在浏览器里。
//
// 签名校验由插件按 tauri.conf.json 内置的公钥完成，前端拿不到也不该拿到私钥。

/** 一次检查的结果。 */
export type UpdateStatus =
  | { kind: "unsupported" }
  | { kind: "up-to-date" }
  | { kind: "available"; version: string; notes: string | null; date: string | null }
  | { kind: "error"; message: string };

/** 浏览器里没有 Tauri 注入的全局，据此判断更新能力是否存在。 */
function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// 插件模块只在真正需要时动态导入：静态 import 会让浏览器构建也去解析它。
async function updaterModule() {
  return await import("@tauri-apps/plugin-updater");
}

/** 已取到的待安装更新。下载安装要用它，所以检查与安装之间必须保住这个句柄。 */
let pending: Awaited<ReturnType<Awaited<ReturnType<typeof updaterModule>>["check"]>> = null;

/** 查有没有新版本。不在 Tauri 里返回 unsupported，网络失败返回 error 而不是抛。 */
export async function checkUpdate(): Promise<UpdateStatus> {
  if (!inTauri()) return { kind: "unsupported" };
  try {
    const { check } = await updaterModule();
    const update = await check();
    pending = update;
    if (!update) return { kind: "up-to-date" };
    return {
      kind: "available",
      version: update.version,
      notes: update.body ?? null,
      date: update.date ?? null,
    };
  } catch (e) {
    // 端点还没有 latest.json（尚未发过版）时也走这里，属于预期内的失败，不该冒泡成崩溃。
    return { kind: "error", message: String(e) };
  }
}

/**
 * 下载并安装刚才查到的更新，完成后重启应用。
 *
 * `onProgress` 收到的是已下载字节与总字节；总字节在服务端没给 Content-Length 时为 null。
 */
export async function installUpdate(
  onProgress?: (downloaded: number, total: number | null) => void,
): Promise<void> {
  if (!pending) throw new Error("没有待安装的更新，请先检查更新");
  let downloaded = 0;
  let total: number | null = null;

  await pending.downloadAndInstall((event) => {
    if (event.event === "Started") {
      total = event.data.contentLength ?? null;
      downloaded = 0;
    } else if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
    }
    onProgress?.(downloaded, total);
  });

  // 安装包已就位，重启进程让新版本生效。
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
