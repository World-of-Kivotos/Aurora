// 轻量页头：小标题 + 行内副标题 + 可选右侧槽（状态/操作）。全站统一，收敛占用空间。
//
// 背景图铺满整个 app 之后，报头是各页里唯一直接压在照片上的文字块（main 本身不挂材质，
// 卡片各自有底，只有它裸着）。所以它必须自带一档材质，不能再假设身下是纸色——
// 满墨标题压在一张随机照片上，最坏情况直接读不出来。
// 取默认档 .surface-panel（86%）而不是更实的 96%：报头只有一两行、不做连续扫读，
// 96% 那档是买给长列表的 AAA 余量，用在这儿只是白白把背景图挡光。
// 材质自带投影与描边（inset box-shadow，不撑盒子），所以这里不再另加 border/paper-on-photo。

import type { ReactNode } from "react";

interface PageHeaderProps {
  title: string;
  subtitle?: string;
  right?: ReactNode;
}

export function PageHeader({ title, subtitle, right }: PageHeaderProps) {
  return (
    <header className="surface-panel mb-6 flex items-baseline justify-between gap-6 rounded-panel px-5 py-3.5">
      <div className="flex min-w-0 items-baseline gap-4">
        <h1 className="shrink-0 text-[20px] font-extrabold tracking-[-0.01em]">{title}</h1>
        {/* 副标题原为 ink/60，玻璃上任何一档的 ink/60 都过不了正文的 4.5，一律提到 ink/75。 */}
        {subtitle && <span className="truncate text-[12px] text-ink/75">{subtitle}</span>}
      </div>
      {right && <div className="shrink-0 text-right">{right}</div>}
    </header>
  );
}
