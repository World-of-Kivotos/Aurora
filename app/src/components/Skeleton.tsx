// 骨架占位：下沉底块 + 一道横扫的高光。
// 只在预取未命中（冷启动、切了筛选参数）时出现——命中缓存的路径直接渲染真内容，不该看到骨架。
// 扫光用 motion 的 x 位移而非 CSS keyframes，全局 MotionConfig 的减少动效设置能一并统管。

import { motion } from "framer-motion";

interface SkeletonProps {
  className?: string;
  /** 错开各行的扫光相位，避免整屏骨架像同一块布在闪。 */
  delay?: number;
}

export function Skeleton({ className = "", delay = 0 }: SkeletonProps) {
  return (
    // 底块改用 .surface-sunken：旧的 5.5% 墨洗压在玻璃上几乎看不出来，
    // 而骨架的全部作用就是让人看见「这里在加载」，看不见等于没有。
    // 它是寄生层，必须套在某个自足材质里——直接铺在照片上会没有底，那是调用方的责任。
    <div
      data-skeleton
      className={`surface-sunken relative overflow-hidden rounded-control ${className}`}
    >
      {/*
        扫光走加墨方向而不是提亮方向：底块本身是墨洗，两端（暗图/亮图）上同向变化才一致，
        提亮向的高光在亮图上会整条消失。
        这里局部叠到约 18% 墨，越过了「单元素墨洗不超过 8%」那条纪律——那条约束的立论是
        ink/75 正文的 4.5，而骨架按定义不承载任何文字，约束不适用。
      */}
      <motion.div
        className="absolute inset-y-0 w-1/3 bg-gradient-to-r from-transparent via-ink/10 to-transparent"
        initial={{ x: "-100%" }}
        animate={{ x: "400%" }}
        transition={{ duration: 1.5, repeat: Infinity, ease: "linear", delay }}
      />
    </div>
  );
}

/**
 * 资源卡骨架：与 ContentTab 真卡片同结构同高同材质，避免内容落位时跳版。
 *
 * 材质必须跟着真卡片走（结果卡是 .surface-panel-strong）：骨架的全部价值就是「落位时什么都不动」，
 * 底色差一档，一屏六块会在数据到达的瞬间集体变实一次，比不做骨架还显眼。
 */
export function ResourceCardSkeleton({ delay = 0 }: { delay?: number }) {
  return (
    <div className="surface-panel-strong flex items-start gap-3.5 rounded-panel p-3.5">
      <Skeleton className="h-12 w-12 shrink-0" delay={delay} />
      <div className="min-w-0 flex-1">
        <Skeleton className="h-[15px] w-1/3" delay={delay + 0.05} />
        <Skeleton className="mt-2 h-[11px] w-full" delay={delay + 0.1} />
        <Skeleton className="mt-1.5 h-[11px] w-4/5" delay={delay + 0.15} />
        <Skeleton className="mt-3 h-[13px] w-24" delay={delay + 0.2} />
      </div>
    </div>
  );
}
