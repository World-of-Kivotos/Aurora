/*
 * 液态玻璃(真折射)—— vendored 单文件实现, 由本仓库自行维护, 不引入 npm 包。
 *
 * 为什么自己养一份:
 *   npm 上那批现成的包(liquid-glass-react / @liquidglass/react / @developer-hub/liquid-glass)
 *   全部停更在 2025-06, 至今十四个月无人维护; @specy 那个为了做折射把 Three.js 一并打进来,
 *   体积 6.8MB; @developer-hub 连 license 字段都没有。2026 年新出的两个(samasante、rizroze)
 *   本身就是「拷一个文件进项目」的形态而不是库。一个只依赖 SVG 滤镜、不到四百行的效果,
 *   不值得为它背一条无人维护的运行时依赖。
 *
 * 思路来源: liquid-glass-react(MIT) 以及公开的 feImage + feDisplacementMap 位移图做法。
 * 本文件的代码与注释为本仓库自行编写, 未复制上述实现的源码。
 *
 * -----------------------------------------------------------------------------
 * 原理(读代码前先读这段, 否则下面的通道与符号会看不懂)
 * -----------------------------------------------------------------------------
 * feDisplacementMap 的定义: 结果像素 P'(x, y) 取自源图的
 *     P( x + scale * (XC(x,y) - 0.5),  y + scale * (YC(x,y) - 0.5) )
 * 其中 XC / YC 是「位移图」在该点上被选中通道的归一化值(0..1)。所以 128 = 不位移,
 * 0 = 往负方向推半个 scale, 255 = 往正方向推半个 scale。
 *
 * 于是位移图只需要是这样一张图: 中间一大片是「不位移」, 只有四条边框里的一圈从中性值
 * 渐变到两端。本实现用两条线性渐变叠出来 —— R 通道承载 x 位移, B 通道承载 y 位移,
 * 两层用 screen 混合(两层各自只有一个通道非零, screen 等价于按通道相加, 互不污染)。
 *
 * 这样得到的位移场是「可分离」的: 直边上只有单轴位移, 圆角处两轴同时非零, 自然合成
 * 一个斜向推力 —— 正是一块圆角透镜边缘该有的方向。真正的圆角由调用方的 border-radius
 * 裁掉, 位移图本身不必知道半径, 所以这里没有 radius 参数(也就不会去碰圆角令牌)。
 *
 * 色散(chromatic aberration): 同一张位移图跑三遍, 三个 scale 略有差异, 再各自只保留
 * R/G/B 中的一个通道 screen 回去。蓝光偏折大于红光, 所以 b 的 scale 最大。
 */

/** 折射档: 真位移滤镜; 毛玻璃档: 只有 blur + saturate。 */
export type GlassMode = "refract" | "frost";

/** 调用方能要求的档位。没有 "refract" —— 能不能折射由环境说了算, 不由调用方一厢情愿。 */
export type GlassModeRequest = "auto" | "frost";

export interface LiquidGlassParams {
  /** 位移强度(px)。边缘最外沿的采样偏移量约为 strength / 2。 */
  strength: number;
  /** 折射边厚度(px)。中间那片平面之外的边框宽度, 决定「这块玻璃有多厚」。 */
  bevel: number;
  /** 色散比例(0..1)。0 表示关掉色散, 滤镜退化成单次位移(省两遍采样)。 */
  dispersion: number;
  /** 背景模糊(px), 语义与 CSS blur() 一致(stdDeviation 就是这个数)。 */
  blur: number;
  /** 饱和度(%), 语义与 CSS saturate() 一致。 */
  saturation: number;
}

/*
 * 默认值的来由:
 *   strength 26 / bevel 22 —— 边厚略小于位移强度时, 边缘的压缩读起来像一圈厚玻璃而不是
 *     一道描边; 两者接近 1:1 是这套参数里最耐看的区间。
 *   dispersion 0.08 —— 再高就从「玻璃」滑向「彩虹边」, 在纸墨版面里会显得廉价。
 *   blur 4 —— 折射档的清晰度本身就是效果的一部分, 大模糊会把折射糊掉; 毛玻璃档另有下限。
 *   saturation 160 —— 与 app.css 各档材质补饱和同一个理由: 只 blur 会把透上来的颜色洗成灰。
 */
