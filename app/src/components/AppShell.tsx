// 应用外壳：无边框窗口内 = 自定义标题栏 + 主体（目录导航 + 内容区）。
// 内容区按 pathname 作 key，路由切换时 motion 容器重挂载，重放入场 stagger（子级 pageItem 依次上滑）。

import { Outlet, useLocation } from "react-router-dom";
import { motion } from "framer-motion";
import { Titlebar } from "./Titlebar";
import { Sidebar } from "./Sidebar";
import { pageContainer } from "../lib/motion";

export function AppShell() {
  const location = useLocation();
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-paper">
      <Titlebar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="min-w-0 flex-1 overflow-auto px-[46px] pt-[34px] pb-[30px]">
          <motion.div
            key={location.pathname}
            className="flex min-h-full flex-col"
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
