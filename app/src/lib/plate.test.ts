// plateLook 是「判定结果 -> 字色类」这一层的唯一实现，启动屏与竞技场屏共用它。
// 对比度实算归 appearance.ts 的 plateMode，这里钉的是它上面那层映射：三态各自的容器基色、
// 两种裸字档的分级规则（ink 档不许分级、paperOn 档必须分级），以及柔化确实被喂了进去。

import { describe, expect, it } from "vitest";
import { plateLook } from "./plate";
import type { ResolvedBackground } from "./appearance";

// 竞技场那张内置图的实测取样值，与后端登记表量出来的一致（list_builtin_backgrounds 的返回）。
// 用真值而不是编一组：这一档的结论（纸色裸字）正是靠这两个数得出来的，编的数验不到那件事。
const ARENA: ResolvedBackground = {
  kind: "builtin",
  id: "arena",
  tint: "#474471",
  plate: { p10: 2, p90: 41 },
};

/** 亮图：最暗的那一成也够亮，满墨字达标。 */
const BRIGHT: ResolvedBackground = {
  kind: "library",
  file: "noon.jpg",
  tint: "#d8d4cc",
  plate: { p10: 200, p90: 250 },
};

/** 明暗跨度大：满墨字栽在 p10，纸色字栽在 p90，两档都撑不住，只能退回纸片。 */
const SPLIT: ResolvedBackground = {
  kind: "library",
  file: "split.jpg",
  tint: "#8a8a8a",
  plate: { p10: 2, p90: 250 },
};

/** 本功能上线前导入的老图，没量过。 */
const UNMEASURED: ResolvedBackground = {
  kind: "library",
  file: "old.jpg",
  tint: null,
  plate: null,
};

/** 中等偏暗：柔化拉满前后会落到不同档，用来验 veil 真的被算了进去。 */
const MIDDARK: ResolvedBackground = {
  kind: "library",
  file: "dusk.jpg",
  tint: "#4a4a4a",
  plate: { p10: 2, p90: 100 },
};

describe("plateLook", () => {
  it("深色图：纸色裸字，且层级三档不许压成一个白", () => {
    const look = plateLook(ARENA, 0);
    expect(look.mode).toBe("paperOn");
    expect(look.naked).toBe(true);
    expect(look.onPhoto).toBe(true);
    expect(look.baseTone).toBe("text-paper-on");
    // 次要与附注必须分得开：全篇一个白会读成一张扁平清单。
    expect(look.fg("text-ink/75", "mid")).toBe("text-paper-on/85");
    expect(look.fg("text-ink/75", "weak")).toBe("text-paper-on/70");
  });

  it("亮图：满墨裸字，且这一档只能全满墨——余量不够再分级", () => {
    const look = plateLook(BRIGHT, 0);
    expect(look.mode).toBe("ink");
    expect(look.naked).toBe(true);
    expect(look.baseTone).toBe("text-ink");
    expect(look.fg("text-ink/75", "mid")).toBe("text-ink");
    expect(look.fg("text-ink/75", "weak")).toBe("text-ink");
  });

  it("跨度大的图退回纸片：不给基色，字色原样交还调用方", () => {
    const look = plateLook(SPLIT, 0);
    expect(look.mode).toBe("plate");
    expect(look.naked).toBe(false);
    expect(look.onPhoto).toBe(true);
    expect(look.baseTone).toBe("");
    expect(look.fg("text-ink/75", "mid")).toBe("text-ink/75");
    expect(look.fg("text-ink/60", "weak")).toBe("text-ink/60");
  });

  it("没量过的图一律上纸片：宁可多一块纸，也不拿没量过的图赌可读性", () => {
    expect(plateLook(UNMEASURED, 0).mode).toBe("plate");
    expect(plateLook(UNMEASURED, 0).onPhoto).toBe(true);
  });

  it("没有图（首屏那一两帧）：坐纸底，既不裸字也不铺纸片", () => {
    const look = plateLook(null, 0);
    expect(look.mode).toBe("plate");
    expect(look.naked).toBe(false);
    // onPhoto 为假是「别铺纸片」的唯一依据：一块纸压在纯纸底上是白挖一块。
    expect(look.onPhoto).toBe(false);
    expect(look.baseTone).toBe("");
  });

  it("柔化算进判定：同一张图拉满柔化后换了一档", () => {
    // 柔化把纸色压上去，最亮那一成被提亮到纸色字撑不住；同时最暗那一成也被提亮，满墨字反而达标了。
    expect(plateLook(MIDDARK, 0).mode).toBe("paperOn");
    expect(plateLook(MIDDARK, 60).mode).toBe("ink");
  });
});
