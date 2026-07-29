// 骨架占位：底块 + 一道横扫的高光。
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
    <div data-skeleton className={`relative overflow-hidden rounded-[3px] bg-ink/[0.055] ${className}`}>
      <motion.div
        className="absolute inset-y-0 w-1/3 bg-gradient-to-r from-transparent via-ink/[0.06] to-transparent"
        initial={{ x: "-100%" }}
        animate={{ x: "400%" }}
        transition={{ duration: 1.5, repeat: Infinity, ease: "linear", delay }}
      />
    </div>
  );
}

/** 资源卡骨架：与 ContentTab 真卡片同结构同高，避免内容落位时跳版。 */
export function ResourceCardSkeleton({ delay = 0 }: { delay?: number }) {
  return (
    <div className="flex items-start gap-3.5 rounded-[3px] border border-ink/8 bg-paper-sink/60 p-3.5">
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

/** 版本格骨架：与 VersionTab 的版本卡同尺寸。 */
export function VersionCardSkeleton({ delay = 0 }: { delay?: number }) {
  return (
    <div className="rounded-[3px] border border-ink/8 bg-paper-sink/60 px-3.5 py-3">
      <Skeleton className="h-[17px] w-20" delay={delay} />
      <Skeleton className="mt-2 h-[11px] w-12" delay={delay + 0.08} />
    </div>
  );
}
