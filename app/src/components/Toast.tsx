// 轻提示：ToastProvider 供全局挂载，useToast() 返回 toast(message, kind?)。
// 右下角栈式排列，约 3.5s 自动消失，也可手动关。
// kind 语义色：error→danger（危险墨点）、success→ink（沉稳墨）、info→中性发丝描边。
// 走 Portal 到 body 顶层；进出场用 framer-motion，减少动效由全局 MotionConfig 降级。

import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { springs } from "../lib/motion";
import { WinCloseIcon } from "./icons";

type ToastKind = "info" | "success" | "error";

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

interface ToastApi {
  toast: (message: string, kind?: ToastKind) => void;
}

const ToastContext = createContext<ToastApi | null>(null);
const AUTO_DISMISS_MS = 3500;

// kind → 面板样式（纸墨体系内的三档，仅 error 借用 danger 墨点）。
const kindClass: Record<ToastKind, string> = {
  info: "border-ink/16 bg-paper text-ink/85",
  success: "border-ink bg-ink text-paper-on",
  error: "border-danger bg-paper text-danger",
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const seq = useRef(0);

  const remove = useCallback((id: number) => {
    setItems((list) => list.filter((t) => t.id !== id));
  }, []);

  const toast = useCallback(
    (message: string, kind: ToastKind = "info") => {
      const id = ++seq.current;
      setItems((list) => [...list, { id, message, kind }]);
      window.setTimeout(() => remove(id), AUTO_DISMISS_MS);
    },
    [remove],
  );

  return (
    <ToastContext.Provider value={{ toast }}>
      {children}
      {createPortal(
        <div className="pointer-events-none fixed right-5 bottom-5 z-[60] flex w-[min(92vw,360px)] flex-col items-stretch gap-2.5">
          <AnimatePresence initial={false}>
            {items.map((t) => (
              <motion.div
                key={t.id}
                layout
                role={t.kind === "error" ? "alert" : "status"}
                initial={{ opacity: 0, x: 24, scale: 0.96 }}
                animate={{ opacity: 1, x: 0, scale: 1 }}
                exit={{ opacity: 0, x: 24, scale: 0.96 }}
                transition={springs.settle}
                className={[
                  "pointer-events-auto flex items-start justify-between gap-3 rounded-[3px] border px-4 py-3 text-[13.5px]",
                  kindClass[t.kind],
                ].join(" ")}
              >
                <span className="min-w-0 flex-1 break-words">{t.message}</span>
                <button
                  type="button"
                  onClick={() => remove(t.id)}
                  aria-label="关闭提示"
                  className={[
                    "-mr-1 -mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-[2px] transition-opacity hover:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                    t.kind === "success" ? "opacity-70" : "opacity-55",
                  ].join(" ")}
                >
                  <WinCloseIcon size={14} />
                </button>
              </motion.div>
            ))}
          </AnimatePresence>
        </div>,
        document.body,
      )}
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast 必须在 ToastProvider 内使用");
  return ctx;
}
