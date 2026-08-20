// 竞技场屏的两条硬约束，用静态渲染钉死（做法同 Account.test.tsx：renderToStaticMarkup，
// 测试环境里没有 DOM）。
//
// 一、零交互。这一屏不许出现任何可点、可聚焦的东西——竞技场没有任何可用功能，
//     一颗按得动的键就是按下去没有后续的承诺。文件头那段话是给人看的，这一条是给机器看的：
//     谁往里加了 button / a / input，或者顺手挂一个 tabindex，这里当场挂。
// 二、字色跟着图走。三种形态各验一遍，防的是「写死墨色」那类改动——它在开发机那张固定壁纸上
//     一点问题都看不出来，换一张深色图才现形。

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ArenaScreen } from "./Arena";
import type { ResolvedBackground } from "../lib/appearance";

/** 竞技场那张内置图的实测取样值（后端 list_builtin_backgrounds 的返回）：右下角偏暗，走纸色裸字。 */
const ARENA: ResolvedBackground = {
  kind: "builtin",
  id: "arena",
  tint: "#474471",
  plate: { p10: 2, p90: 41 },
};

/** 亮图：满墨裸字。 */
const BRIGHT: ResolvedBackground = {
  kind: "library",
  file: "noon.jpg",
  tint: "#d8d4cc",
  plate: { p10: 200, p90: 250 },
};

/** 明暗跨度大：两种字色都撑不住，退回磨砂纸片。 */
const SPLIT: ResolvedBackground = {
  kind: "library",
  file: "split.jpg",
  tint: "#8a8a8a",
  plate: { p10: 2, p90: 250 },
};

function screen(background: ResolvedBackground | null, veil = 0): string {
  return renderToStaticMarkup(<ArenaScreen background={background} veil={veil} />);
}

// 可交互元素与可聚焦标记。contenteditable 也算：它不是标签，但同样能拿到焦点。
const INTERACTIVE = /<(?:a|button|input|select|textarea|form|label|summary)[\s>]|tabindex|contenteditable|role="(?:button|link)"/i;

describe("竞技场屏", () => {
  it("四种背景形态下都是零交互", () => {
    for (const markup of [screen(null), screen(ARENA), screen(BRIGHT), screen(SPLIT)]) {
      expect(markup).not.toMatch(INTERACTIVE);
    }
  });

  it("主角只有 COMING SOON，字号与启动屏那颗主操作键同一档", () => {
    const markup = screen(ARENA);
    expect(markup).toContain("COMING SOON");
    expect(markup).toContain("text-[46px]");
    // 副标只陈述此刻的事实，不许出现上线时间或任何承诺口径。
    expect(markup).toContain("竞技场尚未开放");
  });

  it("深色图：纸色裸字压图，不铺纸片", () => {
    const markup = screen(ARENA);
    expect(markup).toContain("text-paper-on");
    expect(markup).not.toContain("surface-panel-strong");
  });

  it("亮图：满墨裸字压图，同样不铺纸片", () => {
    const markup = screen(BRIGHT);
    expect(markup).toContain("text-ink");
    expect(markup).not.toContain("text-paper-on");
    expect(markup).not.toContain("surface-panel-strong");
  });

  it("图撑不住时退回磨砂纸片，副标改用纸片档的墨色", () => {
    const markup = screen(SPLIT);
    expect(markup).toContain("surface-panel-strong");
    expect(markup).toContain("text-ink/75");
    expect(markup).not.toContain("text-paper-on");
  });

  it("没有图（首屏那一两帧）：坐纸底，不铺纸片也不改字色", () => {
    const markup = screen(null);
    expect(markup).not.toContain("surface-panel-strong");
    expect(markup).not.toContain("text-paper-on");
  });
});
