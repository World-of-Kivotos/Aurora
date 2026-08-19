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
import { photoShows, plateMode } from "../lib/appearance";
import { useAppearance } from "../lib/appearance-context";

export function AppShell() {
  const location = useLocation();
  const { appearance } = useAppearance();
  // 外壳是否压在照片上。判定与 Toast 共用同一个函数，避免两处规则漂移；
  // 其中的 background 判空保证没装背景的人看不到任何变化。
  const showPhoto = photoShows(appearance.background);
  // 压暗层只为启动屏右下角那撮纸色裸字服务, 所以它跟着路由走而不是跟着「有没有图」走:
  // 图现在铺满全站, 但只有启动屏把字裸压在图上, 别的页面文字都落在材质上, 压暗对它们纯属白压暗一角。
  const scrim =
    location.pathname === "/" &&
    showPhoto &&
    plateMode(appearance.plate, appearance.veil) === "paperOn";

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
      <PageBackground
        file={appearance.background}
        tint={appearance.tint}
        veil={appearance.veil}
        scrim={scrim}
      />
      {/* 背景是 absolute，静态兄弟节点会被它盖住，得靠定位把层序拉回来。
          标题栏在自己的根元素上带 relative；侧栏则是靠下面这层 flex 行的 relative 一起抬起来，
          删掉那个 relative 侧栏就会沉到图底下——它不是可有可无的布局类。 */}
      <Titlebar onPhoto={showPhoto} />
      <div className="relative flex min-h-0 flex-1">
        <Sidebar onPhoto={showPhoto} />
        <main className="relative min-w-0 flex-1 overflow-auto px-[46px] pt-[34px] pb-[30px]">
          <motion.div
            key={location.pathname}
            className="relative flex min-h-full flex-col"
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
