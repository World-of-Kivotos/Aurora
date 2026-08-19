// photoShows 决定外壳（标题栏、侧栏）要不要上玻璃材质：装了壁纸就上，没装就保持不透明纸色。
//
// 全站铺图之前它还有半条「只在主页」的判据，现在连路由参数都不存在了——
// 「路由不参与判定」由函数签名本身守住，不必再靠用例守，所以这里只验剩下的那一条判据。

import { describe, expect, it } from "vitest";
import { applyGlassMode, EMPTY_APPEARANCE, photoShows } from "./appearance";

describe("photoShows", () => {
  it("装了壁纸就算压在图上", () => {
    expect(photoShows("wall.jpg")).toBe(true);
    // 判据是「有没有这个字段」而不是字符串真值：空串在协议里不是合法文件名，
    // 但真要出现，它代表的仍然是「设过一张图」，退回纸色反而会和外壳的其余部分对不上。
    expect(photoShows("")).toBe(true);
  });

  it("没装壁纸时为假，免得没背景的人平白看到一层玻璃", () => {
    expect(photoShows(null)).toBe(false);
  });
});

// 玻璃模式的 DOM 契约。CSS 侧只认 :root[data-glass="liquid"]，缺省即 frost，
// 所以这里要钉死的不是「写了什么」，而是「frost 档必须把属性拿掉」——
// 写成 data-glass="frost" 在 CSS 上同样落在保守档，看起来没坏，
// 但同一个状态从此有两种形态，下一个人再判断当前档位时就会两边都要判。
describe("applyGlassMode", () => {
  it("液态档写下属性", () => {
    const root = { dataset: {} as DOMStringMap };
    applyGlassMode("liquid", root);
    expect(root.dataset.glass).toBe("liquid");
  });

  it("磨砂档删掉属性而不是写成 frost", () => {
    const root = { dataset: { glass: "liquid" } as DOMStringMap };
    applyGlassMode("frost", root);
    expect("glass" in root.dataset).toBe(false);
  });

  it("反复切换不残留上一次的值", () => {
    const root = { dataset: {} as DOMStringMap };
    applyGlassMode("liquid", root);
    applyGlassMode("frost", root);
    applyGlassMode("liquid", root);
    expect(root.dataset.glass).toBe("liquid");
    applyGlassMode("frost", root);
    expect(root.dataset.glass).toBeUndefined();
  });

  it("首屏初值落在保守档：配置还没读回来时不能先出一层液态", () => {
    expect(EMPTY_APPEARANCE.glass).toBe("frost");
  });
});
