// 展示型日志控制台：接收 stdout/stderr 行，等宽字体、深墨底纸字做终端感（token 内的深底纸字用法）。
// stderr 用朱红强调（danger #8a2018 在深底上偏暗不可读，故在深底场景改用体系内可读的 accent 朱红）。
// 新行到达自动滚到底；纯展示，不做交互。

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
        "h-full overflow-y-auto rounded-[3px] bg-ink px-4 py-3 font-mono text-[12.5px] leading-[1.65] text-paper-on/85",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {lines.length === 0 ? (
        <div className="text-paper-on/35">暂无输出</div>
      ) : (
        lines.map((line, i) => (
          <div
            key={i}
            className={[
              "whitespace-pre-wrap break-words",
              line.stream === "stderr" ? "text-accent" : "",
            ].join(" ")}
          >
            {line.text}
          </div>
        ))
      )}
    </div>
  );
}
