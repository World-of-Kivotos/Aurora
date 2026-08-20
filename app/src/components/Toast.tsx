// 轻提示：ToastProvider 供全局挂载，useToast() 返回 toast(message, kind?)。
// 右下角栈式排列，约 3.5s 自动消失，也可手动关。
// kind 语义色：error→danger（危险墨点）、success→深墨底纸色字、info→中性。
// 走 Portal 到 body 顶层；进出场用 framer-motion，减少动效由全局 MotionConfig 降级。

import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { springs } from "../lib/motion";
import { useAppearance } from "../lib/appearance-context";
import { LiquidGlass } from "./LiquidGlass";
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
 *        材质类不写在这张表里而是落在下面那层透镜上（见 LiquidGlass 那段注释）：
 *        透镜与纸必须同一个盒子，这里只留文字档。
 * error  .surface-panel-strong —— 报错的可读性优先于材质统一：96% 让 danger 拿到 7.48
 *        （liquid 上只有 5.10，虽过 AA 但报错不该只靠余量的下沿活着）。
 *        额外保留一圈 danger 描边：材质自带的 ink/9% 边只表达「有个盒子」，
 *        颜色才是「这条是错误」的那个信号，两者职责不同不能互相顶替。
 * success 深墨实底 —— 体系里没有深色玻璃档，自造一档等于绕过对比度预算；
 *        小件用实心深墨反而与任何照片都拉得开（纸色字 16:1）。
 *        它不是 .surface-* 材质，所以投影得自己挂 .paper-on-photo——正是那个类留下的用途。
 */
const kindSurface: Record<ToastKind, string> = {
  info: "text-ink/75",
  success: "bg-ink paper-on-photo text-paper-on",
  error: "surface-panel-strong border border-danger text-danger",
};

/*
 * 液态玻璃小件的透镜参数。白名单里的每个小件都该用这一组数，逐字一致：
 * 玻璃的厚度是材料属性而不是尺寸属性，同一种材料在几个小件上给出几种厚度，
 * 读起来就不再是同一种材料——这正是并行改界面时最容易留下的那种不一致。
 *
 * 三条取值依据，都不是拍的：
 *   1. bevel 8 —— bevelStops 会把「边厚 / 边长」夹到 0.5，一旦边厚超过半个边长，
 *      中性区宽度归零，这块玻璃从「有平面的透镜」退化成「整块都是斜面的棱镜」。
 *      小件高度只有 32~44px，库里那个给大面板用的默认值 22 直接触顶，必须调小。
 *   2. strength 10 —— 库里 26/22 的强度边厚比是 1.18，这里按同一比例缩到小件尺度，
 *      边缘最外沿的采样偏移约 5px，落在 8px 的斜面带内。
 *   3. blur/saturation 分两档 —— 组件写的是内联 backdrop-filter，优先级高于 .surface-liquid，
 *      会把类里那条整个盖掉（连 saturate 一起）。所以毛玻璃档必须逐字复刻类里的
 *      blur(14px) saturate(170%)；液态档的 saturate 补到 200%，与 :root[data-glass=liquid]
 *      那条对齐——折射滤镜内部有 feColorMatrix type="saturate" 承接这个数，不会丢。
 *      折射档的 blur 取 4 而不是 10：大模糊会把折射本身糊掉，清晰度是这个效果的一部分。
 */
const LIQUID_LENS = {
  liquid: { mode: "auto", strength: 10, bevel: 8, blur: 4, saturation: 200, sheen: false },
  frost: { mode: "frost", strength: 10, bevel: 8, blur: 14, saturation: 170, sheen: false },
} as const;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const seq = useRef(0);
  // 透镜档跟着全局玻璃模式走。frost 模式下不许出现折射：那一档的契约就是「纯毛玻璃」，
  // 由调用方明确请求 frost，而不是指望组件的能力探测替我们守住产品档位。
  const { appearance } = useAppearance();

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
                  "pointer-events-auto relative flex items-start justify-between gap-3 rounded-panel px-4 py-3 text-[13.5px]",
                  kindSurface[t.kind],
                ].join(" ")}
              >
                {/*
                  info 档的纸与透镜合在这一层，而不是挂在外面那个 motion.div 上。
                  理由是 backdrop-filter 采的是「画在自己下面的东西」：纸若在外层先铺一遍，
                  透镜采到的就是已经糊好的纸，折射的不再是照片，效果等于没接。
                  外层于是只剩定位与进出场动画，材质与折射一起落在这一层。

                  文字对比度不受影响：折射只搬运背景像素，不动纸色的 80% 配比，
                  app.css 那张表算的正是这个配比，ink/75 仍是 5.73。而位移只发生在 8px 的
                  斜面带内，正文从 px-4（16px）起排，整段字都坐在没有位移的中性区上。
                */}
                {t.kind === "info" && (
                  <LiquidGlass
                    {...LIQUID_LENS[appearance.glass]}
                    aria-hidden="true"
                    className="surface-liquid absolute inset-0 rounded-panel"
                  />
                )}
                {/* 透镜是绝对定位的兄弟节点，会盖住普通流里的内容；内容自己定位一次才回到它上面。 */}
                <span className="relative min-w-0 flex-1 break-words">{t.message}</span>
                <button
                  type="button"
                  onClick={() => remove(t.id)}
                  aria-label="关闭提示"
                  // 图标门槛 3:1，玻璃上再按 55% 折一道就贴着下限了，统一提到 70%。
                  className="relative -mr-1 -mt-0.5 inline-flex h-5 w-5 shrink-0 items-center justify-center rounded-chip opacity-70 transition-opacity hover:opacity-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
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
