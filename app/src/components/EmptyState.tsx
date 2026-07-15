// 统一空态：居中 line 图标（muted）+ 一行说明 +（可选）一个次按钮。

import type { ReactNode } from "react";
import { Button } from "./Button";
import styles from "./EmptyState.module.css";

interface EmptyStateProps {
  icon: ReactNode;
  title: string;
  action?: { label: string; onClick: () => void; disabled?: boolean };
}

export function EmptyState({ icon, title, action }: EmptyStateProps) {
  return (
    <div className={styles.empty}>
      <span className={styles.icon}>{icon}</span>
      <p className={styles.text}>{title}</p>
      {action && (
        <Button variant="secondary" onClick={action.onClick} disabled={action.disabled}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
