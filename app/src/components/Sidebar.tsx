// 侧栏：品牌标 + 游戏列表 + 两项导航（账户/下载）+ 页脚设置。
// 当前项一道朱红竖规 + 加粗，无编号、无页脚装饰。竖规用 framer-motion 的 layoutId 在切换时平滑滑动。
//
// 游戏列表取代了原先的「主页」导航项：启动屏本来就是「玩哪个游戏」这一件事，
// 与其在游戏行之外再留一个指向同一条路由的文字入口，不如让游戏行自己就是入口。
// 竖规与下面的导航项共用同一个 layoutId，所以从「World of Kivotos」滑到「下载」是连续的一道，
// 而不是两套各自淡入淡出。
//
// 「版本」导航项已随多实例模型一起撤销：Aurora 收敛成 World of Kivotos 专用启动器之后
// 全程只有一个实例，一份「已安装列表」没有第二行可列。进它卷宗页的入口收进游戏行内部右侧的
// 一枚小箭头（单独可点），因为那一页描述的正是这一台游戏，不是与账户/下载并列的第三件事；
// 用箭头而不是另起一行，是因为「这一行的更多」本来就该长在这一行上。
//
// 材质（背景图铺满全站之后的定档，依据见 app.css 的对比度表）：
//   侧栏本体是窗口外壳的一部分，装了壁纸时整块走 .surface-shell —— 全站最透的一档，让背景图透上来；
//   没装壁纸时不挂（与标题栏同一条判据）：那一档纸色压在纯纸底上像素不变，白采一遍背景而已；
//   导航行不挂材质：静息无底、悬停浮一层极淡的墨，当前项靠朱红竖规与字重表达，
//   这样侧栏的点按手感与设置页、卷宗页里的可点行是同一种，而不是各调各的；
//   唯一的例外是可点的那条游戏行——它是契约白名单里四处小件之一，走 .surface-liquid，
//   并在液态模式下再叠一层真折射透镜（LiquidGlass）。材质与透镜是两件事：纸色归材质类，
//   圆角归 rounded-control，透镜只出折射，三者各管一段，谁都不越界。

import { useEffect, useState, useSyncExternalStore } from "react";
import { NavLink, useLocation } from "react-router-dom";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { getConfig, listInstalled } from "../lib/ipc";
import { instanceChangeRevision, subscribeInstanceChanged } from "../lib/instance-signal";
import { SparkleIcon } from "./icons";
import { LiquidGlass } from "./LiquidGlass";

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
const ROW_INTERACTION =
  "transition-colors hover:bg-ink/6 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent";

/** 独立成行的可点物：交互反馈之外还要自己承担圆角。游戏行的圆角由外层的行容器统一给。 */
const ROW_BASE = `rounded-control ${ROW_INTERACTION}`;

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
 * 竖规。全站只有一道，靠 layoutId 在当前项之间平滑滑动。
 * 上下各内缩 8px：贴着行的上下缘会与相邻行的规连成一条通天柱，读不出「一行」的边界。
 */
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

/**
 * 右向小箭头。icons.tsx 现有的一组是内容图标与窗口控件，没有任何指向性图形，而那份图标集
 * 不归本文件改，所以就地画一枚最小的 V 形。日后 icons.tsx 收了 ChevronRightIcon，
 * 这个函数应当整个删掉改引它——同一枚箭头不该在两处各有一份。
 * 描边与视口沿用 icons.tsx 的 Base（24 视口、currentColor、圆角端点），换过去时形状不变。
 */
function ChevronRightGlyph({ className }: { className?: string }) {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      <path d="m9.5 5.5 6.5 6.5-6.5 6.5" />
    </svg>
  );
}

/**
 * 折射透镜的开关媒体查询。与 app.css 末尾那段无障碍降级同一条判据——那段把玻璃退成实心纸
 * 靠的正是 backdrop-filter: none，而透镜写的是内联 backdrop-filter，内联优先级更高，
 * 不自己让开就等于把系统的无障碍开关废掉。
 */
const LENS_OPT_OUT = "(prefers-reduced-transparency: reduce), (prefers-contrast: more)";

function subscribeLensPreference(onChange: () => void): () => void {
  const media = window.matchMedia(LENS_OPT_OUT);
  media.addEventListener("change", onChange);
  // 玻璃模式由 AppearanceProvider 写在 documentElement 上（:root[data-glass]），设置页改完
  // 当场生效。侧栏不订阅这条属性就会停在旧结论上，直到别的原因触发重渲才跟上。
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-glass"] });
  return () => {
    media.removeEventListener("change", onChange);
    observer.disconnect();
  };
}

function readLensPreference(): boolean {
  if (typeof document === "undefined" || typeof window === "undefined") return false;
  // frost 档的定义就是「不上液态」。透镜的内联 backdrop-filter 会盖过 .surface-liquid 那一条，
  // 不认这个属性就等于绕开全局开关，在 frost 模式下偷偷开了液态。
  if (document.documentElement.dataset.glass !== "liquid") return false;
  return !window.matchMedia(LENS_OPT_OUT).matches;
}

/** 静态渲染那一帧一律无透镜：问不到 DOM 就没有依据，保守档是安全的那一侧。 */
const lensOff = () => false;

