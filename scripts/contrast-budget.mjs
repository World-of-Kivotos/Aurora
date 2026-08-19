// Aurora 玻璃材质对比度预算求解器（零依赖，node scripts/contrast-budget.mjs 直接跑）。
//
// 为什么要有这个脚本：背景图从「只铺主页那一块」改成「铺满整个 app」之后，
// 每一页的每一块内容都压在照片上，旧那套只针对主页一块纸片算出来的预算全部失效。
// 材质不透明度不是审美参数而是可读性参数，必须实算，不能拍。
//
// 口径与 app/src/lib/appearance.ts 保持一致：
//   - CSS 的 alpha 合成发生在 sRGB 非线性空间（未开 color-interpolation 的默认行为），
//     所以先在 0..255 上做线性插值，再走 WCAG 的传递函数求相对亮度。
//   - backdrop-filter 的 blur / saturate 对纯黑与纯白这两个极值不改变数值
//     （模糊一张纯色图仍是同一纯色；saturate 对无彩色恒等），
//     液态玻璃那档的 brightness(1.05) 同理（0*1.05=0，255 被钳位），
//     故极值分析只需要看 alpha 合成，滤镜可以整体忽略。这正是取黑白两端做判据的原因。

const PAPER = [243, 242, 240]; // --color-paper
const SINK = [236, 235, 231]; // --color-paper-sink（保留，供比对用）
const INK = [20, 22, 26]; // --color-ink
const ACCENT = [200, 53, 47]; // --color-accent
const DANGER = [138, 32, 24]; // --color-danger
const PAPER_ON = [246, 244, 240]; // --color-paper-on（实心块上的字色）
const BLACK = [0, 0, 0];
const WHITE = [255, 255, 255];

