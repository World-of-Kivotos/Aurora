// 轻量自定义标题栏（配合 decorations:false 无边框窗口）。极简：品牌标 + 窗口控件，无期号等装饰。
// data-tauri-drag-region 标记可拖拽区；窗口按钮不在拖拽区内。getCurrentWindow 仅在 handler 内调用。

import type { ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { SparkleIcon, WinMinIcon, WinMaxIcon, WinCloseIcon } from "./icons";

function WinButton({
  label,
  danger,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onClick={onClick}
      className={[
        "grid h-[28px] w-[34px] cursor-pointer place-items-center rounded-[3px] text-ink/50 transition-colors",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
        danger ? "hover:bg-accent hover:text-paper-on" : "hover:bg-ink/6 hover:text-ink",
      ].join(" ")}
    >
      {children}
    </button>
  );
}

export function Titlebar() {
  return (
    <header
      data-tauri-drag-region
      className="flex h-[38px] shrink-0 items-center pr-2 pl-4 select-none"
    >
      <div data-tauri-drag-region className="flex h-full flex-1 items-center gap-2">
        <SparkleIcon size={12} className="text-accent" />
        <span className="text-[10.5px] font-extrabold tracking-[0.3em] text-ink/55">AURORA</span>
      </div>
      <div className="flex items-center gap-0.5">
        <WinButton label="最小化" onClick={() => void getCurrentWindow().minimize()}>
          <WinMinIcon />
        </WinButton>
        <WinButton label="最大化" onClick={() => void getCurrentWindow().toggleMaximize()}>
          <WinMaxIcon />
        </WinButton>
        <WinButton label="关闭" danger onClick={() => void getCurrentWindow().close()}>
          <WinCloseIcon />
        </WinButton>
      </div>
    </header>
  );
}
