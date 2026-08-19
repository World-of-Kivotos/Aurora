// 弹窗：Portal 到 body 顶层，幕布 + 居中玻璃面板。Esc 关闭、点遮罩关闭（点面板不关）。
// 入场走 framer-motion（减少动效由全局 MotionConfig 降级）；打开时锁 body 滚动（可选）。
// role=dialog + aria-modal，打开时焦点移入面板、关闭时归还先前焦点，保证键盘可达。

import { useEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { springs } from "../lib/motion";
import { WinCloseIcon } from "./icons";

// 宽度档位：md 是历史默认（表单类窄弹窗），lg/xl 供矩阵、列表这类需要横向铺开的内容用。
type ModalSize = "md" | "lg" | "xl";

const sizeClass: Record<ModalSize, string> = {
  md: "max-w-lg",
  lg: "max-w-3xl",
  xl: "max-w-5xl",
};

interface ModalProps {
  open: boolean;
  onClose: () => void;
  title?: string;
  children: ReactNode;
  footer?: ReactNode;
  lockScroll?: boolean;
  size?: ModalSize;
}

export function Modal({
  open,
  onClose,
  title,
  children,
  footer,
  lockScroll = true,
  size = "md",
}: ModalProps) {
  const panelRef = useRef<HTMLDivElement>(null);
  const prevFocus = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    prevFocus.current = document.activeElement as HTMLElement | null;
    // 打开后把焦点移入面板，便于键盘操作与屏幕阅读器落点。
    panelRef.current?.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);

    const prevOverflow = lockScroll ? document.body.style.overflow : "";
    if (lockScroll) document.body.style.overflow = "hidden";

    return () => {
      document.removeEventListener("keydown", onKey);
      if (lockScroll) document.body.style.overflow = prevOverflow;
      prevFocus.current?.focus();
    };
  }, [open, onClose, lockScroll]);

  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-50 flex items-center justify-center p-6"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={springs.tap}
        >
          {/*
            幕布用 .surface-veil 而不是单纯铺一层墨：它连背景的饱和度一起抽掉，
            背景图整个退场，弹窗才成为唯一的注意力落点。
            它同时把面板身下的底压到接近纯黑那一端——正好是对比度表里最不利的那一端，
            所以面板取默认档 .surface-panel（ink/75 得 6.28）仍然过线，不需要为弹窗单独加实。
          */}
          <div className="surface-veil absolute inset-0" onClick={onClose} aria-hidden="true" />
          <motion.div
            ref={panelRef}
            role="dialog"
            aria-modal="true"
            aria-label={title}
            tabIndex={-1}
            initial={{ opacity: 0, y: 12, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 12, scale: 0.98 }}
            transition={springs.settle}
            className={`surface-panel relative z-10 flex max-h-[85vh] w-full ${sizeClass[size]} flex-col rounded-panel focus:outline-none`}
          >
            {title && (
              <header className="flex items-center justify-between gap-4 border-b border-ink/12 px-6 py-4">
                <h2 className="text-[18px] font-extrabold tracking-[-0.01em]">{title}</h2>
                <button
                  type="button"
                  onClick={onClose}
                  aria-label="关闭"
                  className="surface-control inline-flex h-7 w-7 items-center justify-center rounded-chip text-ink/75 hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                  <WinCloseIcon size={18} />
                </button>
              </header>
            )}
            <div className="overflow-y-auto px-6 py-5 text-[14px] text-ink/75">{children}</div>
            {footer && (
              <footer className="flex items-center justify-end gap-3 border-t border-ink/12 px-6 py-4">
                {footer}
              </footer>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}