/*
 * 折射档的参数，按这一行的实际尺寸定，不用 lib 的默认值：
 *   默认 strength 26 / bevel 22 是给大块面的。这一行约 176x56，bevel 22 会让上下两条斜面
 *   在中线附近相遇（bevelStops 算出来只剩两成中性区），整行读起来是一根棱镜而不是一块玻璃。
 *   14 / 12 保住 lib 那个「边厚略小于位移强度」的比例（默认是 26:22），只是整体缩到行的尺度上。
 * saturation 显式给 200 而不是留默认的 160：折射档的 backdrop-filter 只有一条 url()，
 *   .surface-liquid 那条 saturate(200%) 会被整个盖掉，补偿量得由滤镜内部的 feColorMatrix
 *   接住（lib 已经把 blur/saturate 都折进滤镜里了），数值对不上就会比 frost 档更灰。
 * blur 留 lib 的默认 4：那是折射档专门定的清晰度，位移本身就是玻璃感的来源，
 *   照抄 CSS 的 10px 会把刚接上的折射糊掉。纸色仍是 80%，对比度预算表不受模糊半径影响。
 */
const LENS_STRENGTH = 14;
const LENS_BEVEL = 12;
const LENS_SATURATION = 200;

/**
 * 游戏行。可点的那一台是一个行容器 + 两个并列的可点元素：主体进启动屏，右侧箭头进卷宗页。
 *
 * 箭头不做成嵌在主体链接里的按钮：可点物套可点物是无效 HTML，浏览器的点击归属没有定论，
 * 读屏也会把两个可及名读成一团。并列是唯一能让「点哪块去哪」既确定又可及的结构。
 *
 * 材质与透镜是两件事，都挂在行容器上：
 *   纸色配比走 .surface-liquid（契约白名单里侧栏游戏行的那一格，对比度预算表已经把 80%
 *   这个数算好了），圆角走 rounded-control，LiquidGlass 只出折射不出纸也不出圆角。
 *   sheen 关掉：内联 boxShadow 会盖掉 .surface-liquid 的整条投影（含液态档的顶缘亮边与
 *   下缘暗线），关了才由 CSS 一处说了算。
 */
function GameRow({
  game,
  onPhoto,
  manageable,
}: {
  game: GameDef;
  onPhoto: boolean;
  manageable: boolean;
}) {
  const lensAllowed = useSyncExternalStore(subscribeLensPreference, readLensPreference, lensOff);

  // 未上线：整行不可点，也不给任何材质——一块带描边的底会读成「能按」，那正是这一行要否掉的误解。
  // 灰度同样不往下压：层级差异一律靠字号与字重，「敬请期待」那行才是真正的状态线索。
  if (!game.to) {
    return (
      <div aria-disabled="true" className="py-[10px] pr-3 pl-[16px]">
        <GameLockup game={game} active={false} />
      </div>
    );
  }

  // overflow-hidden 让两个内层可点物的悬停墨洗跟着行的圆角走；焦点环是 -outline-offset，画在内侧，不受裁切。
  const rowClass = "surface-liquid relative flex items-stretch overflow-hidden rounded-control";

  const body = (
    <>
      <NavLink
        to={game.to}
        end
        // 排版把名字切成了两截, 无障碍名得把它拼回完整的一句, 否则读屏念出来是断的。
        aria-label={fullName(game)}
        className={`group relative flex min-w-0 flex-1 items-center py-[10px] pr-2 pl-[16px] ${ROW_INTERACTION}`}
      >
        {({ isActive }) => (
          <>
            {isActive && <ActiveRule />}
            <GameLockup game={game} active={isActive} />
          </>
        )}
      </NavLink>
      {manageable && (
        <NavLink
          to="/instance"
          // 光秃秃一枚箭头没有可及名。说清它去哪，且带上是哪台游戏——侧栏以后不止一行。
          aria-label={`管理 ${fullName(game)}`}
          // 刻意不设 relative：竖规要落在整行的左缘而不是箭头自己的左缘，
          // 让它的 absolute 一路解析到行容器上，那道规才与主体激活时严丝合缝地重合。
          className={`group flex w-[34px] shrink-0 items-center justify-center ${ROW_INTERACTION}`}
        >
          {({ isActive }) => (
            <>
              {isActive && <ActiveRule />}
              <ChevronRightGlyph
                className={`transition-colors ${isActive ? "text-ink" : `group-hover:text-ink ${RESTING_INK}`}`}
              />
            </>
          )}
        </NavLink>
      )}
    </>
  );

  // 没装壁纸时不上透镜：backdrop 采的是一张纯色，位移一张纯色的结果还是同一个颜色，
  // 白烧三遍全区域采样。与侧栏本体「有图才挂外壳材质」是同一条判据。
  if (!lensAllowed || !onPhoto) return <div className={rowClass}>{body}</div>;

  return (
    <LiquidGlass
      className={rowClass}
      sheen={false}
      strength={LENS_STRENGTH}
      bevel={LENS_BEVEL}
      saturation={LENS_SATURATION}
    >
      {body}
    </LiquidGlass>
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
            {/* 实例还没装出来时不给箭头：点进去只有一页「先去装游戏」，
                那句话该由启动屏说，而不是让人先扑一次空。 */}
            <GameRow
              game={game}
              onPhoto={onPhoto}
              manageable={instanceReady && game.id === MANAGED_GAME_ID}
            />
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
