// 唯二按钮：primary（蓝粉渐变填充白字）/ secondary（白底极淡描边深字）。禁止第三种样式。
// 按压走 tap 弹簧。样式全部来自 token，调用方不得再传色值。

import { motion, type HTMLMotionProps } from "framer-motion";
import type { ReactNode } from "react";
import { springs } from "../lib/motion";
import styles from "./Button.module.css";

type Variant = "primary" | "secondary";

// Omit children：HTMLMotionProps 的 children 含 MotionValue，收窄为 ReactNode 才能直接渲染。
interface ButtonProps extends Omit<HTMLMotionProps<"button">, "children"> {
  variant?: Variant;
  icon?: ReactNode;
  children?: ReactNode;
}

export function Button({
  variant = "secondary",
  icon,
  children,
  className,
  ...rest
}: ButtonProps) {
  const cls = [styles.btn, styles[variant], className].filter(Boolean).join(" ");
  return (
    <motion.button
      type="button"
      className={cls}
      whileTap={{ scale: 0.97 }}
      transition={springs.tap}
      {...rest}
    >
      {icon && <span className={styles.icon}>{icon}</span>}
      {children}
    </motion.button>
  );
}
