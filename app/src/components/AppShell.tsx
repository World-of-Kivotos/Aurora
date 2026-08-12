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
import { useAppearance } from "../lib/appearance-context";

export function AppShell() {
  const location = useLocation();
  const { appearance } = useAppearance();
  // 背景只进主页。下载/版本/设置信息密度高，一张图压在下面只会让人找不着重点。
  // 封面用整版图、内页回到纸面，这条分工没变，变的只是图铺到哪儿为止。
  const onHome = location.pathname === "/";
  // 外壳是否该改用磨砂。条件里带 background 判空：没装背景的人不该因为这个功能看到任何变化。
  const showPhoto = onHome && appearance.background !== null;

  // 外壳挂载即在空闲期预取下载页各 tab 的首屏（后端对清单与搜索都没有缓存，
  // 不预取的话每次进下载页都要现等一次网络往返）。只跑一次，与路由无关。
  useEffect(() => {
    schedulePrefetch();
  }, []);
  return (
    <div className="relative flex h-screen flex-col overflow-hidden bg-paper">
      {/* 图垫在整窗最底层，外壳浮在它上面。
          早先图只铺在 main 里，于是侧栏那堵纸墙与标题栏那条纸带留在图外面，两块底硬碰硬。
          现在外壳与内容区底下是同一张图，磨砂让它透上来，缝就不存在了——不是拿渐变去掩饰。 */}
      {onHome && (
        <PageBackground
          file={appearance.background}
          tint={appearance.tint}
          veil={appearance.veil}
        />
      )}
      {/* 外壳一律 relative：背景是 absolute，静态兄弟节点会被它盖住，得靠定位把层序拉回来。 */}
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
