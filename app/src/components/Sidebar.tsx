// 侧栏：品牌标 + 游戏列表 + 两项导航（账户/下载）+ 页脚设置。
// 当前项一道朱红竖规 + 加粗，无编号、无页脚装饰。竖规用 framer-motion 的 layoutId 在切换时平滑滑动。
//
// 游戏列表取代了原先的「主页」导航项：启动屏本来就是「玩哪个游戏」这一件事，
// 与其在游戏行之外再留一个指向同一条路由的文字入口，不如让游戏行自己就是入口。
// 竖规与下面的导航项共用同一个 layoutId，所以从「World of Kivotos」滑到「下载」是连续的一道，
// 而不是两套各自淡入淡出。
//
// 「版本」导航项已随多实例模型一起撤销：Aurora 收敛成 World of Kivotos 专用启动器之后
// 全程只有一个实例，一份「已安装列表」没有第二行可列。进它卷宗页的入口下沉成游戏行的附属行
// （标签「管理」），因为那一页描述的正是上面那一台游戏，不是与账户/下载并列的第三件事。
//
// 材质（背景图铺满全站之后的定档，依据见 app.css 的对比度表）：
//   侧栏本体是窗口外壳的一部分，装了壁纸时整块走 .surface-shell —— 全站最透的一档，让背景图透上来；
//   没装壁纸时不挂（与标题栏同一条判据）：那一档纸色压在纯纸底上像素不变，白采一遍背景而已；
//   行级可点物不挂材质：静息无底、悬停浮一层极淡的墨，当前项靠朱红竖规与字重表达，
//   这样侧栏的点按手感与设置页、卷宗页里的可点行是同一种，而不是各调各的。

import { useEffect, useState, useSyncExternalStore } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { getConfig, listInstalled } from "../lib/ipc";
import { instanceChangeRevision, subscribeInstanceChanged } from "../lib/instance-signal";
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

/** 只有这一台游戏有卷宗页，「管理」行挂在它下面。 */
const MANAGED_GAME_ID = "world-of-kivotos";

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
 * 刻意不挂 .surface-control：那一档会给每一行都画出一个可见的方框，几条导航排下来就是
 * 几个并列的框，侧栏本身已经是一块玻璃，框里套框等于把「一列文字」读成「一列按钮」。
 * 侧栏的当前项一直靠那道朱红竖规加字重表达，不靠给每一行发一个底。
 * 静息无底、悬停才浮一层极淡的墨，是换皮前的做法，观感与信息量都更对。
 */
const ROW_BASE =
  "rounded-control transition-colors hover:bg-ink/6 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

/** 眉标 / 附属行的字号档：10px + 0.22em 字距，全站小标签共用这一档（同 Home 右上角的「状态」）。 */
const CAPTION_TYPE = "text-[10px] leading-none font-bold tracking-[0.22em]";

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

/**
 * 竖规。inset 可调是因为附属行（「管理」）只有导航行的一半高，
 * 沿用 8px 上下内缩会把那道规压成一截短茬，读起来像渲染缺陷而不是当前态。
 */
function ActiveRule({ inset = "top-[8px] bottom-[8px]" }: { inset?: string }) {
  return (
    <motion.span
      layoutId="nav-rule"
      transition={springs.soft}
      className={`absolute left-0 w-[2px] bg-accent ${inset}`}
    />
  );
}

/**
 * 眉标：两个条目共享的那半句刊头。
 */
