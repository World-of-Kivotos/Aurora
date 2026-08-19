// 启动控件：Start -> 竖线从右扫到左「钉住不动」-> 沿草书单笔路径按笔顺「慢慢手写」出 Aurora（进度跟真实启动）
//  -> 进程起 + 停留 2s -> Stop（竖线扫回右）。点 Stop 竖线先左移手势再复位 Start。日志不在此显示（后台存）。
//
// 材质：主 CTA 是 .surface-liquid 的四个白名单落点之一（另三个是侧栏游戏行、Toast、分段控件选中页）。
// 默认的 frost 模式下它就是一档更透的毛玻璃，结构先按毛玻璃做对；<html data-glass="liquid"> 一开，
// 同一个类自己加上受光亮边与斜向高光，这里不需要任何配合代码。别的地方不准再用这个类。
//
// 手写用 SVG stroke-dashoffset 沿单线草书路径（scriptc，见 auroraPath.ts）描绘 = 一笔笔写出来。
// 进度模型：竖线扫左(SWEEP) 后开始写；慢速爬升到 90%(CREEP)；进程未起则停在 90%（真实进度感）；
// 进程起后补满 100%(FINAL)；写满停留 HOLD 再切 Stop。逐帧直接改 DOM，避免 React 重渲染。

import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { AURORA_H, AURORA_PATH, AURORA_VIEWBOX, AURORA_W } from "./auroraPath";

export type LaunchPhase = "idle" | "launching" | "spawned";

interface LaunchControlProps {
  phase: LaunchPhase;
  disabled?: boolean;
  onStart: () => void;
  onStop: () => void;
  /**
   * 这颗按钮此刻是否压在偏暗的图上。为真则改用纸色字。
   * 由 Home 从 plateMode 的判定结果传下来——判定归一处, 这里只负责照做,
   * 免得同一张图在按钮与它旁边那撮信息上判出两种结果。
   */
  onDark?: boolean;
}

const SWEEP_MS = 420; // 竖线右->左
const CREEP_MS = 3200; // 慢速写到 90% 的时长（“慢慢写”）
const FINAL_MS = 460; // 进程起后 90%->100%
const HOLD_MS = 2000; // 写满(进程起)后停留再切 Stop
const STOP_GESTURE_MS = 440; // 点 Stop 竖线左移手势时长

const H_PX = 46; // Aurora 渲染高度（与 Start 同量级，不放大）
const W_PX = Math.round((H_PX * AURORA_W) / AURORA_H);
const BAR_GAP = 22; // 竖线 + 左侧留白
// 撤掉玻璃框之后不再需要内缩：内缩本来是为了让笔画避开 16px 圆角的斜切，
// 没有框就没有圆角可避，留着只会平白把按钮撑宽一圈。
const PAD = 0;
const TEXT_RIGHT = 12; // 字样右缘：让开右侧竖线（4px 宽）再留 8px 气口

function easeOut(t: number): number {
  return 1 - Math.pow(1 - t, 3);
}

