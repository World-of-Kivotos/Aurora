// 细线描边图标集（内联 SVG，零外部依赖）。
// 统一 24 视口、currentColor 描边、圆角端点；默认跟随父级 color，激活态由使用方切到强调渐变。
// AccentDefs 在 AppShell 挂一次，提供 id="auroraAccent" 的线性渐变，供选中图标 stroke: url(#auroraAccent) 使用。

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

// 强调渐变定义：品牌两色为设计系统常量，集中在此单处硬编码（非页面文件）。
export function AccentDefs() {
  return (
    <svg width="0" height="0" aria-hidden="true" style={{ position: "absolute" }}>
      <defs>
        <linearGradient id="auroraAccent" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" stopColor="#6fa8ff" />
          <stop offset="100%" stopColor="#ff8fc7" />
        </linearGradient>
      </defs>
    </svg>
  );
}

export const HomeIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 9.5 12 3l9 6.5" />
    <path d="M5 21V10.5h14V21" />
    <path d="M9.5 21v-6h5v6" />
  </Base>
);

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

// 设置用推子（sliders）图标，避免复杂齿轮路径出错。
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

// 品牌标记：四角星（Aurora 极光意象）。
export const SparkleIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3.5c.4 3.6 1.9 5.1 5.5 5.5-3.6.4-5.1 1.9-5.5 5.5-.4-3.6-1.9-5.1-5.5-5.5 3.6-.4 5.1-1.9 5.5-5.5Z" />
  </Base>
);

// 播放三角：主 CTA 用，实心以在渐变填充上更醒目。
export const PlayIcon = ({ size = 20, className }: IconProps) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="currentColor"
    className={className}
    aria-hidden="true"
  >
    <path d="M8 5.2v13.6a1 1 0 0 0 1.5.87l11-6.8a1 1 0 0 0 0-1.74l-11-6.8A1 1 0 0 0 8 5.2Z" />
  </svg>
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
