// 唯二按钮：primary（满墨底纸字，hover 转朱红）/ secondary（控件底）。禁止第三种。
// 按压走 tap 弹簧。品牌色来自 token，调用方不得再传色值。

import { motion, type HTMLMotionProps } from "framer-motion";
import type { ReactNode } from "react";
import { springs } from "../lib/motion";

type Variant = "primary" | "secondary";

// Omit children：HTMLMotionProps 的 children 含 MotionValue，收窄为 ReactNode 才能直接渲染。
interface ButtonProps extends Omit<HTMLMotionProps<"button">, "children"> {
  variant?: Variant;
  icon?: ReactNode;
  children?: ReactNode;
}

// 过渡不写在这里：secondary 的悬停/按下由 .surface-control 统一驱动，
// 它是无层样式、会覆掉工具类的 transition 简写，两边都写只会让人误以为这里还管用。
const base =
  "inline-flex cursor-pointer items-center justify-center gap-2 rounded-control font-extrabold focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-45";

const variants: Record<Variant, string> = {
  // 主操作刻意不上玻璃：背景图铺满全站后，唯一该被一眼认出的实心块就是它，
  // 半透明会让它跟身下的面板糊成同一层，主次就没了。满墨底上纸色字 16:1，任何图上都成立。
  primary:
    "bg-ink px-5 py-[13px] text-[15px] tracking-[0.06em] text-paper-on transition-colors hover:bg-accent",
  // 描边从 border 换成 .surface-control 的 inset 阴影：盒子不再被描边撑大，
  // 悬停与按下态也一并交给材质，避免 32 个组件各调出一种手感。
  secondary: "surface-control px-4 py-2.5 text-[13px] tracking-[0.04em] text-ink/75 hover:text-ink",
};

export function Button({ variant = "secondary", icon, children, className, ...rest }: ButtonProps) {
  return (
    <motion.button
      type="button"
      className={[base, variants[variant], className].filter(Boolean).join(" ")}
      whileTap={{ scale: 0.98 }}
      transition={springs.tap}
      {...rest}
    >
      {icon && <span className="inline-flex">{icon}</span>}
      {children}
    </motion.button>
  );
}
