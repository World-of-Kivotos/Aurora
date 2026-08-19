// 内容容器：默认档玻璃面板。背景图铺满全站之后它多半直接压在照片上，
// 所以必须用自足材质（自带纸色、模糊、描边、投影），不能再假设身下是纯纸底。
// 保留 framer-motion 属性透传（variants 等），供页面做入场编排。

import { motion, type HTMLMotionProps } from "framer-motion";
import type { ReactNode } from "react";

type CardProps = Omit<HTMLMotionProps<"div">, "children"> & {
  children?: ReactNode;
  /**
   * 危险语气：给「删除实例」「清理数据」这类不可逆区块用。
   * 调用方过去写 className="border-danger/40" 是无效的——那条与基类的描边同层同特异性，
   * 按生成顺序判负；换成材质类之后更是连描边宽度都没有了。
   * 这里走 outline：它不参与 box-shadow 的覆盖之争，也不撑大盒子，是唯一能叠在材质描边之上的手段。
   */
  tone?: "danger";
};

export function Card({ className, children, tone, ...rest }: CardProps) {
  const cls = [
    "surface-panel rounded-panel p-[18px]",
    tone === "danger" ? "outline-2 -outline-offset-1 outline-danger/45" : "",
    className,
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <motion.div className={cls} {...rest}>
      {children}
    </motion.div>
  );
}
