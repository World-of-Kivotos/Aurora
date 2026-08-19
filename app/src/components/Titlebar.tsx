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
        "grid h-[28px] w-[34px] cursor-pointer place-items-center rounded-control",
        // 静息就给底，不再是「悬停才显形」。玻璃标题栏透着照片，无底图标按钮的热区完全读不出来，
        // 照片一花连图标本身都要费眼找；4% 墨洗加一圈 8% 描边把三颗按钮的边界画死，
        // 悬停/按下的手感也由材质统一，不再各写一遍。
        "surface-control",
        // 图标门槛是 3:1。ink/60 在最坏格（控件悬停态压在外壳上）实算 3.27 已达标，
        // 有图时抬到 ink/75（4.53）留一档余量——窗口控件误触代价高，值这一档。
        // 余量档取 ink/75 而不是 ink/70：灰阶表只认 满墨/75/60/30 四档，
        // 而 75 的余量本来就比 70 大，没有任何理由为它单开一档表外值。
        onPhoto ? "text-ink/75" : "text-ink/60",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
        // 关闭键的朱红必须盖过 .surface-control 自带的悬停底色。材质类写在 CSS 分层之外，
        // 而工具类在 @layer utilities 里，无层规则恒定压过有层规则（与特指度无关），
        // 所以这里只能靠 important 抬过去，不是随手加的。
        danger ? "hover:bg-accent! hover:text-paper-on" : "hover:text-ink",
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
        // 外壳材质：72% 纸色 + 大半径模糊，让图透上来而不是拿一条不透明纸带把它切掉。
        // 不做成全透明是因为关闭键悬停是朱红，压在偏红的图上会直接读不出来。
        // 没装壁纸时不挂：底下就是纯纸色，backdrop-filter 采一遍纯色是白烧 GPU。
        onPhoto ? "surface-shell" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div data-tauri-drag-region className="flex h-full flex-1 items-center gap-2">
        <SparkleIcon size={12} className="text-accent" />
        {/* 10.5px 按正文档判，不吃「大字可以 ink/60」那条豁免：外壳 72% 上 ink/75 实算 4.99 过线，
            纸底上 7.56，一档通吃两种底，比原先按有无图分叉少一条分支。 */}
        <span className="text-[10.5px] font-extrabold tracking-[0.3em] text-ink/75">AURORA</span>
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
