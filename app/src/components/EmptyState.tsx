// 空态：左对齐留白式（编辑部不居中堆叠）。图标弱化，一行说明，附可选次按钮。
//
// 它是内容片段不是容器：自身不挂任何 surface-* 材质，永远由调用方把它放进
// Card / .surface-panel 之类的自足材质里。自己带底会在卡片里叠出第二层玻璃。

import type { ReactNode } from "react";
import { Button } from "./Button";

interface EmptyStateProps {
  icon: ReactNode;
  title: string;
  action?: { label: string; onClick: () => void; disabled?: boolean };
}

export function EmptyState({ icon, title, action }: EmptyStateProps) {
  return (
    <div className="flex flex-col items-start gap-4 py-4">
      {/* 图标虽是 aria-hidden，仍从 ink/30 提到 ink/60：整块空态就这一个视觉锚点，
          玻璃底上 ink/30 会彻底化掉；ink/60 是图标档的下限（全档 >=3.27）。 */}
      <span className="text-ink/60 [&_svg]:h-8 [&_svg]:w-8">{icon}</span>
      {/* 这行是空态里唯一承载信息的文字，按玻璃上的正文下限走 ink/75。 */}
      <p className="text-[14px] text-ink/75">{title}</p>
      {action && (
        <Button variant="secondary" onClick={action.onClick} disabled={action.disabled}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
