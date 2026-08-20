// 裸字压图那几屏（启动屏 / 竞技场屏）的字色与容器形态。
//
// 判定本身不在这里：对比度按背景图右下角的 p10/p90 取样实算，那段在 appearance.ts 的 plateMode。
// 这一层只把「判定结果 -> Tailwind 类」收成一处。抽出来的理由是它有第二个调用点了——
// 同一套三态在两屏各写一遍，改一次判定必然漏掉一屏，而漏掉的症状是「字在某些图上读不出来」，
// 恰恰是开发机那张固定壁纸最不容易撞见的一种。
//
// 入参是解析后的那张图而不是整份 AppearanceDto：这套判定只关心「实际铺着的是哪张图、它的
// 右下角量出来什么」，自选壁纸压内置图那条优先级归 resolveBackground 说了算，两件事不搅在一起。

import { photoShows, plateMode, type PlateMode, type ResolvedBackground } from "./appearance";

/** 裸字的层级档：mid 是次要信息（版本副行、副标），weak 是标签与附注。 */
export type PlateTier = "mid" | "weak";

export interface PlateLook {
  /** 三态判定结果；没有图时恒为 plate。 */
  mode: PlateMode;
  /** 背景图是否在场。为假时容器该退回无材质的纸底档，而不是拿一块纸压在纸上。 */
  onPhoto: boolean;
  /** 字是否裸压在图上。为真时容器要自带基色（baseTone），为假时字坐在材质或纸底上。 */
  naked: boolean;
  /**
   * 裸字模式下容器的基色类；另两档为空串。
   *
   * 基色定在容器上而不是逐个节点写：页面里多数文字本来就没写颜色类、靠继承 body 的墨色，
   * 压在深色图上直接看不见。逐处去补是治标，往后谁再加一行不写颜色的文字就会重犯；
   * 定在容器上，新增的节点自动就是对的。
   */
  baseTone: string;
  /**
   * 分档字色。shade 是纸片 / 纸底档沿用的原色阶，tier 是它在层级里的档位。
   *
   * 两种裸字档的余量天差地别，不能套同一套规则：满墨字压在浅色图上，准入线只保证满强度
   * 达到 4.5:1，所以这一档只能全满墨，层级交给字号与字重；纸色字压在压暗后的深色图上则
   * 宽裕得多，三级都过得了 4.5，层级该还回来就还回来，全篇一个白只会读成一张扁平清单。
   */
  fg: (shade: string, tier: PlateTier) => string;
}

/**
 * `background` 传 resolveBackground 的结果，`veil` 传 AppearanceDto.veil（柔化会把底色提亮，
 * 判定必须跟着它变，否则玩家把柔化一拉、浅色字早该失效了却还在用）。
 */
export function plateLook(background: ResolvedBackground | null, veil: number): PlateLook {
  const onPhoto = photoShows(background);
  // 判空写成内联而不是复用上面那个布尔：TS 只认得出内联这一种收窄。
  const mode: PlateMode = background !== null ? plateMode(background.plate, veil) : "plate";
  return {
    mode,
    onPhoto,
    naked: mode !== "plate",
    baseTone: mode === "ink" ? "text-ink" : mode === "paperOn" ? "text-paper-on" : "",
    fg: (shade, tier) =>
      mode === "ink"
        ? "text-ink"
        : mode === "paperOn"
          ? tier === "mid"
            ? "text-paper-on/85"
            : "text-paper-on/70"
          : shade,
  };
}
