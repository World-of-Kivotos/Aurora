// 应用外壳：无边框窗口内 = 自定义标题栏 + 主体（目录导航 + 内容区）。
// 内容区按 pathname 作 key，路由切换时 motion 容器重挂载，重放入场 stagger（子级 pageItem 依次上滑）。

import { useEffect } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { motion } from "framer-motion";
import { Titlebar } from "./Titlebar";
import { Sidebar } from "./Sidebar";
import { PageBackground } from "./PageBackground";
import { pageContainer } from "../lib/motion";
import { schedulePrefetch } from "../lib/prefetch";
import {
  gameScreenOf,
  MAIN_GAME_ID,
  photoShows,
  plateMode,
  resolveBackground,
} from "../lib/appearance";
import { useAppearance } from "../lib/appearance-context";

export function AppShell() {
  const location = useLocation();
  const { appearance, builtins } = useAppearance();
  // 现在站在哪台游戏的启动屏上；内页（账户/下载/卷宗/设置）不属于任何一台，为 null。
  const gameScreen = gameScreenOf(location.pathname);
  // 内页沿用主服那张内置图：背景是整个窗口的底，不能因为翻进设置页就把底抽掉。
  // 玩家自选的壁纸压过内置图，且不分游戏——判定全在 resolveBackground 里，这里只负责给出游戏。
  const background = resolveBackground(appearance, builtins, gameScreen ?? MAIN_GAME_ID);
  // 外壳是否压在照片上。标题栏与侧栏分处两棵子树，判定在这里算一次往下传，避免两处规则漂移。
  const showPhoto = photoShows(background);
  // 压暗层只为启动屏右下角那撮纸色裸字服务（主服的启动键组、竞技场的 COMING SOON），
  // 所以它跟着「在不在启动屏」走而不是跟着「有没有图」走：图铺满全站，但内页的文字都落在
  // 材质上，字底色由材质的纸色比例定死、与照片无关，压暗对它们纯属白压暗一角。
  //
  // 判空写在最前面既是给 plate 取值让路（内联判空 TS 才认），也是同一条判据：没图就没有裸字可言。
  const scrim =
    background !== null &&
    gameScreen !== null &&
    plateMode(background.plate, appearance.veil) === "paperOn";

  // 外壳挂载即在空闲期预取下载页各 tab 的首屏（后端对清单与搜索都没有缓存，
  // 不预取的话每次进下载页都要现等一次网络往返）。只跑一次，与路由无关。
  useEffect(() => {
    schedulePrefetch();
  }, []);
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper">
      {/* 图垫在整窗最底层，外壳浮在它上面，且不再按路由开关——全站铺图。
          旧版只铺主页，理由是「下载/版本/设置信息密度高，一张图压在下面只会让人找不着重点」。
          那个顾虑本身没错，被推翻的是它当时唯一可选的解法：那一版内页只有裸纸面可用，
          图铺过去就真没有东西托住密集小字，于是只能靠「不铺」来回避。
          现在可读性改由内容容器自己扛：长列表/表格/日志台用 .surface-panel-strong 托底，
          96% 纸色让 ink/75 实算 7.20，越过 WCAG AAA 的 7:1——比旧版那块裸纸面还稳。
          所以守住可读性的手段是「把内容托起来」，不是「把图弄糊」，更不是把图挡掉。

          外壳这一层刻意不给 main 挂任何材质，两个理由：一、一挂就是玻璃叠玻璃，页面里的面板
          会压在第二层模糊上，可读性与 GPU 双输；二、带 backdrop-filter 的元素会成为后代
          position:fixed 的包含块，给 main 挂材质会把页面内浮层的定位基准悄悄挪到 main 上。 */}
      <PageBackground background={background} veil={appearance.veil} scrim={scrim} />
      {/* 背景是 absolute，静态兄弟节点会被它盖住，得靠定位把层序拉回来。
          标题栏在自己的根元素上带 relative；侧栏则是靠下面这层 flex 行的 relative 一起抬起来，
          删掉那个 relative 侧栏就会沉到图底下——它不是可有可无的布局类。 */}
      <Titlebar onPhoto={showPhoto} />
      <div className="relative flex min-h-0 flex-1">
        <Sidebar onPhoto={showPhoto} />
        {/*
          内容盒。这一层的契约是「高度确定、永不滚动」，全文与白名单在 app.css 第六节，
          动它之前先读那一节。这里只留三条落在这个元素上的理由：

          1. overflow-clip 而不是 overflow-auto。外壳一旦可滚，页面写多高都不会报错，
             滚动条就成了「布局没算过」的遮羞布——这正是要消灭的东西。clip 连编程滚动
             都不给（hidden 会被 Element.focus() 悄悄滚走一截且再也滚不回来，没有滚动条可用）。
             开发期这一条被 app.css 的溢出告警改回 hidden，那是探测溢出的必要条件，见那一节。
          2. flex flex-col + 下面那层 min-h-0 flex-1：把「窗口高度 - 标题栏 - 上下内边距」
             这个确定高度原样交给页面，页面才可能在内部划出一块定高的滚动区。
          3. min-h-0 是本次最容易翻车的一条，页面侧同样适用：flex 子项的 min-height 默认是
             auto（不是 0），子项因此不肯缩到内容高度以下。内部滚动区若不置 min-h-0，
             它会把这条 flex 链一路撑高到内容的真实高度，超出的部分顶回外壳——
             滚动条于是又长在外层，而里层那块 overflow-y-auto 永远滚不起来。
        */}
        <main
          data-app-content
          className="relative flex min-w-0 flex-1 flex-col overflow-clip px-[46px] pt-[34px] pb-[30px]"
        >
          <motion.div
            key={location.pathname}
            className="relative flex min-h-0 flex-1 flex-col"
            variants={pageContainer}
            initial="hidden"
            animate="show"
          >
            <Outlet />
          </motion.div>
        </main>
      </div>
    </div>
  );
}