export function LaunchControl({ phase, disabled, onStart, onStop, onDark }: LaunchControlProps) {
  const pathRef = useRef<SVGPathElement>(null);
  const rafRef = useRef<number | null>(null);
  const tl = useRef({ start: 0, spawnedAt: 0, completeAt: 0, holdTimer: 0, lastP: 0 });
  const [showStop, setShowStop] = useState(false);
  const [stopping, setStopping] = useState(false);

  const setDraw = useCallback((p: number) => {
    if (pathRef.current) pathRef.current.style.strokeDashoffset = String(1 - p);
  }, []);

  const stopRaf = useCallback(() => {
    if (rafRef.current !== null) cancelAnimationFrame(rafRef.current);
    rafRef.current = null;
  }, []);

  // 进程起：记录时刻，rAF 会在写到 90% 后据此补满。
  useEffect(() => {
    if (phase === "spawned" && tl.current.spawnedAt === 0) {
      tl.current.spawnedAt = performance.now();
    }
  }, [phase]);

  useEffect(() => {
    if (phase === "idle") {
      stopRaf();
      window.clearTimeout(tl.current.holdTimer);
      tl.current = { start: 0, spawnedAt: 0, completeAt: 0, holdTimer: 0, lastP: 0 };
      setShowStop(false);
      setDraw(0); // 收笔隐藏
      return;
    }
    if (phase === "launching") {
      setShowStop(false);
      tl.current = { start: performance.now(), spawnedAt: 0, completeAt: 0, holdTimer: 0, lastP: 0 };
    }
    if (rafRef.current !== null) return; // launching -> spawned 沿用同一条 rAF

    const frame = (now: number) => {
      const t = now - tl.current.start;
      let p: number;
      if (t < SWEEP_MS) {
        p = 0; // 竖线扫左期间不落笔
      } else {
        const wt = t - SWEEP_MS;
        const creep = 0.9 * easeOut(Math.min(1, wt / CREEP_MS));
        if (tl.current.spawnedAt !== 0 && creep >= 0.9) {
          if (tl.current.completeAt === 0) tl.current.completeAt = now;
          p = 0.9 + 0.1 * easeOut(Math.min(1, (now - tl.current.completeAt) / FINAL_MS));
        } else {
          p = creep; // 慢速爬升；进程未起则停在 90%
        }
      }
      tl.current.lastP = p;
      setDraw(p);
      if (p >= 0.999 && tl.current.holdTimer === 0) {
        tl.current.holdTimer = window.setTimeout(() => setShowStop(true), HOLD_MS);
      }
      rafRef.current = requestAnimationFrame(frame);
    };
    rafRef.current = requestAnimationFrame(frame);
  }, [phase, setDraw, stopRaf]);

  useEffect(
    () => () => {
      stopRaf();
      window.clearTimeout(tl.current.holdTimer);
    },
    [stopRaf],
  );

  const view = showStop ? "running" : phase === "idle" ? "idle" : "writing";
  // 竖线在左：写字期间 / 点 Stop 的左移手势期间。其余（idle、运行态 Stop）在右。
  const barAtLeft = view === "writing" || stopping;

  const handleClick = () => {
    if (view === "idle") {
      if (!disabled) onStart();
    } else if (view === "running") {
      setStopping(true);
      window.setTimeout(() => setStopping(false), STOP_GESTURE_MS);
      onStop();
    }
    // writing 期间点击忽略
  };

  return (
    <motion.button
      type="button"
      onClick={handleClick}
      disabled={disabled && view === "idle"}
      aria-label={view === "running" ? "结束游戏" : "开始游戏"}
      style={{ width: W_PX + BAR_GAP + PAD * 2, height: 62 }}
      // 不套任何材质：这颗按钮是裸字直接压在照片上。一块纸压在图上无论做得多透，
      // 都还是在图里挖了一块出来——启动屏整版留给图，字自己适应它压着的那片图，
      // 这正是 onDark 与 plateMode 那套判定存在的理由。
      // 按下反馈走与 Button 同一条 whileTap 缩放：全站唯一入口的那颗按钮不该点下去没反应。
      whileTap={{ scale: 0.98 }}
      transition={springs.tap}
      className="group relative inline-flex items-center focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
    >
      {/* Start / Stop：块体字，右对齐。字色随底图明暗反相，判定见 appearance.ts 的 plateMode。 */}
      <span
        style={{ right: TEXT_RIGHT }}
        className={`absolute inset-y-0 flex items-center text-[46px] leading-none font-extrabold tracking-[-0.02em] ${onDark ? "text-paper-on" : "text-ink"} transition-[color,opacity] duration-200 group-hover:text-accent ${
          view === "idle" ? "opacity-100" : "opacity-0"
        }`}
      >
        Start
      </span>
      <span
        style={{ right: TEXT_RIGHT }}
        className={`absolute inset-y-0 flex items-center text-[46px] leading-none font-extrabold tracking-[-0.02em] ${onDark ? "text-paper-on" : "text-ink"} transition-[color,opacity] duration-200 group-hover:text-accent ${
          view === "running" ? "opacity-100" : "opacity-0"
        }`}
      >
        Stop
      </span>

      {/* Aurora：草书单线，stroke-dashoffset 沿笔顺描出（实体笔迹） */}
      <svg
        aria-hidden
        viewBox={AURORA_VIEWBOX}
        width={W_PX}
        height={H_PX}
        className={`absolute top-1/2 -translate-y-1/2 text-accent transition-opacity duration-300 ${
          view === "writing" ? "opacity-100" : "opacity-0"
        }`}
        style={{ right: PAD }}
      >
        <path
          ref={pathRef}
          d={AURORA_PATH}
          pathLength={1}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
          strokeLinecap="round"
          strokeLinejoin="round"
          style={{ strokeDasharray: 1, strokeDashoffset: 1 }}
        />
      </svg>

      {/* 竖线：右(idle/运行) <-> 左(写字/停止手势)，CSS 过渡平移 */}
      <span
        aria-hidden
        style={{ left: barAtLeft ? `${PAD}px` : `calc(100% - ${PAD + 4}px)` }}
        className="absolute top-1/2 h-[42px] w-[4px] -translate-y-1/2 bg-accent transition-[left] duration-[420ms] ease-[cubic-bezier(0.22,0.61,0.24,1)]"
      />
    </motion.button>
  );
}
