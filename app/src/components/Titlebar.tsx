// 轻量自定义标题栏（配合 decorations:false 无边框窗口）。极简：品牌标 + 窗口控件，无期号等装饰。
// data-tauri-drag-region 标记可拖拽区；窗口按钮不在拖拽区内。getCurrentWindow 仅在 handler 内调用。

import type { ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SparkleIcon, WinMinIcon, WinMaxIcon, WinCloseIcon } from "./icons";

function WinButton({
  label,
  danger,
  onPhoto,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onPhoto: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={[
        "grid h-[28px] w-[34px] cursor-pointer place-items-center rounded-control transition-colors",
        // 磨砂放到 65% 之后 ink/50 只剩 2.57，跌破非文字对比的 3:1；加粗到 ink/70 得 3.93。
        // 这一档是磨砂能放这么透的前提，不是可选的润色。
        onPhoto ? "text-ink/70" : "text-ink/60",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
        danger ? "hover:bg-accent hover:text-paper-on" : "hover:bg-ink/6 hover:text-ink",
      ].join(" ")}
    >
      {children}
    </button>
  );
}

export function Titlebar({ onPhoto }: { onPhoto: boolean }) {
  return (
    <header
      data-tauri-drag-region
      className={[
        "relative flex h-[38px] shrink-0 items-center pr-2 pl-4 select-none",
        // 铺了图就改磨砂：让图透上来而不是拿一条不透明纸带把它切掉。
        // 不做成全透明是因为关闭键 hover 是朱红，压在偏红的图上会直接读不出来。
        onPhoto ? "paper-frost" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div data-tauri-drag-region className="flex h-full flex-1 items-center gap-2">
        <SparkleIcon size={12} className="text-accent" />
        <span
          className={`text-[10.5px] font-extrabold tracking-[0.3em] ${
            onPhoto ? "text-ink/75" : "text-ink/60"
          }`}
        >
          AURORA
        </span>
      </div>
      <div className="flex items-center gap-0.5">
        <WinButton onPhoto={onPhoto} label="最小化" onClick={() => void getCurrentWindow().minimize()}>
          <WinMinIcon />
        </WinButton>
        <WinButton onPhoto={onPhoto} label="最大化" onClick={() => void getCurrentWindow().toggleMaximize()}>
          <WinMaxIcon />
        </WinButton>
        <WinButton onPhoto={onPhoto} label="关闭" danger onClick={() => void getCurrentWindow().close()}>
          <WinCloseIcon />
        </WinButton>
      </div>
    </header>
  );
}