function GameEyebrow({ text }: { text: string }) {
  return <span className={`${CAPTION_TYPE} uppercase ${RESTING_INK}`}>{text}</span>;
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
      {game.note && <span className={`mt-[7px] ${CAPTION_TYPE} ${RESTING_INK}`}>{game.note}</span>}
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
      // 游戏行与下面的导航是同一种可点行，手感必须一致，故共用 ROW_BASE。
      // 契约白名单里「允许」这一行用 .surface-liquid（另三处是主 CTA、
      // Toast、分段控件选中页）。允许不等于必须：液态材质自身不带悬停/按下态，换上去就得把
      // ROW_BASE 那套统一手感再手写一遍，而这一行与下面的导航是同一种可点行，手感必须一致。
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

/**
 * 卷宗页入口。占的是 Arena 那行「敬请期待」的同一个视觉槽位——游戏主名下方的附属行，
 * 所以它读起来是「这台游戏的一件事」，而不是与账户/下载并列的第三个去处。
 * 竖规左缘与游戏行对齐（同为 left-0 + pl-[16px]），换行时那道规是平移不是跳格。
 */
function ManageRow({ game }: { game: GameDef }) {
  return (
    <NavLink
      to="/instance"
      aria-label={`管理 ${fullName(game)}`}
      className={`group relative mt-[2px] block py-[8px] pr-3 pl-[16px] ${ROW_BASE}`}
    >
      {({ isActive }) => (
        <>
          {isActive && <ActiveRule inset="top-[5px] bottom-[5px]" />}
          <span
            className={[
              CAPTION_TYPE,
              "transition-colors",
              isActive ? "text-ink" : `group-hover:text-ink ${RESTING_INK}`,
            ].join(" ")}
          >
            管理
          </span>
        </>
      )}
    </NavLink>
  );
}

/**
 * 实例是否已就位。判据只认后端 config 的 selected_version，且要求它真的还在已安装列表里——
 * 目录被人手动删掉时配置里那行字仍在，只信配置就会把「管理」指向一页空态。
 *
 * 取数放在侧栏自己这里而不是由外壳传入：外壳只关心「有没有壁纸」，为了一个附属入口
 * 给它加一条数据链路，会让每个渲染侧栏的地方都得先准备好实例状态。
 *
 * 两个重探信号缺一不可。instance-signal 那条是主的：实例的唯一产生途径是在启动屏装完受管
 * 整合包，而那条流程装完并不跳转——玩家原地就能开始玩，只认路由变化的话入口永远长不出来，
 * 所以由写入方装完之后显式广播。pathname 那条是兜底：外部改动（手删版本目录、另一处改配置）
 * 不会经过广播，换页时顺手重探一次比让侧栏一直停在陈旧结论上划算。
 */
function useInstanceReady(): boolean {
  const { pathname } = useLocation();
  const revision = useSyncExternalStore(
    subscribeInstanceChanged,
    instanceChangeRevision,
    instanceChangeRevision,
  );
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void Promise.all([getConfig(), listInstalled()])
      .then(([config, scan]) => {
        if (cancelled) return;
        const id = config.selected_version;
        setReady(!!id && scan.versions.some((v) => v.id === id));
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // 侧栏是窗口外壳的一部分，没有位置摆错误块，所以这里只能按「未就位」渲染。
        // 但不静默：真正的现场由启动屏与卷宗页的错误块负责呈现，这一笔留给控制台对时间线。
        console.error("侧栏实例探测失败", e);
        setReady(false);
      });
    return () => {
      cancelled = true;
    };
  }, [pathname, revision]);

  return ready;
}

/**
 * 纯渲染层。与取数分开，是为了让「管理」入口的出现/隐藏两种形态能被静态渲染断言直接覆盖——
 * 测试环境里没有 DOM，副作用不会跑，只探测一条真实链路就永远只测得到隐藏那一半。
 */
export function SidebarView({
  onPhoto,
  instanceReady,
}: {
  onPhoto: boolean;
  instanceReady: boolean;
}) {
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
            {/* 实例还没装出来时不给这个入口：点进去只有一页「先去装游戏」，
                那句话该由启动屏说，而不是让人先扑一次空。 */}
            {instanceReady && game.id === MANAGED_GAME_ID && <ManageRow game={game} />}
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

export function Sidebar({ onPhoto }: { onPhoto: boolean }) {
  const instanceReady = useInstanceReady();
  return <SidebarView onPhoto={onPhoto} instanceReady={instanceReady} />;
}
