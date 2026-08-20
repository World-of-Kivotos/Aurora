// 竞技场屏：World of Kivotos : Arena 现在只有一句 COMING SOON。
//
// 严格零交互。这一屏不许出现 button / a / input / 任何可聚焦元素，往后也不许加。
// 这不是「暂时还没做」：竞技场没有任何可用功能，摆一颗按得动的键就是给一个按下去没有后续的
// 承诺。侧栏那一行从「敬请期待」放开成可点之后，玩家点进来要得到的答案只有「还没开」这一个，
// 那句话由这一屏自己说清就够了。真开放时是整屏重写，不是在这撮字旁边补一颗键。
// 同名用例（Arena.test.tsx）会把渲染结果扫一遍，加了可点物当场挂。
//
// 版面与启动屏同构：整版留给背景图（图由外壳铺，本页不碰），只有右下角一撮字。
// 字色三态判定与启动屏共用 lib/plate.ts —— 直接写死墨色的话，换一张深色图就读不出来了。

import { motion } from "framer-motion";
import { useAppearance } from "../lib/appearance-context";
import { ARENA_GAME_ID, resolveBackground, type ResolvedBackground } from "../lib/appearance";
import { plateLook } from "../lib/plate";
import { pageItem } from "../lib/motion";

/**
 * 这一撮字的公共版位：贴右下角，按自然高度占位。
 *
 * mt-auto 吃掉上方全部留白。启动屏那边禁用 mt-auto 是因为它同一根 flex 列里有两个块要贴底，
 * 剩余空间会被两个 auto 外边距均分；这一屏只有这一块，不存在那个问题。
 * self-end 是给纸片档用的：容器在列里默认 stretch，不收窄的话那块纸会横贯整屏。
 */
const SCREEN = "mt-auto flex shrink-0 flex-col items-end self-end text-right";

/**
 * 兜底纸片：图的明暗跨度大到两种字色都撑不住时才用，与启动屏同一档材质（最实的那一档）。
 * 内边距比启动屏那块小一圈——那边裹着三行信息加一颗主操作键，这边只有一句话加一行副标。
 */
const PLATE_FROSTED = "surface-panel-strong rounded-panel px-7 py-6";

/**
 * 纯渲染层，背景由参数注入。与取数分开是为了让三种形态都能被静态渲染断言直接覆盖：
 * 测试环境里没有 DOM 也没有 Provider，走 context 就只测得到一种形态。
 */
export function ArenaScreen({
  background,
  veil,
}: {
  background: ResolvedBackground | null;
  veil: number;
}) {
  const { naked, onPhoto, baseTone, fg } = plateLook(background, veil);
  // 三种形态，与启动屏右下角同构：无图坐纸底 / 有图且撑得住就裸字压图 / 图撑不住才退回磨砂纸片。
  const plate = naked ? baseTone : onPhoto ? PLATE_FROSTED : "";

  return (
    <motion.section variants={pageItem} aria-label="竞技场" className={`${SCREEN} ${plate}`}>
      {/* 这一屏唯一的主角，字号与启动屏那颗主操作键同一档（46px）。
          字距往开里拉是全大写字组的常规处理，但拉开的那一格会跟在最后一个字母后面：
          右对齐时它把整行往左顶出半个字距、与下面那行副标错开，所以按同一个数补回去。 */}
      <div className="-mr-[0.14em] text-[46px] leading-none font-extrabold tracking-[0.14em]">
        COMING SOON
      </div>
      {/* 副标只陈述此刻的事实。不写上线时间，也不写任何形式的承诺——这一屏没有任何依据说那些。 */}
      <div className={`mt-3 text-[13px] tracking-[0.02em] ${fg("text-ink/75", "mid")}`}>
        竞技场尚未开放
      </div>
    </motion.section>
  );
}

export function Arena() {
  const { appearance, builtins } = useAppearance();
  // 这一屏恒定属于竞技场那台游戏，所以游戏 id 是常量而不是拿路由反查一遍：能渲染到这个组件，
  // 「站在竞技场门口」本身就已经是结论了。自选壁纸压过内置图那条优先级仍归 resolveBackground。
  const background = resolveBackground(appearance, builtins, ARENA_GAME_ID);
  return <ArenaScreen background={background} veil={appearance.veil} />;
}
