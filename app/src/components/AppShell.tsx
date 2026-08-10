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
  const onHome = location.pathname === "/";

  // 外壳挂载即在空闲期预取下载页各 tab 的首屏（后端对清单与搜索都没有缓存，
  // 不预取的话每次进下载页都要现等一次网络往返）。只跑一次，与路由无关。
  useEffect(() => {
    schedulePrefetch();
  }, []);
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-paper">
      <Titlebar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="relative min-w-0 flex-1 overflow-auto px-[46px] pt-[34px] pb-[30px]">
          {/* 背景在 padding 之内、内容之下：铺满整个内容区矩形，而不是缩在留白里。 */}
          {onHome && (
            <PageBackground
              file={appearance.background}
              tint={appearance.tint}
              veil={appearance.veil}
            />
          )}
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
