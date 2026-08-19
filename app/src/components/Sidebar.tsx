// 侧栏：品牌标 + 游戏列表 + 三项导航（账户/版本/下载）+ 页脚设置。
// 当前项一道朱红竖规 + 加粗，无编号、无页脚装饰。竖规用 framer-motion 的 layoutId 在切换时平滑滑动。
//
// 游戏列表取代了原先的「主页」导航项：启动屏本来就是「玩哪个游戏」这一件事，
// 与其在游戏行之外再留一个指向同一条路由的文字入口，不如让游戏行自己就是入口。
// 竖规与下面三项共用同一个 layoutId，所以从「World of Kivotos」滑到「版本」是连续的一道，
// 而不是两套各自淡入淡出。

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

// 未选中态的字色。磨砂 65% 下 ink/55 只有 2.86，连正文门槛的一半余量都没有；ink/80 得 4.82。
// 这一档是磨砂能放这么透的前提，删掉它就必须把 paper-frost 推回 83% 以上。
function restingInk(onPhoto: boolean): string {
  return onPhoto ? "text-ink/80" : "text-ink/60";
}

function NavRow({ to, label, end, onPhoto }: NavDef & { onPhoto: boolean }) {
  return (
    <NavLink
      to={to}
      end={end}
      className="group relative flex items-center rounded-[3px] py-[10px] pr-3 pl-[16px] transition-colors hover:bg-ink/4 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
    >
      {({ isActive }) => (
        <>
          {isActive && <ActiveRule />}
          <span
            className={[
              "text-[15px] tracking-[0.02em] transition-colors",
              isActive
                ? "font-extrabold text-ink"
                : `font-semibold group-hover:text-ink ${restingInk(onPhoto)}`,
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
function GameEyebrow({ text, tone }: { text: string; tone: string }) {
  return (
    <span className={`text-[10px] leading-none font-bold tracking-[0.22em] uppercase ${tone}`}>
      {text}
    </span>
  );
}

/** 一台游戏的字组：眉标（弱）压主名（强），未上线的再补一行说明。 */
function GameLockup({ game, active, onPhoto }: { game: GameDef; active: boolean; onPhoto: boolean }) {
  const tone = restingInk(onPhoto);
  return (
    <span className="flex min-w-0 flex-col">
      <GameEyebrow text={game.eyebrow} tone={tone} />
      <span
        className={[
          "mt-[5px] text-[18px] leading-[1.15] tracking-[-0.01em] transition-colors",
          active ? "font-extrabold text-ink" : `font-semibold group-hover:text-ink ${tone}`,
        ].join(" ")}
      >
        {game.title}
      </span>
      {game.note && (
        <span className={`mt-[7px] text-[10px] leading-none font-bold tracking-[0.22em] ${tone}`}>
          {game.note}
        </span>
      )}
    </span>
  );
}

function GameRow({ game, onPhoto }: { game: GameDef; onPhoto: boolean }) {
  // 未上线：整行不可点。灰度不往下压——本项目可读区间只有 ink/60~100 这一段（见 app.css 的色阶表），
  // 层级差异一律靠字号与字重表达，而不是把字调淡到读不出来；「敬请期待」那行才是真正的状态线索。
  if (!game.to) {
    return (
      <div aria-disabled="true" className="rounded-[3px] py-[10px] pr-3 pl-[16px]">
        <GameLockup game={game} active={false} onPhoto={onPhoto} />
      </div>
    );
  }

  return (
    <NavLink
      to={game.to}
      end
      // 排版把名字切成了两截, 无障碍名得把它拼回完整的一句, 否则读屏念出来是断的。
      aria-label={fullName(game)}
      className="group relative block rounded-[3px] py-[10px] pr-3 pl-[16px] transition-colors hover:bg-ink/4 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent"
    >
      {({ isActive }) => (
        <>
          {isActive && <ActiveRule />}
          <GameLockup game={game} active={isActive} onPhoto={onPhoto} />
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
        // 有图时把那道竖线隐去：磨砂边界本身已经把侧栏与内容区分开了，
        // 再压一道墨线等于把刚抹平的那条缝重新描一遍。
        //
        // 隐去用 border-transparent 而不是撤掉 border-r：外观设置要过一次 IPC 才回来，
        // 首帧 background 必为 null，落到主页时 onPhoto 会由假翻真。若那时边框整条增删，
        // 竖线是硬消失；保留占位只换颜色，配合 transition-colors 就是淡出。
        onPhoto ? "paper-frost border-transparent" : "border-ink/10",
      ].join(" ")}
    >
      <div className="mb-6 flex items-center gap-2.5 pl-[6px]">
        <SparkleIcon size={22} className="text-ink" />
        <span className="text-[21px] leading-none font-extrabold tracking-[-0.02em]">Aurora</span>
      </div>

      <ul aria-label="游戏" className="flex flex-col gap-2">
        {GAMES.map((game) => (
          <li key={game.id}>
            <GameRow game={game} onPhoto={onPhoto} />
          </li>
        ))}
      </ul>

      {/* 游戏与导航之间的分界。用一条 ink/10 的细线而不是留白：两组条目的行高与结构不同，
          只靠间距分不出「这是两类东西」。 */}
      <hr className="my-4 border-0 border-t border-ink/10" />

      <ul className="flex flex-col gap-0.5">
        {TOP.map((it) => (
          <li key={it.to}>
            <NavRow {...it} onPhoto={onPhoto} />
          </li>
        ))}
      </ul>

      <div className="mt-auto">
        {BOTTOM.map((it) => (
          <NavRow key={it.to} {...it} onPhoto={onPhoto} />
        ))}
      </div>
    </nav>
  );
}
