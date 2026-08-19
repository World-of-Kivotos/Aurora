// 侧栏：品牌标 + 游戏列表 + 三项导航（账户/版本/下载）+ 页脚设置。
// 当前项一道朱红竖规 + 加粗，无编号、无页脚装饰。竖规用 framer-motion 的 layoutId 在切换时平滑滑动。
//
// 游戏列表取代了原先的「主页」导航项：启动屏本来就是「玩哪个游戏」这一件事，
// 与其在游戏行之外再留一个指向同一条路由的文字入口，不如让游戏行自己就是入口。
// 竖规与下面三项共用同一个 layoutId，所以从「World of Kivotos」滑到「版本」是连续的一道，
// 而不是两套各自淡入淡出。
//
// 材质（背景图铺满全站之后的定档，依据见 app.css 的对比度表）：
//   侧栏本体是窗口外壳的一部分，装了壁纸时整块走 .surface-shell —— 全站最透的一档，让背景图透上来；
//   没装壁纸时不挂（与标题栏同一条判据）：那一档纸色压在纯纸底上像素不变，白采一遍背景而已；
//   行级可点物不挂材质：静息无底、悬停浮一层极淡的墨，当前项靠朱红竖规与字重表达，
//   这样侧栏的点按手感与设置页、版本页里的可点行是同一种，而不是各调各的。

import { NavLink } from "react-router-dom";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { SparkleIcon } from "./icons";

interface NavDef {
  to: string;
  label: string;
  end?: boolean;
}

interface GameDef {
  id: string;
  /**
   * 名字拆成刊头与主名两截，是排版决定不是命名：两个条目共享「World of」这半句，
   * 整名并排就是两行开头一模一样、要读到第二行才分得出谁是谁。把共享的那半降成眉标、
   * 让差异那半（Kivotos / Kivotos : Arena）独占大号字，扫一眼就分得开。
   * 无障碍名仍是完整的一句，见 fullName。
   */
  eyebrow: string;
  title: string;
  /** 缺省即尚未上线：渲染成不可点的行，而不是点进去看空页。 */
  to?: string;
  /** 未上线时的说明；已上线的游戏靠竖规与字重表达当前态，不另占一行。 */
  note?: string;
}

const GAMES: GameDef[] = [
  { id: "world-of-kivotos", eyebrow: "World of", title: "Kivotos", to: "/" },
  {
    id: "world-of-kivotos-arena",
    eyebrow: "World of",
    title: "Kivotos : Arena",
    note: "敬请期待",
  },
];

/**
 * 无障碍名。眉标那半句在视觉上被 CSS 转成了全大写，而浏览器算可及名时会把 text-transform
 * 一并算进去（Chromium 如此），读屏于是可能把 WORLD OF 逐字母拼读。显式给一句原大小写的
 * 完整名字，比让它去拼可靠。
 */
function fullName(game: GameDef): string {
  return `${game.eyebrow} ${game.title}`;
}

const TOP: NavDef[] = [
  { to: "/account", label: "账户" },
  { to: "/versions", label: "版本" },
  { to: "/download", label: "下载" },
];

const BOTTOM: NavDef[] = [{ to: "/settings", label: "设置" }];

/**
 * 未选中态的字色。背景图铺满全站之后，判据从纸底换成了玻璃，这一档是解出来的不是挑的：
 * 侧栏行的控件底寄生在外壳上，悬停态是全系统最坏的一格（ink/75 实算 4.53，余量只剩 0.03），
 * 而 ink/60 在同一格只有 3.27，连正文门槛的四分之三都不到。所以未选中态统一 ink/75，
 * 与选中态的差异交给字重和那道朱红竖规去表达，不靠继续调灰——可读区间下面已经没有空间了。
 */
const RESTING_INK = "text-ink/75";

/**
 * 可点行的公共底。
 *
 * 刻意不挂 .surface-control：那一档会给每一行都画出一个可见的方框，四条导航排下来就是
 * 四个并列的框，侧栏本身已经是一块玻璃，框里套框等于把「一列文字」读成「一列按钮」。
 * 侧栏的当前项一直靠那道朱红竖规加字重表达，不靠给每一行发一个底。
 * 静息无底、悬停才浮一层极淡的墨，是换皮前的做法，观感与信息量都更对。
 */
const ROW_BASE =
  "rounded-control transition-colors hover:bg-ink/6 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

function NavRow({ to, label, end }: NavDef) {
  return (
    <NavLink
      to={to}
      end={end}
      className={`group relative flex items-center py-[10px] pr-3 pl-[16px] ${ROW_BASE}`}
    >
      {({ isActive }) => (
        <>
          {isActive && <ActiveRule />}
          <span
            className={[
              "text-[15px] tracking-[0.02em] transition-colors",
              isActive
                ? "font-extrabold text-ink"
                : `font-semibold group-hover:text-ink ${RESTING_INK}`,
            ].join(" ")}
          >
            {label}
          </span>
        </>
      )}
    </NavLink>
  );
}

function ActiveRule() {
  return (
    <motion.span
      layoutId="nav-rule"
      transition={springs.soft}
      className="absolute top-[8px] bottom-[8px] left-0 w-[2px] bg-accent"
    />
  );
}

