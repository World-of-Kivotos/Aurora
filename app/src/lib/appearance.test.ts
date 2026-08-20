// 背景解析层。这一层回答两个问题：这一帧铺哪张图（自选壁纸还是游戏自带的那张），
// 以及外壳要不要上玻璃材质。
//
// 判定必须落在纯函数里才能这样验：真机上「切游戏」是一次路由跳转加一次协议取图，
// 拿不到中间那个结论；而结论错了的表现（换了游戏图没换、撤掉壁纸整屏变白）在截图上
// 又都长得像「图没加载出来」。

import { describe, expect, it } from "vitest";
import {
  applyGlassMode,
  ARENA_GAME_ID,
  EMPTY_APPEARANCE,
  gameScreenOf,
  MAIN_GAME_ID,
  photoShows,
  resolveBackground,
} from "./appearance";
import type { AppearanceDto, BuiltinBackground } from "./ipc";

// 取样值抄的是 Rust 侧对那两张图的实测结果（aurora-core 的单测把它们钉死了）。
// 用真值而不是编两个数，断言才能直接比对「拿到的是哪一张」，而不是只验「拿到了个对象」。
const BUILTINS: BuiltinBackground[] = [
  { id: "master", tint: "#8c8192", plate: { p10: 2, p90: 57 } },
  { id: "arena", tint: "#474471", plate: { p10: 2, p90: 41 } },
];

/** 没自选过壁纸：background 为 null，此时该由内置表说了算。 */
const NO_PICK: AppearanceDto = EMPTY_APPEARANCE;

/** 自选过一张壁纸，取样值与内置的任何一张都不同，混了一眼就能看出来。 */
const PICKED: AppearanceDto = {
  background: "雪山黄昏.jpg",
  tint: "#4a6274",
  plate: { p10: 96, p90: 158 },
  veil: 15,
  glass: "frost",
};

describe("resolveBackground", () => {
  it("自选壁纸压过内置图，且切游戏也不换", () => {
    // 两台游戏解出同一张：壁纸是「我的壁纸」不是「这台游戏的图」，
    // 切一下侧栏就把玩家亲手选的图换掉，会读成启动器把设置弄丢了。
    for (const game of [MAIN_GAME_ID, ARENA_GAME_ID]) {
      expect(resolveBackground(PICKED, BUILTINS, game)).toEqual({
        kind: "library",
        file: "雪山黄昏.jpg",
        tint: "#4a6274",
        plate: { p10: 96, p90: 158 },
      });
    }
  });

  it("自选壁纸在内置表还没到手时照样生效", () => {
    // 两条数据各拉各的，到达顺序不该改变结论。
    expect(resolveBackground(PICKED, [], MAIN_GAME_ID)).toEqual({
      kind: "library",
      file: "雪山黄昏.jpg",
      tint: "#4a6274",
      plate: { p10: 96, p90: 158 },
    });
  });

  it("没自选就按当前游戏取内置那张，取样值跟着换", () => {
    expect(resolveBackground(NO_PICK, BUILTINS, MAIN_GAME_ID)).toEqual({
      kind: "builtin",
      id: "master",
      tint: "#8c8192",
      plate: { p10: 2, p90: 57 },
    });
    // 竞技场那张的取样与主服不同：只验 id 的话，「图换了但 tint/plate 还是上一张的」
    // 这种错法能整条溜过去——而那正是开机闪色与裸字判错档的来源。
    expect(resolveBackground(NO_PICK, BUILTINS, ARENA_GAME_ID)).toEqual({
      kind: "builtin",
      id: "arena",
      tint: "#474471",
      plate: { p10: 2, p90: 41 },
    });
  });

  it("认不出的游戏 id 退回主服那张，而不是空着", () => {
    // 这种 id 只可能来自「加了新路由却忘了登记」。少一张图是整屏空白，多一张是背景不对，
    // 后者轻得多。
    expect(resolveBackground(NO_PICK, BUILTINS, "world-of-kivotos-3")).toEqual({
      kind: "builtin",
      id: "master",
      tint: "#8c8192",
      plate: { p10: 2, p90: 57 },
    });
  });

  it("没自选、内置表也没到手时才为空", () => {
    // 首屏那一两帧，以及内置表拉取失败的情形。这一帧退回纯纸底，是唯一没有图的时候。
    expect(resolveBackground(NO_PICK, [], MAIN_GAME_ID)).toBeNull();
  });
});

describe("gameScreenOf", () => {
  it("两条启动屏路由各自认出自己那台游戏", () => {
    expect(gameScreenOf("/")).toBe(MAIN_GAME_ID);
    expect(gameScreenOf("/arena")).toBe(ARENA_GAME_ID);
  });

  it("内页不属于任何一台游戏", () => {
    // 判空的下游有两件事：背景退回主服那张，以及右下角的压暗层不铺。
    // 内页的文字都落在材质上，压暗对它们只是白白压暗一角。
    for (const path of ["/account", "/download", "/instance", "/settings"]) {
      expect(gameScreenOf(path)).toBeNull();
    }
  });
});

// photoShows 决定外壳（标题栏、侧栏）要不要上玻璃材质。判据已随内置默认背景改成
// 「解析器有没有给出一张图」——它没有恒真：内置表还没到手的那一帧仍然为假，
// 那一帧的玻璃采的是一张纯色，模糊出来还是同一个颜色，纯属白烧一次全表面合成。
describe("photoShows", () => {
  it("解析出图就算压在图上，自选与内置一视同仁", () => {
    expect(photoShows(resolveBackground(PICKED, BUILTINS, MAIN_GAME_ID))).toBe(true);
    expect(photoShows(resolveBackground(NO_PICK, BUILTINS, ARENA_GAME_ID))).toBe(true);
  });

  it("一张图都还没有的那一帧为假，免得白采一遍纯色", () => {
    expect(photoShows(resolveBackground(NO_PICK, [], MAIN_GAME_ID))).toBe(false);
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
