// 轻量页头：小标题 + 行内副标题 + 可选右侧槽（状态/操作）。全站统一，收敛占用空间。

import type { ReactNode } from "react";

interface PageHeaderProps {
  title: string;
  subtitle?: string;
  /** 兼容旧调用，已不渲染。 */
  eyebrow?: string;
  right?: ReactNode;
}

export function PageHeader({ title, subtitle, right }: PageHeaderProps) {
  return (
    <header className="mb-6 flex items-baseline justify-between gap-6">
      <div className="flex min-w-0 items-baseline gap-4">
        <h1 className="shrink-0 text-[20px] font-extrabold tracking-[-0.01em]">{title}</h1>
        {subtitle && <span className="truncate text-[12px] text-ink/35">{subtitle}</span>}
      </div>
      {right && <div className="shrink-0 text-right">{right}</div>}
    </header>
  );
}
