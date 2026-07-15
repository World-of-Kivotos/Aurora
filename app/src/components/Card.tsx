// 唯一卡片样式：纯白 / 圆角 14 / 内边距 20 / 极淡描边 / 柔和阴影。禁止变体。
// 用 motion.div 承载，页面可直接传 variants（如 pageItem）参与入场 stagger。

import { motion, type HTMLMotionProps } from "framer-motion";
import type { ReactNode } from "react";
import styles from "./Card.module.css";

// Omit children：HTMLMotionProps 的 children 含 MotionValue，收窄为 ReactNode 才能直接渲染。
type CardProps = Omit<HTMLMotionProps<"div">, "children"> & { children?: ReactNode };

export function Card({ className, children, ...rest }: CardProps) {
  const cls = [styles.card, className].filter(Boolean).join(" ");
  return (
    <motion.div className={cls} {...rest}>
      {children}
    </motion.div>
  );
}