/**
 * 眉标：两个条目共享的那半句刊头。10px + 0.22em 字距是本项目既有的小标签写法
 * （同 Home 右上角的「状态」），不是为侧栏新造的一档。
 */
function GameEyebrow({ text }: { text: string }) {
  return (
    <span
      className={`text-[10px] leading-none font-bold tracking-[0.22em] uppercase ${RESTING_INK}`}
    >
      {text}
    </span>
  );
}

/** 一台游戏的字组：眉标（弱）压主名（强），未上线的再补一行说明。 */
function GameLockup({ game, active }: { game: GameDef; active: boolean }) {
  return (
    <span className="flex min-w-0 flex-col">
      <GameEyebrow text={game.eyebrow} />
      <span
        className={[
          "mt-[5px] text-[18px] leading-[1.15] tracking-[-0.01em] transition-colors",
          active ? "font-extrabold text-ink" : `font-semibold group-hover:text-ink ${RESTING_INK}`,
        ].join(" ")}
      >
        {game.title}
      </span>
      {game.note && (
        <span className={`mt-[7px] text-[10px] leading-none font-bold tracking-[0.22em] ${RESTING_INK}`}>
          {game.note}
        </span>
      )}
    </span>
  );
}

function GameRow({ game }: { game: GameDef }) {
  // 未上线：整行不可点，也不给控件底——一块带描边的底会读成「能按」，那正是这一行要否掉的误解。
  // 灰度同样不往下压：层级差异一律靠字号与字重，「敬请期待」那行才是真正的状态线索。
  if (!game.to) {
    return (
      <div aria-disabled="true" className="py-[10px] pr-3 pl-[16px]">
        <GameLockup game={game} active={false} />
      </div>
    );
  }

  return (
    <NavLink
      to={game.to}
      end
      // 排版把名字切成了两截, 无障碍名得把它拼回完整的一句, 否则读屏念出来是断的。
      aria-label={fullName(game)}
      // 游戏行与下面四条导航是同一种可点行，手感必须一致，故共用 ROW_BASE。
      // 契约白名单里「允许」这一行用 .surface-liquid（另三处是主 CTA、
      // Toast、分段控件选中页）。允许不等于必须：液态材质自身不带悬停/按下态，换上去就得把
      // ROW_BASE 那套统一手感再手写一遍，而这一行与下面四条导航是同一种可点行，手感必须一致。
      // 所以这里定档 .surface-control，白名单那一格留空是有意的，不是漏做。
      className={`group relative block py-[10px] pr-3 pl-[16px] ${ROW_BASE}`}
    >
      {({ isActive }) => (
        <>
          {isActive && <ActiveRule />}
          <GameLockup game={game} active={isActive} />
        </>
      )}
    </NavLink>
  );
}

export function Sidebar({ onPhoto }: { onPhoto: boolean }) {
  return (
    <nav
      aria-label="主导航"
      className={[
        // 216 而不是原先的 196：主名那档 18px 下「Kivotos : Arena」实测 138px，
        // 196 的文本列只有 128px 装不下，会把它折成两行、把冒号吊在行尾。
        "flex w-[216px] shrink-0 flex-col border-r px-5 pt-6 pb-5 transition-colors",
        // 有图才挂外壳材质。无图时它是 72% 纸色压在纯纸底上，像素与纯纸一致——
        // 也就是说那一帧的 backdrop-filter 采的是一张纯色，模糊出来还是同一个颜色，
        // 纯属白烧一次全表面合成，而侧栏是常驻且满高的元素，这笔开销比标题栏那条更该省。
        // 判据与 Titlebar 同源（appearance.ts 的 photoShows），两处不再各有一套答案。
        onPhoto ? "surface-shell" : "",
        // 有图时把那道竖线隐去：外壳材质的边界本身已经把侧栏与内容区分开了，
        // 再压一道墨线等于把刚抹平的那条缝重新描一遍。
        //
        // 隐去用 border-transparent 而不是撤掉 border-r：外观设置要过一次 IPC 才回来，
        // 首帧 background 必为 null，落到主页时 onPhoto 会由假翻真。若那时边框整条增删，
        // 竖线是硬消失；保留占位只换颜色，配合 transition-colors 就是淡出。
        onPhoto ? "border-transparent" : "border-ink/10",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className="mb-6 flex items-center gap-2.5 pl-[6px]">
        <SparkleIcon size={22} className="text-ink" />
        <span className="text-[21px] leading-none font-extrabold tracking-[-0.02em]">Aurora</span>
      </div>

      <ul aria-label="游戏" className="flex flex-col gap-2">
        {GAMES.map((game) => (
          <li key={game.id}>
            <GameRow game={game} />
          </li>
        ))}
      </ul>

      {/* 游戏与导航之间的分界。用一条 ink/10 的细线而不是留白：两组条目的行高与结构不同，
          只靠间距分不出「这是两类东西」。 */}
      <hr className="my-4 border-0 border-t border-ink/10" />

      <ul className="flex flex-col gap-1">
        {TOP.map((it) => (
          <li key={it.to}>
            <NavRow {...it} />
          </li>
        ))}
      </ul>

      <div className="mt-auto">
        {BOTTOM.map((it) => (
          <NavRow key={it.to} {...it} />
        ))}
      </div>
    </nav>
  );
}
