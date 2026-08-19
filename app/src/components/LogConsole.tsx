// 展示型日志控制台：接收 stdout/stderr 行，等宽字体、深墨底纸字做终端感。
// 新行到达自动滚到底；纯展示，不做交互。
//
// 为什么它在玻璃体系里保持不透明（这一档是全站唯一的实底面，值得把理由写死）：
//   一、材质表里根本没有深色档。要做「深色玻璃」就得新造一档 ink 半透 + backdrop-filter，
//       而那一档不在 contrast-budget 的预算表里，等于绕过整套对比度纪律凭手感调数。
//   二、日志是全站最需要长时间连续扫读的面。同样的需求在纸色侧的答案是
//       .surface-panel-strong（96%，买 AAA 余量），映到深底上的等价答案就是「完全不透」。
//   三、半透明深底会让纸色字的对比度随身下那张照片浮动：同一段日志滚过图的暗部与亮部
//       读起来深浅不一，而这里恰恰是要逐字符核对堆栈的地方。
// 代价是这块面不透光，所以它只该出现在真正要读日志的场景，不做常驻装饰。
//
// 它不是 .surface-* 材质，投影没有焊进来：直接压在照片上时由调用方补 .paper-on-photo，
// 套在卡片/面板里则按「纸对纸永远无影」保持无影——两种用法都存在，组件自己判不了。

import { useEffect, useRef } from "react";

interface LogLine {
  stream: "stdout" | "stderr";
  text: string;
}

interface LogConsoleProps {
  lines: LogLine[];
  className?: string;
}

export function LogConsole({ lines, className }: LogConsoleProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // 每次行数变化滚到底，跟随最新输出。
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [lines.length]);

  return (
    <div
      ref={scrollRef}
      className={[
        "h-full overflow-y-auto rounded-panel bg-ink px-4 py-3 font-mono text-[12.5px] leading-[1.65] text-paper-on/85",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {lines.length === 0 ? (
        // 原为 paper-on/35，在深墨底上实算只有 3.02，够图标不够正文；这句是要读的字，提到 /60（6.53）。
        <div className="text-paper-on/60">暂无输出</div>
      ) : (
        lines.map((line, i) => (
          // stderr 原来整行染 accent 朱红，但 accent 压在满墨底上实算只有 3.45，
          // 12.5px 的正文按 4.5 判是不及格的。体系里没有第二个能在深底上读的彩色，
          // 所以改成「颜色做记号、正文保持可读」：左侧一道朱红槽线标记错误流，
          // 行文本提到满纸色（12.08）与 stdout 的 /85 拉开层级。
          // 两种行都带同宽的槽线（stdout 透明），保证滚动时文本左边缘不跳动。
          <div
            key={i}
            className={[
              "-ml-2 border-l-2 pl-2 whitespace-pre-wrap break-words",
              line.stream === "stderr" ? "border-accent text-paper-on" : "border-transparent",
            ].join(" ")}
          >
            {line.text}
          </div>
        ))
      )}
    </div>
  );
}
