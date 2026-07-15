// 自定义下拉（替代原生 <select>）：泛型于选项值类型，纸墨样式的按钮 + 浮层选项。
// 无障碍走 listbox 模式：button(aria-haspopup=listbox) + role=listbox/option，
// 用 aria-activedescendant 指示高亮项（焦点始终留在按钮上，键盘 Up/Down/Enter/Esc 可操作）。
// 浮层入场交给 framer-motion，减少动效由全局 MotionConfig 统一降级。

import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { springs } from "../lib/motion";

interface SelectOption<T extends string> {
  value: T;
  label: string;
}

interface SelectProps<T extends string> {
  value: T;
  onChange: (value: T) => void;
  options: SelectOption<T>[];
  placeholder?: string;
  disabled?: boolean;
  ariaLabel?: string;
}

// 尾逗号 <T extends string,> 让 TSX 把它解析为泛型而非 JSX 标签。
export function Select<T extends string>({
  value,
  onChange,
  options,
  placeholder = "请选择",
  disabled,
  ariaLabel,
}: SelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const rootRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const selectedIndex = options.findIndex((o) => o.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : null;

  // 点外部关闭：mousedown 阶段判定，避免与选项 click 抢先。
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const openList = () => {
    if (disabled) return;
    setHighlight(selectedIndex >= 0 ? selectedIndex : 0);
    setOpen(true);
  };

  const commit = (index: number) => {
    const opt = options[index];
    if (!opt) return;
    onChange(opt.value);
    setOpen(false);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        openList();
      }
      return;
    }
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setHighlight((h) => Math.min(h + 1, options.length - 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        setHighlight((h) => Math.max(h - 1, 0));
        break;
      case "Home":
        e.preventDefault();
        setHighlight(0);
        break;
      case "End":
        e.preventDefault();
        setHighlight(options.length - 1);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        commit(highlight);
        break;
      case "Escape":
        e.preventDefault();
        setOpen(false);
        break;
      case "Tab":
        setOpen(false);
        break;
    }
  };

  return (
    <div ref={rootRef} className="relative inline-block w-full">
      <button
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        aria-activedescendant={open && options[highlight] ? `${listId}-opt-${highlight}` : undefined}
        onClick={() => (open ? setOpen(false) : openList())}
        onKeyDown={onKeyDown}
        className={[
          "flex w-full items-center justify-between gap-3 rounded-[3px] border bg-paper px-3.5 py-2.5",
          "text-[14px] transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
          "disabled:pointer-events-none disabled:opacity-45",
          open ? "border-ink" : "border-ink/16 hover:border-ink/40",
        ].join(" ")}
      >
        <span className={selected ? "text-ink" : "text-ink/45"}>
          {selected ? selected.label : placeholder}
        </span>
        <svg
          width={16}
          height={16}
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth={1.75}
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
          className={["shrink-0 text-ink/45 transition-transform", open ? "rotate-180" : ""].join(" ")}
        >
          <path d="m6 9 6 6 6-6" />
        </svg>
      </button>

      <AnimatePresence>
        {open && (
          <motion.ul
            role="listbox"
            id={listId}
            initial={{ opacity: 0, y: -6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -6 }}
            transition={springs.tap}
            className="absolute z-40 mt-1.5 max-h-64 w-full overflow-y-auto rounded-[3px] border border-ink/16 bg-paper p-1 shadow-[0_8px_24px_-12px_rgba(20,22,26,0.45)]"
          >
            {options.map((opt, i) => {
              const isSelected = opt.value === value;
              const isHigh = i === highlight;
              return (
                <li
                  key={opt.value}
                  id={`${listId}-opt-${i}`}
                  role="option"
                  aria-selected={isSelected}
                  onMouseEnter={() => setHighlight(i)}
                  onClick={() => commit(i)}
                  className={[
                    "flex cursor-pointer items-center justify-between rounded-[2px] px-3 py-2 text-[14px]",
                    isHigh ? "bg-ink text-paper-on" : "text-ink/80",
                  ].join(" ")}
                >
                  <span>{opt.label}</span>
                  {isSelected && (
                    <span className={isHigh ? "text-paper-on" : "text-accent"} aria-hidden="true">
                      ·
                    </span>
                  )}
                </li>
              );
            })}
          </motion.ul>
        )}
      </AnimatePresence>
    </div>
  );
}
