// 自定义背景的取图地址与状态。
//
// 图片不走 IPC 传字节，而是由 Rust 侧注册的 aurora-bg 协议直接吐给 WebView——
// 一张 JPEG 走 invoke 要先 base64 膨胀三分之一再过 JSON，纯属浪费。
//
// 浏览器（mock 开发环境）里没有这个协议，用一张内联 SVG 顶上，让版式在浏览器里也看得出效果。

import { getAppearance, type AppearanceDto } from "./ipc";

/** 协议在 Windows WebView 里的形态。macOS/Linux 是 aurora-bg://localhost，但 Aurora 只出桌面 Windows。 */
const ORIGIN = "http://aurora-bg.localhost";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// 浏览器占位图：一道斜向渐变加几何块，足以看出「图铺在下面、纸片压在上面」的层次。
const MOCK_IMAGE =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900">
      <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="#2f4858"/><stop offset="0.55" stop-color="#5a7d7c"/>
        <stop offset="1" stop-color="#c4b7a6"/>
      </linearGradient></defs>
      <rect width="1600" height="900" fill="url(#g)"/>
      <circle cx="1180" cy="250" r="150" fill="#f3f2f0" opacity="0.16"/>
      <rect x="120" y="560" width="520" height="240" fill="#14161a" opacity="0.12"/>
    </svg>`,
  );

/**
 * 当前背景的地址。
 *
 * `stamp` 是缓存版本戳：协议侧给图片打了 immutable 缓存头（同一张图不该每次进主页都重读磁盘），
 * 所以换图后必须换 URL 才能让 WebView 放弃旧的那份。
 */
export function currentBackgroundUrl(stamp: number): string {
  return inTauri() ? `${ORIGIN}/current?v=${stamp}` : MOCK_IMAGE;
}

/** 图库里指定一张的地址，用于设置页的缩略图网格。 */
export function libraryBackgroundUrl(file: string): string {
  return inTauri() ? `${ORIGIN}/library/${encodeURIComponent(file)}` : MOCK_IMAGE;
}

/** 外观设置为空时的形态，用作首屏渲染的初值。 */
export const EMPTY_APPEARANCE: AppearanceDto = { background: null, tint: null, veil: 0 };

/** 纸色遮罩上限，与后端的 MAX_BACKGROUND_VEIL 对齐。 */
export const MAX_VEIL = 60;

/**
 * 弹系统文件框选一张图，返回磁盘绝对路径；用户取消返回 null。
 *
 * 必须走原生对话框：WebView 里的 `<input type="file">` 出于安全只给 File 对象，
 * 拿不到路径，而后端要按路径把图复制进图库。
 * 浏览器里没有这个插件，返回 null 让调用方当作取消——mock 的图库本来就是编的。
 */
export async function pickImageFile(): Promise<string | null> {
  if (!inTauri()) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "图片", extensions: ["jpg", "jpeg", "png", "webp"] }],
  });
  return typeof picked === "string" ? picked : null;
}

/** 当前环境能否弹出文件选择框。浏览器预览里不能，按钮据此禁用而不是点了没反应。 */
export function canPickFile(): boolean {
  return inTauri();
}

/**
 * 读一次外观设置，失败时退回纯纸面。
 *
 * 这里是全文件唯一一处吞异常的地方，理由是它的调用点在应用外壳的渲染路径上：
 * 配置读不出来该退化成「没有背景」，而不是让整个界面因为一张壁纸打不开就白屏。
 * 真正需要报错的操作（导入、切换、删除）都在设置页，那边照常把异常抛给 toast。
 */
export async function loadAppearance(): Promise<AppearanceDto> {
  try {
    return await getAppearance();
  } catch {
    return EMPTY_APPEARANCE;
  }
}
