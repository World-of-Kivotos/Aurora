// 版面下沉块：纸底之上略暗一档的下沉面板 + 发丝描边，去阴影（编辑部靠版面而非投影立层次）。
// 保留 framer-motion 属性透传（variants 等），供页面做入场编排。

import { motion, type HTMLMotionProps } from "framer-motion";
import type { ReactNode } from "react";

type CardProps = Omit<HTMLMotionProps<"div">, "children"> & { children?: ReactNode };

export function Card({ className, children, ...rest }: CardProps) {
  const cls = ["rounded-[3px] border border-ink/9 bg-paper-sink p-[18px]", className]
    .filter(Boolean)
    .join(" ");
  return (
    <motion.div className={cls} {...rest}>
      {children}
    </motion.div>
  );
}
