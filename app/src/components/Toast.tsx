// 轻提示：ToastProvider 供全局挂载，useToast() 返回 toast(message, kind?)。
// 右下角栈式排列，约 3.5s 自动消失，也可手动关。
// kind 语义色：error→danger（危险墨点）、success→深墨底纸色字、info→中性。
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

/*
 * kind → 材质 + 文字档。三档刻意不同材质，理由各自独立：
 *
 * info   .surface-liquid —— Toast 是液态白名单里的四个位置之一，中性提示走这一档，
 *        它也是全站唯一会跟着 data-glass 模式变的小件。ink/75 在其上实算 5.73。
 * error  .surface-panel-strong —— 报错的可读性优先于材质统一：96% 让 danger 拿到 7.48
 *        （liquid 上只有 5.10，虽过 AA 但报错不该只靠余量的下沿活着）。
 *        额外保留一圈 danger 描边：材质自带的 ink/9% 边只表达「有个盒子」，
 *        颜色才是「这条是错误」的那个信号，两者职责不同不能互相顶替。
 * success 深墨实底 —— 体系里没有深色玻璃档，自造一档等于绕过对比度预算；
 *        小件用实心深墨反而与任何照片都拉得开（纸色字 16:1）。
 *        它不是 .surface-* 材质，所以投影得自己挂 .paper-on-photo——正是那个类留下的用途。
 */
const kindSurface: Record<ToastKind, string> = {
  info: "surface-liquid text-ink/75",
  success: "bg-ink paper-on-photo text-paper-on",
  error: "surface-panel-strong border border-danger text-danger",
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
                // 投影不再按「是否压在图上」分支：Toast 是浮在整窗最上层的小件，
                // 底下是照片还是纸片，它都实打实高出一层，本来就该有影子。
                // 三档材质各自带影（liquid/panel-strong 焊在类里，深墨底挂 paper-on-photo）。
                className={[
                  "pointer-events-auto flex items-start justify-between gap-3 rounded-panel px-4 py-3 text-[13.5px]",
                  kindSurface[t.kind],
                ].join(" ")}
              >
                <span className="min-w-0 flex-1 break-words">{t.message}</span>
                <button
                  type="button"
                  onClick={() => remove(t.id)}
                  aria-label="关闭提示"
                  // 图标门槛 3:1，玻璃上再按 55% 折一道就贴着下限了，统一提到 70%。
                  className="-mr-1 -mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-chip opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
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
