// 启动屏右下角那一撮的三态判定。
//
// 守的是「默认状态下这一屏还读不读得出来」：Aurora 装完就是没自选壁纸的状态，那时铺的是内置
// master.jpg——一张右下角很暗的图（实测 p10=2 / p90=57），字必须走纸色裸字。这条一旦退化成
// 「按 appearance.background 判有没有图」，默认那一屏就会拿墨色字压在被压暗层再压过一道的
// 深图上：类型对、测试过、构建成，界面也不报错，只是那一撮字看不见了。

import { describe, expect, it } from "vitest";
import { launchPlate } from "./Home";
import { EMPTY_APPEARANCE } from "../lib/appearance";
import type { AppearanceDto, BuiltinBackground } from "../lib/ipc";

/** 后端 list_builtin_backgrounds 的实测返回，与 ipc-mock 及 aurora-core 单测同源。 */
const BUILTINS: BuiltinBackground[] = [
  { id: "master", tint: "#8c8192", plate: { p10: 2, p90: 57 } },
  { id: "arena", tint: "#474471", plate: { p10: 2, p90: 41 } },
];

function appearanceOf(patch: Partial<AppearanceDto> = {}): AppearanceDto {
  return { ...EMPTY_APPEARANCE, ...patch };
}

describe("launchPlate", () => {
  it("默认状态（没自选壁纸）：铺内置 master，走纸色裸字而不是纸片", () => {
    const plate = launchPlate(appearanceOf(), BUILTINS);

    expect(plate.mode).toBe("paperOn");
    expect(plate.onPhoto).toBe(true);
    expect(plate.naked).toBe(true);
    // 基色必须挂在容器上：版本名与账户名没写颜色类，靠继承拿色。
    expect(plate.cls).toContain("text-paper-on");
    expect(plate.cls).not.toContain("surface-panel-strong");
    // 分档字色跟着走，纸片档那套墨色阶不该再出现。
    expect(plate.fg("text-ink/75", "mid")).toBe("text-paper-on/85");
    expect(plate.fg("text-ink/75", "weak")).toBe("text-paper-on/70");
  });

  it("自选壁纸压过内置图：亮图走满墨裸字，取样用的是壁纸那张的值", () => {
    const plate = launchPlate(
      appearanceOf({ background: "noon.jpg", tint: "#d8d4cc", plate: { p10: 200, p90: 250 } }),
      BUILTINS,
    );

    expect(plate.mode).toBe("ink");
    expect(plate.cls).toContain("text-ink");
    expect(plate.cls).not.toContain("text-paper-on");
    expect(plate.fg("text-ink/75", "weak")).toBe("text-ink");
  });

  it("壁纸明暗跨度大到两种字色都撑不住时，才退回磨砂纸片", () => {
    const plate = launchPlate(
      appearanceOf({ background: "split.jpg", tint: "#8a8a8a", plate: { p10: 2, p90: 250 } }),
      BUILTINS,
    );

    expect(plate.mode).toBe("plate");
    expect(plate.naked).toBe(false);
    expect(plate.cls).toContain("surface-panel-strong");
    // 纸片档把原色阶原样还回去，字坐在材质上按 app.css 那张实算表走。
    expect(plate.fg("text-ink/75", "mid")).toBe("text-ink/75");
  });

  it("柔化会把深图提亮，纸色字该失效时就得失效", () => {
    // 内置 master 在不柔化时走纸色裸字；柔化拉满后底色被纸色抬上来，改走满墨。
    expect(launchPlate(appearanceOf({ veil: 0 }), BUILTINS).mode).toBe("paperOn");
    expect(launchPlate(appearanceOf({ veil: 60 }), BUILTINS).mode).toBe("ink");
  });

  it("内置表还没到手的那一两帧：没有图可铺，坐纸底且不改字色", () => {
    const plate = launchPlate(appearanceOf(), []);

    expect(plate.onPhoto).toBe(false);
    expect(plate.naked).toBe(false);
    expect(plate.cls).not.toContain("surface-panel-strong");
    expect(plate.cls).not.toContain("text-paper-on");
    expect(plate.fg("text-ink/75", "mid")).toBe("text-ink/75");
  });
});
