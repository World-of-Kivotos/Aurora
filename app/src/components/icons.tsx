// 图标集（内联 SVG，零外部依赖）。
// 线性图标 24 视口、currentColor 描边、圆角端点，跟随父级 color；品牌星/播放三角为实心锐形，贴合编辑部气质。
// 窗口控件（最小化/最大化/关闭）供自定义标题栏使用（20 视口，细描边）。

import type { ReactNode } from "react";

interface IconProps {
  size?: number;
  className?: string;
}

function Base({ size = 20, className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

// 品牌标记：四角星（Aurora 极光意象），实心锐形。
export const SparkleIcon = ({ size = 20, className }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
    <path d="M12 1c1 7 4 10 11 11-7 1-10 4-11 11-1-7-4-10-11-11 7-1 10-4 11-11Z" />
  </svg>
);

// 播放三角：主 CTA 用，实心锐角。
export const PlayIcon = ({ size = 20, className }: IconProps) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" className={className} aria-hidden="true">
    <path d="M7 4.8 18.6 12 7 19.2Z" />
  </svg>
);

// ---- 内容区线性图标（空态 / 错误 / 占位页）----
export const UserIcon = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="8" r="4" />
    <path d="M4 20c0-4 4-6 8-6s8 2 8 6" />
  </Base>
);

export const LayersIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3 3 8l9 5 9-5-9-5Z" />
    <path d="m3 13 9 5 9-5" />
  </Base>
);

export const PackageIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3 3 7.5v9L12 21l9-4.5v-9L12 3Z" />
    <path d="M3 7.5 12 12l9-4.5" />
    <path d="M12 12v9" />
  </Base>
);

export const SettingsIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M4 6h8" />
    <path d="M18 6h2" />
    <circle cx="15" cy="6" r="2.2" />
    <path d="M4 12h2" />
    <path d="M12 12h8" />
    <circle cx="9" cy="12" r="2.2" />
    <path d="M4 18h8" />
    <path d="M18 18h2" />
    <circle cx="15" cy="18" r="2.2" />
  </Base>
);

export const AlertIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3 2 20h20L12 3Z" />
    <path d="M12 10v4" />
    <path d="M12 17h.01" />
  </Base>
);

export const RefreshIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M21 12a9 9 0 1 1-3-6.7" />
    <path d="M21 4v5h-5" />
  </Base>
);

// ---- 自定义标题栏窗口控件（20 视口，细描边）----
function WinBase({ size = 20, className, children }: IconProps & { children: ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const WinMinIcon = (p: IconProps) => (
  <WinBase {...p}>
    <path d="M4 10h12" />
  </WinBase>
);

export const WinMaxIcon = (p: IconProps) => (
  <WinBase {...p}>
    <rect x="5" y="5" width="10" height="10" rx="1" />
  </WinBase>
);

export const WinCloseIcon = (p: IconProps) => (
  <WinBase {...p}>
    <path d="M5.5 5.5 14.5 14.5M14.5 5.5 5.5 14.5" />
  </WinBase>
);