export const DEFAULT_PARAMS: LiquidGlassParams = {
  strength: 26,
  bevel: 22,
  dispersion: 0.08,
  blur: 4,
  saturation: 160,
};

/**
 * 折射档的宽度上限(px)。
 *
 * 实测 >800px 宽的元素会把合成器打到掉帧: 位移滤镜的成本随被采样面积走, 一块跨屏的面板
 * 每帧都要重采一遍整个背景。超过这个宽度一律降级成毛玻璃 —— 掉帧比没有折射难看得多。
 */
export const MAX_REFRACTION_WIDTH = 800;

/**
 * 毛玻璃档的模糊下限(px)。
 *
 * 折射档的玻璃感主要来自边缘的位移, 模糊只是配角; 一旦降级, 位移没了, 就只剩模糊一个人
 * 扛整块玻璃的观感, 必须给够。12 是 app.css 里 .surface-panel-strong 那一档的值,
 * 取同一个数是为了让降级后的元素与全站毛玻璃看起来仍是同一种材料。
 */
export const FROST_MIN_BLUR = 12;

function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(Math.max(value, min), max);
}

/** 万分位取整。所有要落进 SVG 属性的数都过这一道, 免得浮点尾巴污染生成的字符串。 */
function round4(value: number): number {
  return Math.round(value * 1e4) / 1e4;
}

function fmt(value: number): string {
  return String(round4(value));
}

export interface BevelStops {
  /** 起点一侧「渐变结束、进入中性区」的位置(0..0.5)。 */
  near: number;
  /** 终点一侧「离开中性区、开始渐变」的位置(0.5..1)。 */
  far: number;
}

/**
 * 把边厚换算成渐变停靠点。
 *
 * 上限 0.5 不是保护性写法而是几何事实: 边厚超过半边长时两侧斜面已在中线相遇, 中性区
 * 宽度为零, 这块玻璃从「有平面的透镜」变成「整块都是斜面的棱镜」。尺寸为 0 或非法时
 * 同样退到 0.5/0.5, 保证 stop 的 offset 单调不倒序(倒序的 stop 在 Chromium 上会被
 * 静默夹紧成一条硬边, 表现为玻璃边缘出现一圈锯齿, 极难倒查)。
 */
export function bevelStops(size: number, bevel: number): BevelStops {
  if (!(size > 0)) return { near: 0.5, far: 0.5 };
  const ratio = clamp(clamp(bevel, 0, Number.MAX_SAFE_INTEGER) / size, 0, 0.5);
  return { near: round4(ratio), far: round4(1 - ratio) };
}

export interface LensGeometry {
  width: number;
  height: number;
  bevel: number;
}

/**
 * 生成位移图的 SVG 源码。
 *
 * 通道约定(与滤镜里的 xChannelSelector / yChannelSelector 必须一致):
 *   R -> x 位移, B -> y 位移, G 恒为 0(空着, 留给以后想加第三种形变时用)。
 * 用 B 而不是 G 承载 y, 是跟公开实现的惯例保持一致, 方便对着别人的 map 图排查问题。
 */
export function buildDisplacementMapSvg(geometry: LensGeometry): string {
  const w = Math.max(0, Math.round(geometry.width));
  const h = Math.max(0, Math.round(geometry.height));
  const x = bevelStops(w, geometry.bevel);
  const y = bevelStops(h, geometry.bevel);

  return [
    `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">`,
    "<defs>",
    '<linearGradient id="x" x1="0" y1="0" x2="1" y2="0">',
    '<stop offset="0" stop-color="rgb(0,0,0)"/>',
    `<stop offset="${fmt(x.near)}" stop-color="rgb(128,0,0)"/>`,
    `<stop offset="${fmt(x.far)}" stop-color="rgb(128,0,0)"/>`,
    '<stop offset="1" stop-color="rgb(255,0,0)"/>',
    "</linearGradient>",
    '<linearGradient id="y" x1="0" y1="0" x2="0" y2="1">',
    '<stop offset="0" stop-color="rgb(0,0,0)"/>',
    `<stop offset="${fmt(y.near)}" stop-color="rgb(0,0,128)"/>`,
    `<stop offset="${fmt(y.far)}" stop-color="rgb(0,0,128)"/>`,
    '<stop offset="1" stop-color="rgb(0,0,255)"/>',
    "</linearGradient>",
    "</defs>",
    // 黑底打底: screen 的中性元是黑, 有了它两层渐变叠出来的结果才等于按通道相加。
    `<rect width="${w}" height="${h}" fill="rgb(0,0,0)"/>`,
    `<rect width="${w}" height="${h}" fill="url(#x)"/>`,
    `<rect width="${w}" height="${h}" fill="url(#y)" style="mix-blend-mode:screen"/>`,
    "</svg>",
  ].join("");
}

