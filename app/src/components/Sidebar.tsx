// 麦麦式左侧固定图标导航（64px，不展开）。选中项为白色浮起圆角胶囊：
// 用 framer-motion 的 layoutId 让胶囊在切换时从旧位平滑滑到新位（morph 弹簧）。
// 选中图标切到强调渐变描边；导航栏底色比内容区偏冷灰一档（token surface-nav）。

import type { ReactNode } from "react";
import { NavLink } from "react-router-dom";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import {
  HomeIcon,
  UserIcon,
  LayersIcon,
  PackageIcon,
  SettingsIcon,
  SparkleIcon,
} from "./icons";
import styles from "./Sidebar.module.css";

interface NavItemDef {
  to: string;
  label: string;
  icon: ReactNode;
  end?: boolean;
}

// 顶部主导航；资源/版本为下载类（本轮仅占位路由）。
const TOP: NavItemDef[] = [
  { to: "/", label: "主页", icon: <HomeIcon />, end: true },
  { to: "/account", label: "账户", icon: <UserIcon /> },
  { to: "/versions", label: "版本", icon: <LayersIcon /> },
  { to: "/resources", label: "资源", icon: <PackageIcon /> },
];

// 底部固定：设置。
const BOTTOM: NavItemDef[] = [{ to: "/settings", label: "设置", icon: <SettingsIcon /> }];

function NavItem({ to, label, icon, end }: NavItemDef) {
  return (
    <NavLink to={to} end={end} className={styles.link} title={label} aria-label={label}>
      {({ isActive }) => (
        <>
          {isActive && (
            <motion.span layoutId="nav-pill" className={styles.pill} transition={springs.morph} />
          )}
          <span className={styles.icon} data-active={isActive}>
            {icon}
          </span>
        </>
      )}
    </NavLink>
  );
}

export function Sidebar() {
  return (
    <nav className={styles.rail}>
      <div className={styles.brand} title="Aurora" aria-label="Aurora">
        <SparkleIcon size={24} />
      </div>
      <div className={styles.group}>
        {TOP.map((item) => (
          <NavItem key={item.to} {...item} />
        ))}
      </div>
      <div className={styles.spacer} />
      <div className={styles.group}>
        {BOTTOM.map((item) => (
          <NavItem key={item.to} {...item} />
        ))}
      </div>
    </nav>
  );
}
