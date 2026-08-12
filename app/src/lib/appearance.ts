// 自定义背景的取图地址与状态。
//
// 图片不走 IPC 传字节，而是由 Rust 侧注册的 aurora-bg 协议直接吐给 WebView——
// 一张 JPEG 走 invoke 要先 base64 膨胀三分之一再过 JSON，纯属浪费。
//
// 浏览器（mock 开发环境）里没有这个协议，用一张内联 SVG 顶上，让版式在浏览器里也看得出效果。

import { getAppearance, type AppearanceDto, type PlateZone } from "./ipc";

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
export const EMPTY_APPEARANCE: AppearanceDto = {
  background: null,
  tint: null,
  plate: null,
  veil: 0,
};

/**
 * 主页右下角那撮信息该怎么摆。
 *
 * - `ink`：字直接压在图上，满墨，不要纸片。
 * - `plate`：这块图撑不住裸字，退回磨砂纸片。
 */
export type PlateMode = "ink" | "plate";

/*
 * 裸字的准入线。由 WCAG 反解得出，不是拍脑袋定的：
 *
 *   墨色字 #14161a 的相对亮度是 0.00797，要 (L + 0.05) / (0.00797 + 0.05) >= 4.5
 *   解得 L >= 0.2109，映射到 0..255 就是 54。
 *
 * 但实际取 69，因为判据用的是 p10——按定义还有一成像素比它更暗，而那一成完全可能
 * 正压在字底下。54 在 p10 处刚好压线 4.52:1，没有余量；69 对应 5.53:1，
 * 把那条暗尾也兜进去。宁可多退回纸片，不能让字在某张图上恰好糊掉。
 */
const INK_NEEDS_AT_LEAST = 69;

export function plateMode(plate: PlateZone | null): PlateMode {
  // 没量过的图（本功能上线前导入的）一律上纸片：宁可多一块纸，也不拿没量过的图赌可读性。
  // 玩家在设置页重新选一次这张图，后端就会补上取样，自动切到裸字。
  if (plate === null) return "plate";
  // 比的是偏暗那一端而不是均值：选了墨色字，怕的就是区域里最暗的部分。
  // 明暗跨度大的角落 p10 一定低，自然落到纸片，不需要另设「多花算花」的阈值。
  //
  // 深色图本可以反过来用纸色字，这里没有做：那要连 LaunchControl 的 Start 字样与
  // 朱红强调色一起反相，是另一套配色决策。没定之前深色图一律走纸片这条稳妥路径。
  return plate.p10 >= INK_NEEDS_AT_LEAST ? "ink" : "plate";
}

/**
 * 背景图当前是否正在显示——外壳与浮层据此决定用不用磨砂纸。
 *
 * 判定只在这里定义一次。外壳（AppShell）与 Toast 分处两棵子树、各自拿得到 pathname 与 appearance，
 * 两边各写一遍是能跑，但「哪些页面铺图」这条规则一改就会漂移，
 * 届时会出现外壳已经磨砂、右下角的提示却仍按无图渲染这种错配。
 */
export function photoShowsOn(pathname: string, background: string | null): boolean {
  return pathname === "/" && background !== null;
}

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
