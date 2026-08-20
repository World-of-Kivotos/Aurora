// 背景的取图地址、解析与状态。
//
// 图片不走 IPC 传字节，而是由 Rust 侧注册的 aurora-bg 协议直接吐给 WebView——
// 一张 JPEG 走 invoke 要先 base64 膨胀三分之一再过 JSON，纯属浪费。
//
// 浏览器（mock 开发环境）里没有这个协议，用一张内联 SVG 顶上，让版式在浏览器里也看得出效果。
//
// 图有两个来源：玩家自己导入的壁纸（图库，一份，与游戏无关），以及嵌在 exe 里的内置背景
// （一台游戏一张）。谁压谁由 resolveBackground 一处判定。

import {
  getAppearance,
  listBuiltinBackgrounds,
  type AppearanceDto,
  type BuiltinBackground,
  type GlassMode,
  type PlateZone,
} from "./ipc";

/** 协议在 Windows WebView 里的形态。macOS/Linux 是 aurora-bg://localhost，但 Aurora 只出桌面 Windows。 */
const ORIGIN = "http://aurora-bg.localhost";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * 浏览器占位图：一道斜向渐变加几何块，足以看出「图铺在下面、纸片压在上面」的层次。
 *
 * 三种色调必须产出三串不同的 data URI。这不是为了好看——PageBackground 拿地址当图层的 key，
 * 两台游戏的占位图若是同一串，浏览器里切游戏时图层根本不重挂载，
 * 「换游戏换背景」与「来回切会不会把图弄丢」这两件事就都验不了，
 * 而它们恰恰只在浏览器里驱动得起来（真 Tauri 窗口没法挂 CDP）。
 */
function mockImage(from: string, mid: string, to: string): string {
  return (
    "data:image/svg+xml;utf8," +
    encodeURIComponent(
      `<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900">
      <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
        <stop offset="0" stop-color="${from}"/><stop offset="0.55" stop-color="${mid}"/>
        <stop offset="1" stop-color="${to}"/>
      </linearGradient></defs>
      <rect width="1600" height="900" fill="url(#g)"/>
      <circle cx="1180" cy="250" r="150" fill="#f3f2f0" opacity="0.16"/>
      <rect x="120" y="560" width="520" height="240" fill="#14161a" opacity="0.12"/>
    </svg>`,
    )
  );
}

/** 玩家自选壁纸的占位图。 */
const MOCK_IMAGE = mockImage("#2f4858", "#5a7d7c", "#c4b7a6");

/**
 * 内置图的占位图，按 id 取色。
 *
 * 色调刻意贴近后端实测的 tint（master #8c8192 偏紫灰、arena #474471 偏靛蓝），
 * 这样浏览器预览里看到的明暗关系与真机同向，调版面时不会被一张不相干的占位图带偏。
 */
const MOCK_BUILTIN: Record<string, string> = {
  master: mockImage("#3b3340", "#8c8192", "#c9bfcb"),
  arena: mockImage("#1e1c33", "#474471", "#9a97c4"),
};

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

/**
 * 内置背景的地址。
 *
 * 不带 currentBackgroundUrl 那样的缓存版本戳：内置图是编译进二进制的常量，同一个 id 的字节
 * 永远是同一份，immutable 缓存正好该命中。给它加戳等于每次进程重启都让 WebView 重解一遍图。
 */
export function builtinBackgroundUrl(id: string): string {
  if (inTauri()) return `${ORIGIN}/builtin/${encodeURIComponent(id)}`;
  // 认不出的 id 回落到自选壁纸那张占位图：浏览器预览没有「这个 id 不存在」这种错要报，
  // 有张图能铺就行。真机那侧的未知 id 由协议查白名单后回 404，两边各管各的一段。
  return MOCK_BUILTIN[id] ?? MOCK_IMAGE;
}

/**
 * 游戏标识。取值直接复用后端内置背景登记表的 id（crates/aurora-core/src/background.rs）：
 * 一台游戏对应一张内置图，两套 id 天然一一对应，再造一层「游戏 -> 背景」对照表只会多出
 * 一处会漂移的映射。
 */
export const MAIN_GAME_ID = "master";
export const ARENA_GAME_ID = "arena";

/**
 * 这条路由是不是某台游戏的启动屏；不是就返回 null。
 *
 * 一个函数同时回答两件事，是因为它们本来就是同一个判断：启动屏是唯一「整屏属于某台游戏」的
 * 页面，背景挑哪一张、右下角那撮字要不要压暗，问的都是「现在站在哪台游戏的门口」。
 * 拆成两个函数就会出现两张路由表，加第三台游戏时必然漏改其中一张。
 *
 * 内页（账户/下载/卷宗/设置）返回 null：它们不属于任何一台游戏，背景由调用方退回主服那张。
 */