/** sRGB 分量（0..255）转线性光。 */
function toLinear(channel) {
  const c = channel / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2.x 相对亮度。 */
function luminance(rgb) {
  return 0.2126 * toLinear(rgb[0]) + 0.7152 * toLinear(rgb[1]) + 0.0722 * toLinear(rgb[2]);
}

/** 把 top 以 alpha 压在 bottom 上（sRGB 空间线性插值，与浏览器合成一致）。 */
function over(top, alpha, bottom) {
  return top.map((c, i) => c * alpha + bottom[i] * (1 - alpha));
}

function contrast(a, b) {
  const la = luminance(a);
  const lb = luminance(b);
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

/**
 * 材质定义。layers 自下而上依次压在背景上。
 * 「寄生层」（control / sunken）自己不铺纸，只铺一层墨洗，
 * 所以它们的 layers 里必须先带上宿主材质——它们的最不利情形取决于寄生在谁身上。
 */
const SHELL = { color: PAPER, alpha: 0.72 };
const PANEL = { color: PAPER, alpha: 0.86 };
const STRONG = { color: PAPER, alpha: 0.96 };
const LIQUID = { color: PAPER, alpha: 0.8 };
const CONTROL_REST = { color: INK, alpha: 0.04 };
const CONTROL_HOVER = { color: INK, alpha: 0.08 };
const SUNKEN_WASH = { color: INK, alpha: 0.08 };

const MATERIALS = [
  { name: ".surface-shell", layers: [SHELL] },
  { name: ".surface-panel", layers: [PANEL] },
  { name: ".surface-panel-strong", layers: [STRONG] },
  { name: ".surface-liquid", layers: [LIQUID] },
  // 寄生层按最不利宿主算：外壳是全站最透的一档，压在它身上就是下界。
  { name: ".surface-control 静息 on shell", layers: [SHELL, CONTROL_REST] },
  { name: ".surface-control 悬停 on shell", layers: [SHELL, CONTROL_HOVER] },
  { name: ".surface-control 悬停 on panel", layers: [PANEL, CONTROL_HOVER] },
  { name: ".surface-sunken on shell", layers: [SHELL, SUNKEN_WASH] },
  { name: ".surface-sunken on panel", layers: [PANEL, SUNKEN_WASH] },
];

const INK_STEPS = [0.6, 0.7, 0.75, 0.8, 0.9, 1];
const BODY = 4.5; // WCAG AA 正文
const LARGE = 3; // WCAG AA 大字 / 图标

/** 材质压在某个极端背景上之后的实际底色。 */
function surfaceOn(layers, bg) {
  return layers.reduce((base, layer) => over(layer.color, layer.alpha, base), bg);
}

/** 某档墨色不透明度的文字，压在给定底色上的对比度。 */
function inkContrastOn(surface, inkAlpha) {
  return contrast(over(INK, inkAlpha, surface), surface);
}

function pad(text, width) {
  const w = [...text].reduce((n, ch) => n + (/[一-鿿＀-￯]/.test(ch) ? 2 : 1), 0);
  return text + " ".repeat(Math.max(0, width - w));
}

function fmt(value) {
  return value.toFixed(2).padStart(5);
}

const rows = [];
for (const material of MATERIALS) {
  const onBlack = surfaceOn(material.layers, BLACK);
  const onWhite = surfaceOn(material.layers, WHITE);
  const worst = INK_STEPS.map((a) =>
    Math.min(inkContrastOn(onBlack, a), inkContrastOn(onWhite, a)),
  );
  // 门槛档位：满足目标的最低墨色档。墨越淡对比越低，单调，直接取首个达标项。
  const floorFor = (target) => {
    const idx = worst.findIndex((c) => c >= target);
    return idx === -1 ? "无（本档不可用）" : `ink/${Math.round(INK_STEPS[idx] * 100)}`;
  };
  rows.push({
    name: material.name,
    black: onBlack,
    white: onWhite,
    worst,
    body: floorFor(BODY),
    large: floorFor(LARGE),
    blackIsWorst: inkContrastOn(onBlack, 1) < inkContrastOn(onWhite, 1),
  });
}

console.log("=== 一、各档材质压在两个极端背景上的实际底色（sRGB 等效 / 相对亮度）===\n");
console.log(pad("材质", 34) + pad("纯黑图上", 22) + "纯白图上");
for (const row of rows) {
  const b = `${row.black.map((c) => Math.round(c)).join(",")} (L=${luminance(row.black).toFixed(3)})`;
  const w = `${row.white.map((c) => Math.round(c)).join(",")} (L=${luminance(row.white).toFixed(3)})`;
  console.log(pad(row.name, 34) + pad(b, 22) + w);
}

console.log("\n=== 二、对比度矩阵（两端取最不利那一端）===\n");
console.log(pad("材质", 34) + INK_STEPS.map((a) => `ink/${Math.round(a * 100)}`.padStart(8)).join(""));
for (const row of rows) {
  console.log(pad(row.name, 34) + row.worst.map((c) => fmt(c).padStart(8)).join(""));
}

console.log("\n=== 三、结论：每档材质上的最低可用墨色档 ===\n");
console.log(pad("材质", 34) + pad(`正文 >=${BODY}`, 16) + pad(`大字/图标 >=${LARGE}`, 18) + "最不利端");
for (const row of rows) {
  console.log(
    pad(row.name, 34) +
      pad(row.body, 16) +
      pad(row.large, 18) +
      (row.blackIsWorst ? "纯黑图" : "纯白图"),
  );
}

console.log("\n=== 四、求解：各档纸色不透明度的下限（自足材质，非寄生层）===\n");
console.log("在纯黑图上让指定墨色档达到指定门槛，所需的最小纸色不透明度：\n");
console.log(pad("目标", 34) + "最小不透明度");
const targets = [
  ["满墨正文 >=4.5（硬底线）", 1, BODY],
  ["ink/75 正文 >=4.5", 0.75, BODY],
  ["ink/70 正文 >=4.5", 0.7, BODY],
  ["ink/60 正文 >=4.5", 0.6, BODY],
  ["ink/60 大字 >=3.0", 0.6, LARGE],
];
for (const [label, inkAlpha, target] of targets) {
  let found = null;
  for (let alpha = 0.4; alpha <= 1.0001; alpha += 0.005) {
    const surface = over(PAPER, Math.min(alpha, 1), BLACK);
    if (inkContrastOn(surface, inkAlpha) >= target) {
      found = Math.min(alpha, 1);
      break;
    }
  }
  console.log(pad(label, 34) + (found === null ? "不可达" : `${(found * 100).toFixed(1)}%`));
}

console.log("\n=== 五、寄生层的墨洗上限（不得把宿主拖下门槛）===\n");
console.log("在最透的宿主 .surface-shell（纸色 72%）上，墨洗浓度对 ink/75 正文的影响：\n");
console.log(pad("墨洗浓度", 16) + pad("底色", 16) + "ink/75 对比度");
for (const wash of [0, 0.05, 0.08, 0.1, 0.13, 0.16, 0.2]) {
  const surface = over(INK, wash, over(PAPER, 0.72, BLACK));
  const c = inkContrastOn(surface, 0.75);
  console.log(
    pad(`${(wash * 100).toFixed(0)}%`, 16) +
      pad(surface.map((v) => Math.round(v)).join(","), 16) +
      `${c.toFixed(2)}${c >= BODY ? "" : "  (跌破 4.5)"}`,
  );
}

/*
 * 第六节存在的理由：上面五节只算了墨色通道，而 accent 在界面里被当成真文字色用（不是装饰图标）。
 * 一条只覆盖墨色的预算，等于给彩色文字发了一张永远查不到的免检通行证——
 * 全站铺图之后，正是这批彩色小字先跌破门槛，却没有任何一格数字能拦住它。
 */
console.log("\n=== 六、彩色文字：强调色与危险色能不能当字用 ===\n");
console.log(pad("材质", 34) + pad("accent 满色", 15) + pad("danger 满色", 15) + pad("danger/85", 12) + "danger/80");
for (const material of MATERIALS) {
  const onBlack = surfaceOn(material.layers, BLACK);
  const onWhite = surfaceOn(material.layers, WHITE);
  const worstOf = (color, alpha) =>
    Math.min(
      contrast(over(color, alpha, onBlack), onBlack),
      contrast(over(color, alpha, onWhite), onWhite),
    );
  console.log(
    pad(material.name, 34) +
      pad(fmt(worstOf(ACCENT, 1)), 15) +
      pad(fmt(worstOf(DANGER, 1)), 15) +
      pad(fmt(worstOf(DANGER, 0.85)), 12) +
      fmt(worstOf(DANGER, 0.8)),
  );
}

console.log("\n淡底徽标（bg-accent/12 底 + text-accent 字）在两端的实算：\n");
console.log(pad("宿主材质", 34) + pad("纯黑图上", 12) + "纯白图上");
for (const material of MATERIALS) {
  const badgeOn = (bg) => {
    const host = surfaceOn(material.layers, bg);
    return contrast(ACCENT, over(ACCENT, 0.12, host));
  };
  console.log(pad(material.name, 34) + pad(fmt(badgeOn(BLACK)), 12) + fmt(badgeOn(WHITE)));
}

const strongOnBlack = surfaceOn([STRONG], BLACK);
const shellOnBlack = surfaceOn([SHELL], BLACK);
console.log("\n结论（写进 app.css 的第一节，代码侧按这三条执行）：");
console.log(
  `  1. accent 不是文字色：最实的一档（96%）上也只有 ${contrast(ACCENT, strongOnBlack).toFixed(2)}，全档过不了 4.5。`,
);
console.log("     它只能做实心块的底、纯装饰，以及 3.0 就够的 aria-hidden 图标（且不得落在外壳上）。");
console.log(
  `  2. 状态徽标一律「实心 accent 底 + paper-on 字」= ${contrast(PAPER_ON, ACCENT).toFixed(2)}，与背景图无关（底不透明）。`,
);
console.log("     bg-accent/12 那种淡底最高只到 4.06，暗图上跌破 3.0，禁止再用。");
console.log(
  `  3. danger 满色可当正文，但仅限面板档及以上（外壳上只有 ${contrast(DANGER, shellOnBlack).toFixed(2)}）；`,
);
console.log("     danger/85 与 danger/80 一律不得再出现，见上表控件底那两行。");

console.log(`\n参考：纸底 #f3f2f0 上满墨的基准对比度 = ${contrast(INK, PAPER).toFixed(2)}`);
console.log(`参考：--color-paper-sink 与 --color-paper 的亮度差 = ${(luminance(PAPER) - luminance(SINK)).toFixed(4)}（不足以单独承担「下沉」语义，故下沉改用墨洗）`);
