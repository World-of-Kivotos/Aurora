// 空态：左对齐留白式（编辑部不居中堆叠）。图标弱化，一行说明，附可选次按钮。

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
      <span className="text-ink/26 [&_svg]:h-8 [&_svg]:w-8">{icon}</span>
      <p className="text-[14px] text-ink/60">{title}</p>
      {action && (
        <Button variant="secondary" onClick={action.onClick} disabled={action.disabled}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