export function gameScreenOf(pathname: string): string | null {
  if (pathname === "/") return MAIN_GAME_ID;
  if (pathname === "/arena") return ARENA_GAME_ID;
  return null;
}

/**
 * 本次实际要显示的那张背景图。
 *
 * 用判别联合而不是「file 与 id 两个可空字段」：后者能表达出「两个都有」「两个都没有」这两种
 * 不存在的状态，取图那一侧就得再判一次优先级——而优先级只该由 resolveBackground 说了算。
 */
export type ResolvedBackground =
  | {
      kind: "library";
      file: string;
      /** 图库里的老图可能没量过，故可空；内置图一定量过。 */
      tint: string | null;
      plate: PlateZone | null;
    }
  | { kind: "builtin"; id: string; tint: string; plate: PlateZone };

/**
 * 定下这一帧铺哪张图，以及跟着它走的取样值。
 *
 * 自选壁纸优先且不分游戏：它是「我的壁纸」，玩家亲手挑的东西压过我们预设的东西，
 * 而且切游戏时把人家选的图换掉，会读成启动器把设置弄丢了。
 *
 * 没自选就按当前游戏取内置那张。认不出的游戏 id 退回主服那张而不是返回 null：
 * 这种 id 只可能来自「加了新路由却忘了登记」，那时候少一张图是整屏空白，多一张是背景不对，
 * 后者显然轻。
 *
 * 返回 null 只有一种情形——没自选、内置表也还没到手（首屏那一两帧，或者拉取失败）。
 * 那一帧照旧走纯纸底，与本功能上线前的观感一致。
 *
 * 纯函数，取的是外部传进来的表而不是自己去 IPC 拉：外壳每次渲染都要问一遍，
 * 拉取归 Provider 管一次，判定归这里管一次。
 */
export function resolveBackground(
  appearance: AppearanceDto,
  builtins: readonly BuiltinBackground[],
  gameId: string,
): ResolvedBackground | null {
  if (appearance.background !== null) {
    return {
      kind: "library",
      file: appearance.background,
      tint: appearance.tint,
      plate: appearance.plate,
    };
  }
  const builtin =
    builtins.find((b) => b.id === gameId) ?? builtins.find((b) => b.id === MAIN_GAME_ID);
  if (!builtin) return null;
  return { kind: "builtin", id: builtin.id, tint: builtin.tint, plate: builtin.plate };
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
 * 判据前后换过两次，每次都是因为「什么时候有图」这件事本身变了：最早还要求 pathname === "/"，
 * 那时图只铺主页；全站铺图之后收窄成「装没装壁纸」；内置默认背景上线之后，有没有图不再取决于
 * 玩家去没去装一张，所以现在问的是「resolveBackground 到底给没给出一张图」。
 *
 * 它没有退化成恒真：没自选壁纸、内置表又还没到手（首屏那一两帧，或者那次拉取失败了）时仍为假。
 * 这一帧必须为假才对——玻璃在没有图的时候采的是一张纯色，模糊出来还是同一个颜色，
 * 纯属白烧一次全表面合成。
 *
 * 判定仍然只在这里定义一次。标题栏与侧栏分处两棵子树，两边各写一遍是能跑，
 * 但「什么时候算压在图上」这条规则一改就会漂移，届时会出现标题栏已经上玻璃、
 * 侧栏却仍按无图渲染这种错配。
 */
export function photoShows(background: ResolvedBackground | null): boolean {
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
 * 读一次外观设置，失败时退回空外观（没有自选壁纸，也就是照内置默认走）。
 *
 * 这里是全文件唯二吞异常的地方（另一处是下面的 loadBuiltinBackgrounds），理由是它的调用点
 * 在应用外壳的渲染路径上：配置读不出来该退化成「没有自选背景」，而不是让整个界面因为一张
 * 壁纸打不开就白屏。真正需要报错的操作（导入、切换、删除）都在设置页，那边照常把异常抛给 toast。
 */
export async function loadAppearance(): Promise<AppearanceDto> {
  try {
    return await getAppearance();
  } catch {
    return EMPTY_APPEARANCE;
  }
}

/**
 * 读一次内置背景登记表，失败时退回空表。
 *
 * 与 loadAppearance 同一条理由，且这一条更硬：内置表是默认背景的唯一来源，它在外壳挂载时拉，
 * 拉不到就让整个启动器打不开，等于把「背景图」这件装饰性的事升级成启动阻塞项。
 * 空表由 resolveBackground 兜住（返回 null），界面退回纯纸底，其余功能一概不受影响。
 *
 * 但不静默：表的内容编译进二进制、命令无参数，正常情况下没有失败的道理，真失败了必是
 * 后端出了别的问题，这一笔留给控制台对时间线。
 */
export async function loadBuiltinBackgrounds(): Promise<BuiltinBackground[]> {
  try {
    return await listBuiltinBackgrounds();
  } catch (e) {
    console.error("内置背景表读取失败", e);
    return [];
  }
}
