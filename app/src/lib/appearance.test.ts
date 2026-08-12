// plateMode 决定主页右下角那撮字是裸压在图上还是退回纸片，直接决定可读性，
// 所以这里不测「返回了某个值」，而是独立复算 WCAG 对比度，验证它给出的每一档都真的达标。
//
// 对比度这套公式在测试里重写一遍是有意的：与 appearance.ts 共用实现的话，
// 常数写错、合成顺序写反都测不出来——两边一起错，断言照样通过。

import { describe, expect, it } from "vitest";
import { plateMode, SCRIM_ALPHA, type PlateMode } from "./appearance";
import type { PlateZone } from "./ipc";

// ---- 独立实现的 WCAG 口径，作为判定的对照物 ----

function lumaOfSrgb(srgb: number): number {
  const c = srgb / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

function srgbOfLuma(luma: number): number {
  return (luma <= 0.0031308 ? luma * 12.92 : 1.055 * luma ** (1 / 2.4) - 0.055) * 255;
}

function contrast(a: number, b: number): number {
  const [hi, lo] = a > b ? [a, b] : [b, a];
  return (hi + 0.05) / (lo + 0.05);
}

const L_INK = lumaOfSrgb(22);
const L_PAPER_ON = 0.905855;
const PAPER_SRGB = 242;
const INK_SRGB = 22;

/** 取样值经柔化、压暗两层后的实际底色亮度。顺序必须是 图 -> 柔化 -> 压暗。 */
function backdrop(sampled: number, veil: number, scrim: number): number {
  const image = srgbOfLuma(sampled / 255);
  const veiled = PAPER_SRGB * (veil / 100) + image * (1 - veil / 100);
  return lumaOfSrgb(INK_SRGB * scrim + veiled * (1 - scrim));
}

/** 某一档在最不利端实际拿到的对比度。 */
function achieved(mode: PlateMode, zone: PlateZone, veil: number): number {
  if (mode === "ink") return contrast(backdrop(zone.p10, veil, 0), L_INK);
  if (mode === "paperOn") return contrast(backdrop(zone.p90, veil, SCRIM_ALPHA), L_PAPER_ON);
  throw new Error("plate 档没有裸字，不该问它对比度");
}

/** 声明的余量档位：比 AA 的 4.5 高一档，用来兜住 p10/p90 之外那一成像素。 */
const TARGET = 5.5;

describe("plateMode", () => {
  it("没量过的图退回纸片，绝不拿未知底色赌可读性", () => {
    expect(plateMode(null, 0)).toBe("plate");
    expect(plateMode(null, 60)).toBe("plate");
  });

  it("亮角落用满墨裸字", () => {
    expect(plateMode({ p10: 200, p90: 250 }, 0)).toBe("ink");
  });

  it("暗角落用纸色裸字", () => {
    expect(plateMode({ p10: 2, p90: 30 }, 0)).toBe("paperOn");
  });

  it("明暗横跨两端的角落退回纸片：两种字色都撑不住", () => {
    expect(plateMode({ p10: 3, p90: 200 }, 0)).toBe("plate");
  });

  it("玩家实测的那张壁纸（p10=4 p90=66）走纸色裸字", () => {
    const real: PlateZone = { p10: 4, p90: 66 };
    expect(plateMode(real, 0)).toBe("paperOn");
    // 不只断言档位，连它究竟拿到多少对比度也钉住，避免常数被改松了还悄悄通过。
    expect(achieved("paperOn", real, 0)).toBeGreaterThan(6);
  });

  it("柔化会提亮底色，深色图因此失去纸色裸字资格", () => {
    const dark: PlateZone = { p10: 4, p90: 66 };
    expect(plateMode(dark, 0)).toBe("paperOn");
    // 柔化是压在图上的纸色层。判定若不把它算进去，玩家一拉滑条浅色字就该失效却仍在用。
    expect(plateMode(dark, 40)).not.toBe("paperOn");
  });

  it("柔化拉满后底色足够亮，深色图反过来可用满墨", () => {
    expect(plateMode({ p10: 4, p90: 66 }, 60)).toBe("ink");
  });

  // 这是本文件的核心断言：凡是判成裸字的组合，实际对比度必须真的达标。
  // 任何一个阈值常数被改松、或合成顺序被写反，都会在这里挂掉。
  it("所有判成裸字的组合都必须实际达到声明的余量", () => {
    let inkSeen = 0;
    let paperSeen = 0;
    for (let p10 = 0; p10 <= 255; p10 += 5) {
      for (let p90 = p10; p90 <= 255; p90 += 5) {
        for (const veil of [0, 15, 30, 45, 60]) {
          const zone: PlateZone = { p10, p90 };
          const mode = plateMode(zone, veil);
          if (mode === "plate") continue;
          if (mode === "ink") inkSeen++;
          else paperSeen++;
          expect(
            achieved(mode, zone, veil),
            `p10=${p10} p90=${p90} veil=${veil} 判成 ${mode} 却不达标`,
          ).toBeGreaterThanOrEqual(TARGET - 1e-9);
        }
      }
    }
    // 防止上面那层循环因为「全都判成 plate」而空转通过。
    expect(inkSeen).toBeGreaterThan(100);
    expect(paperSeen).toBeGreaterThan(100);
  });

  it("裸字判定只看最不利那一端，不看均值", () => {
    // 两者均值相同，但暗端差得远：靠均值判会给出同一档，靠 p10 判才会分开。
    const even: PlateZone = { p10: 120, p90: 140 };
    const skewed: PlateZone = { p10: 10, p90: 250 };
    expect(plateMode(even, 0)).toBe("ink");
    expect(plateMode(skewed, 0)).toBe("plate");
  });
});
