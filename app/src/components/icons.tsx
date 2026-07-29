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

// ---- 下载中心 ----
export const SearchIcon = (p: IconProps) => (
  <Base {...p}>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </Base>
);

export const DownloadIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3v12" />
    <path d="m7 11 5 5 5-5" />
    <path d="M4 20h16" />
  </Base>
);

export const CheckIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="m4 12.5 5 5L20 6.5" />
  </Base>
);

// 整合包：盒中盒，与单体资源（PackageIcon）区分。
export const BoxesIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M3 8.5 8 6l5 2.5L8 11 3 8.5Z" />
    <path d="M11 15.5 16 13l5 2.5L16 18l-5-2.5Z" />
    <path d="M3 8.5v6l5 2.5v-6" />
    <path d="M16 18v3" />
  </Base>
);

// 资源包：画笔/调色，代表贴图外观。
export const PaletteIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 3a9 9 0 1 0 0 18c1.4 0 2-.9 2-1.8 0-1.5-1.4-1.8-1.4-3 0-.9.8-1.7 1.8-1.7H16a5 5 0 0 0 5-5c0-3.6-4-6.5-9-6.5Z" />
    <circle cx="8" cy="9.5" r="1.1" />
    <circle cx="15" cy="8" r="1.1" />
  </Base>
);

// 光影：太阳，代表光照渲染。
export const SunIcon = (p: IconProps) => (
  <Base {...p}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 17.3l1.8 1.8M19.1 4.9l-1.8 1.8M6.7 17.3l-1.8 1.8" />
  </Base>
);

// 游戏版本：立方体网格，代表 MC 本体。
export const CubeIcon = (p: IconProps) => (
  <Base {...p}>
    <path d="M12 2.5 3.5 7v10L12 21.5 20.5 17V7L12 2.5Z" />
    <path d="M3.5 7 12 11.5 20.5 7" />
    <path d="M12 11.5v10" />
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
