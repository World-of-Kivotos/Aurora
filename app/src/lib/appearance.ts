// 自定义背景的取图地址与状态。
//
// 图片不走 IPC 传字节，而是由 Rust 侧注册的 aurora-bg 协议直接吐给 WebView——
// 一张 JPEG 走 invoke 要先 base64 膨胀三分之一再过 JSON，纯属浪费。
//
// 浏览器（mock 开发环境）里没有这个协议，用一张内联 SVG 顶上，让版式在浏览器里也看得出效果。

import { getAppearance, type AppearanceDto, type GlassMode, type PlateZone } from "./ipc";

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
  glass: "frost",
};

/**
 * 玻璃模式落到 DOM 上的唯一一处写入：`<html data-glass="liquid">`。
 *
 * 只有液态档写属性，frost 档把属性删掉而不是写成 "frost"。CSS 侧的契约是「缺省即 frost」，
 * 写一个显式的 frost 值等于给同一个状态造出两种形态（缺省与显式），
 * 那么以后谁写了 data-glass="frosted" 之类的错别字也会静默落在保守档上而没人发现。
 *
 * 取 documentElement 而不是某个容器：CSS 选择器是 :root[data-glass="liquid"]，
 * 而且弹窗与 Toast 都 Portal 到 body 之外的层，挂在应用容器上它们就吃不到。
 *
 * 形参只要 dataset 而不是整个 HTMLElement：这条规则值得用例守住，而测试跑在 node 环境里
 * 没有真 DOM。收窄到实际用到的那一个成员，真元素照样满足，用例也不必为它引一整个 jsdom。
 */
export function applyGlassMode(mode: GlassMode, root: { dataset: DOMStringMap }): void {
  if (mode === "liquid") {
    root.dataset.glass = "liquid";
  } else {
    delete root.dataset.glass;
  }
}

/*
 * 以下这套「裸字可读性判定」在玻璃换皮时被整体删掉过一次, 此处按原样恢复。
 * 删它的理由是换皮后右下角那撮信息恒定坐在一块材质上, 判定看似没了用处;
 * 但恒定上材质正是被否掉的那个做法 —— 一块纸压在照片上, 无论做得多透,
 * 都还是在图里挖了一块出来。恢复裸字就必须连它的判定一起恢复,
 * 否则字色写死成墨色, 换一张深色壁纸就直接读不出来。
 *
 * 数据链路一直是通的: Rust 侧 background.rs 的 plate_zone_of 始终在采样右下角的 p10/p90,
 * AppearanceDto.plate 也始终在传, 被删的只是前端这一段消费逻辑。
 */

/**
 * 主页右下角那撮信息该怎么摆。
 *
 * - `ink`：字直接压在图上，满墨。图够亮时走这档，什么都不加。
 * - `paperOn`：字直接压在图上，纸色，底下垫一层柔和压暗。图偏暗时走这档。
 * - `plate`：这块图明暗跨度太大，两种字色都撑不住，退回磨砂纸片。
 */
export type PlateMode = "ink" | "paperOn" | "plate";

/**
 * 压暗层的亮度系数。与 app.css 里 .plate-scrim 的 brightness() 必须是同一个数。
 *
 * 是等比压暗照片本身，不是铺一层墨色——铺墨会把彩色照片洗成灰，
 * 而等比缩放保留色彩关系，同样观感代价下给的对比度还更高。
 */
export const SCRIM_BRIGHTNESS = 0.5;

/**
 * 裸字要达到的对比度，比 AA 的 4.5 高一档。
 *
 * 判据用的是 p10/p90，按定义两头各还有一成像素比它更差，而那一成完全可能正压在字底下。
 * 压着 4.5 判等于把那条尾巴当不存在。多要一档余量，宁可多退回纸片，
 * 也不能让字在某张图上恰好糊掉。
 */
const NAKED_TARGET = 5.5;

// 三个关键色的等效灰阶（sRGB 0..255）与相对亮度。都是近中性色，按灰阶做合成估算误差可忽略。
const PAPER_SRGB = 242;
const L_INK = 0.007971;
const L_PAPER_ON = 0.905855;

/** sRGB 灰阶转相对亮度。 */
function lumaOf(srgb: number): number {
  const c = srgb / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** 相对亮度还原成等效 sRGB 灰阶——上面那条传递函数的逆。 */
function srgbOf(luma: number): number {
  const c = luma <= 0.0031308 ? luma * 12.92 : 1.055 * luma ** (1 / 2.4) - 0.055;
  return c * 255;
}

/** 把 `over` 以 alpha 压在 `base` 上（都按 sRGB 灰阶算）。 */
function composite(base: number, over: number, alpha: number): number {
  return over * alpha + base * (1 - alpha);
}

/**
 * 取样值经过柔化与压暗两层之后，字实际落在的那层底色亮度。
 *
 * 顺序必须和 PageBackground 的图层顺序一致：图 -> 纸色柔化 -> 压暗。
 * 柔化会把底色提亮，直接拿原图的取样值判会高估深色图的可用性——
 * 玩家把柔化一拉，浅色字就该失效了，判定必须跟着变。
 */
function effectiveLuma(sampled: number, veil: number, dim: number): number {
  const afterImage = srgbOf(sampled / 255);
  const afterVeil = composite(afterImage, PAPER_SRGB, veil / 100);
  // CSS 的 brightness() 是对 sRGB 各通道等比缩放，这里照同一口径算。
  return lumaOf(afterVeil * dim);
}

export function plateMode(plate: PlateZone | null, veil: number): PlateMode {
  // 没量过的图（本功能上线前导入的）一律上纸片：宁可多一块纸，也不拿没量过的图赌可读性。
  // 玩家在设置页重新选一次这张图，后端就会补上取样，自动切到裸字。
  if (plate === null) return "plate";

  // 比的都是最不利的那一端而不是均值：满墨字怕最暗处，纸色字怕最亮处。
  // 明暗跨度大的角落两头都过不了，自然落到纸片，不需要另设「多花算花」的阈值。
  //
  // 满墨字这一档不压暗——压暗只会让深色字更难读，所以系数给 1（原样）。
  const darkEnd = effectiveLuma(plate.p10, veil, 1);
  if ((darkEnd + 0.05) / (L_INK + 0.05) >= NAKED_TARGET) return "ink";

  // 纸色字这一档底下垫压暗层，把最亮那一成拉进达标区。
  const brightEnd = effectiveLuma(plate.p90, veil, SCRIM_BRIGHTNESS);
  if ((L_PAPER_ON + 0.05) / (brightEnd + 0.05) >= NAKED_TARGET) return "paperOn";

  return "plate";
}

/**
 * 背景图当前是否正在显示——外壳与浮层据此决定用不用玻璃材质。
 *
 * 判据只剩「装没装壁纸」这一条。上一版还要求 pathname === "/"，因为那时图只铺主页；
 * 全站铺图之后那半条判据成了假命题：内页照样压在图上，还按它判会让内页的标题栏与侧栏
 * 退回不透明纸色，在图上切出两块硬边——正是磨砂当初要消掉的那种缝。
 *
 * 判定仍然只在这里定义一次。标题栏与侧栏分处两棵子树，两边各写一遍是能跑，
 * 但「什么时候算压在图上」这条规则一改就会漂移，届时会出现标题栏已经上玻璃、
 * 侧栏却仍按无图渲染这种错配。
 */
export function photoShows(background: string | null): boolean {
  return background !== null;
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
