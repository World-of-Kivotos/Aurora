// 唯二按钮：primary（墨底纸字，hover 转朱红）/ secondary（发丝描边）。禁止第三种。
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

const base =
  "inline-flex cursor-pointer items-center justify-center gap-2 rounded-[3px] font-extrabold transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-45";

const variants: Record<Variant, string> = {
  primary: "bg-ink px-5 py-[13px] text-[15px] tracking-[0.06em] text-paper-on hover:bg-accent",
  secondary:
    "border border-ink/16 px-4 py-2.5 text-[13px] tracking-[0.04em] text-ink/80 hover:border-ink hover:text-ink",
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
