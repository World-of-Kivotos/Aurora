// 应用外壳：左固定导航 + 右内容区（Outlet 渲染当前页）。
// 内容区按 pathname 作 key，路由切换时 motion 容器重挂载，重放入场 stagger（子级 pageItem 依次上滑）。
// AccentDefs 在此挂一次，供全局选中图标引用强调渐变。

import { Outlet, useLocation } from "react-router-dom";
import { motion } from "framer-motion";
import { Sidebar } from "./Sidebar";
import { AccentDefs } from "./icons";
import { pageContainer } from "../lib/motion";
import styles from "./AppShell.module.css";

export function AppShell() {
  const location = useLocation();
  return (
    <div className={styles.shell}>
      <AccentDefs />
      <Sidebar />
      <main className={styles.content}>
        <motion.div
          key={location.pathname}
          className={styles.page}
          variants={pageContainer}
          initial="hidden"
          animate="show"
        >
          <Outlet />
        </motion.div>
      </main>
    </div>
  );
}