/**
 * 位移图的 data URI。
 *
 * 走 utf-8 百分号编码而不是 base64: 体积更小, 出问题时能直接在 devtools 里读出来。
 * encodeURIComponent 会把 url(#x) 里那个 `#` 转义成 %23 —— 不转义的话它会被当成 URL
 * 片段分隔符, 整张图在第一个 `#` 处被截断, 这是这类实现最常见的一个哑火点。
 */
export function buildDisplacementMapDataUri(geometry: LensGeometry): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(buildDisplacementMapSvg(geometry))}`;
}

export interface DispersionScales {
  r: number;
  g: number;
  b: number;
}

/**
 * 三个通道各自的位移强度。蓝端偏折最大, 与真实色散方向一致。
 * 结果直接进 SVG 的 scale 属性, 所以在这里就取整: 30 * 0.9 在 IEEE 754 下是
 * 27.000000000000004, 不取整会把这串尾巴原样写进 DOM。
 */
export function dispersionScales(strength: number, dispersion: number): DispersionScales {
  const d = clamp(dispersion, 0, 1);
  return { r: round4(strength * (1 - d)), g: round4(strength), b: round4(strength * (1 + d)) };
}

/** 要不要跑三遍位移。强度为 0(整条滤镜空转)或色散为 0 时只跑一遍, 省两次全区域采样。 */
export function needsDispersion(strength: number, dispersion: number): boolean {
  return strength !== 0 && clamp(dispersion, 0, 1) > 0;
}

/** feColorMatrix 的「只留一个通道、alpha 原样」矩阵。三份 screen 回去正好还原成一张彩图。 */
export const KEEP_CHANNEL: Readonly<Record<keyof DispersionScales, string>> = {
  r: "1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0",
  g: "0 0 0 0 0 0 1 0 0 0 0 0 0 0 0 0 0 0 1 0",
  b: "0 0 0 0 0 0 0 0 0 0 0 0 1 0 0 0 0 0 1 0",
};

/** feColorMatrix type="saturate" 收的是倍率, CSS 收的是百分比, 在这里换算并夹紧。 */
export function saturationRatio(saturation: number): number {
  return clamp(saturation, 0, 400) / 100;
}

export interface GlassEnvironment {
  /** 通常就是 CSS.supports。取不到时留空, 判定一律走保守分支。 */
  cssSupports?: (property: string, value: string) => boolean;
  /** navigator.userAgentData 是否存在 —— 见 detectUrlFilterSupport 的注释。 */
  hasUserAgentData?: boolean;
}

/**
 * 能不能用 backdrop-filter: url(#id)。
 *
 * 两道判据缺一不可:
 *   1. CSS.supports 认这条声明 —— 挡住把 backdrop-filter 整个当不认识的浏览器。
 *   2. 运行在 Chromium 上 —— 挡的是「解析得过、渲染不出来」。backdrop-filter 的 url()
 *      引用只有 Chromium 真的实现了, 而 Firefox 用同一套 filter 语法做解析, CSS.supports
 *      会点头、画的时候却什么都没有: 那是比不做更糟的失败模式, 元素会连基础模糊一起丢掉。
 *      用 navigator.userAgentData 是否存在来判 Chromium, 而不是抠 UA 字符串: 这个 API
 *      至今只有 Chromium 系实现, 判据稳定且不必用正则去猜版本号。万一将来它被移除, 判定
 *      会倒向毛玻璃 —— 降级方向是安全的那一侧。
 *
 * Aurora 只出 Windows 包, 运行时必然是 WebView2 = Chromium, 所以这条降级路径实际只在
 * 开发期(vite dev 跑在别的浏览器里)生效。它存在是为了让开发期看到毛玻璃而不是一块空洞。
 */
export function detectUrlFilterSupport(env: GlassEnvironment): boolean {
  if (typeof env.cssSupports !== "function") return false;
  if (env.hasUserAgentData !== true) return false;
  return env.cssSupports("backdrop-filter", "url(#aurora-lens-probe)");
}

/** 从全局读一次环境。非浏览器环境(单测的 node)读到的是空对象, 于是判定为不支持。 */
export function readGlassEnvironment(): GlassEnvironment {
  const cssApi = typeof globalThis.CSS === "undefined" ? undefined : globalThis.CSS;
  return {
    cssSupports:
      cssApi && typeof cssApi.supports === "function" ? cssApi.supports.bind(cssApi) : undefined,
    hasUserAgentData: typeof navigator !== "undefined" && "userAgentData" in navigator,
  };
}

let cachedSupport: boolean | null = null;

/**
 * 探测结果只算一次并缓存: 它在一次运行里不会变, 而每个玻璃实例每次渲染都要问一遍。
 * 缓存放模块级而不是 React 层, 是为了让同一页面上几十个实例共用同一次探测。
 */
export function probeUrlFilterSupport(): boolean {
  if (cachedSupport === null) cachedSupport = detectUrlFilterSupport(readGlassEnvironment());
  return cachedSupport;
}

export interface GlassModeInput {
  requested: GlassModeRequest;
  supportsUrlFilter: boolean;
  /** 元素实测宽度; 尚未量到时为 null。 */
  width: number | null;
  /** 元素实测高度; 尚未量到时为 null。 */
  height: number | null;
  maxRefractionWidth: number;
}

/**
 * 定档。四种降级理由按代价从小到大排, 任一命中即退回毛玻璃:
 *   1. 调用方明确要毛玻璃(全局 frost 模式 / 无障碍偏好);
 *   2. 环境不支持 url() 滤镜;
 *   3. 尺寸还没量到, 或量到的是 0 —— 位移图没有尺寸就无从生成;
 *   4. 超过折射档的宽度上限。
 *
 * 只卡宽度不卡高度: 掉帧的实测口径就是宽度 >800px, 高度这一侧没有实测数据, 不编。
 * 需要更严的调用方自己把 maxRefractionWidth 调小。
 */
export function resolveGlassMode(input: GlassModeInput): GlassMode {
  if (input.requested === "frost") return "frost";
  if (!input.supportsUrlFilter) return "frost";
  if (input.width === null || input.height === null) return "frost";
  if (!(input.width > 0) || !(input.height > 0)) return "frost";
  if (!(input.width <= input.maxRefractionWidth)) return "frost";
  return "refract";
}

/**
 * 滤镜 id 消毒。
 *
 * React 各版本 useId 的形态一路在变(18 是 `:r0:`, 19.0 是 `«r0»`, 19.2 是 `_R_1_`),
 * 其中冒号与书名号都不是合法的 XML Name 首字符。这里只留 [A-Za-z0-9_-] 并加固定前缀,
 * 换 React 版本时不必回来改这里。大小写必须原样保留 —— React 用大小写区分服务端与客户端
 * 生成的 id, 一起小写化会把两个不同实例撞成同一个 id, 那正是多实例互相串滤镜的成因。
 */
export function sanitizeFilterId(rawId: string): string {
  const cleaned = rawId.replace(/[^A-Za-z0-9_-]/g, "");
  // 字符被剔光只可能出现在 useId 换成纯符号的假想情形; 给个固定兜底, 不静默产出空 id。
  return `aurora-lens-${cleaned === "" ? "anon" : cleaned}`;
}

/**
 * 元素上那条 backdrop-filter 的值。
 *
 * 折射档只留一个 url(): blur 与 saturate 都挪进了 SVG 滤镜内部。这么排不是洁癖 ——
 * 「url() 与其它滤镜函数混在同一条 backdrop-filter 里」在 Chromium 上是灰色地带,
 * 而滤镜内部的 feGaussianBlur / feColorMatrix 是 SVG 的老地基, 稳。
 * 顺带还定死了顺序: 先模糊、再补饱和、最后位移, 折射到的是一张已经柔化过的背景, 边缘的
 * 压缩因此是干净的; 反过来先位移再模糊, 会把折射本身一起糊掉。
 */
export function backdropFilterValue(
  mode: GlassMode,
  filterId: string,
  params: LiquidGlassParams,
): string {
  if (mode === "refract") return `url(#${filterId})`;
  const blur = Math.max(clamp(params.blur, 0, 200), FROST_MIN_BLUR);
  return `blur(${fmt(blur)}px) saturate(${fmt(clamp(params.saturation, 0, 400))}%)`;
}
