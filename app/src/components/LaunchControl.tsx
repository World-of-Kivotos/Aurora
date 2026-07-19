// 启动控件：Start -> 竖线从右扫到左「钉住不动」-> 沿草书单笔路径按笔顺「慢慢手写」出 Aurora（进度跟真实启动）
//  -> 进程起 + 停留 2s -> Stop（竖线扫回右）。点 Stop 竖线先左移手势再复位 Start。日志不在此显示（后台存）。
//
// 手写用 SVG stroke-dashoffset 沿单线草书路径（scriptc，见 auroraPath.ts）描绘 = 一笔笔写出来。
// 进度模型：竖线扫左(SWEEP) 后开始写；慢速爬升到 90%(CREEP)；进程未起则停在 90%（真实进度感）；
// 进程起后补满 100%(FINAL)；写满停留 HOLD 再切 Stop。逐帧直接改 DOM，避免 React 重渲染。

import { useCallback, useEffect, useRef, useState } from "react";
import { AURORA_H, AURORA_PATH, AURORA_VIEWBOX, AURORA_W } from "./auroraPath";

export type LaunchPhase = "idle" | "launching" | "spawned";

interface LaunchControlProps {
  phase: LaunchPhase;
  disabled?: boolean;
  onStart: () => void;
  onStop: () => void;
}

const SWEEP_MS = 420; // 竖线右->左
const CREEP_MS = 3200; // 慢速写到 90% 的时长（“慢慢写”）
const FINAL_MS = 460; // 进程起后 90%->100%
const HOLD_MS = 2000; // 写满(进程起)后停留再切 Stop
const STOP_GESTURE_MS = 440; // 点 Stop 竖线左移手势时长

const H_PX = 46; // Aurora 渲染高度（与 Start 同量级，不放大）
const W_PX = Math.round((H_PX * AURORA_W) / AURORA_H);
const BAR_GAP = 22; // 竖线 + 左侧留白

function easeOut(t: number): number {
  return 1 - Math.pow(1 - t, 3);
}

export function LaunchControl({ phase, disabled, onStart, onStop }: LaunchControlProps) {
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
    <button
      type="button"
      onClick={handleClick}
      disabled={disabled && view === "idle"}
      aria-label={view === "running" ? "结束游戏" : "开始游戏"}
      style={{ width: W_PX + BAR_GAP, height: 62 }}
      className="group relative inline-flex items-center focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
    >
      {/* Start / Stop：块体字，右对齐 */}
      <span
        className={`absolute inset-y-0 right-3 flex items-center text-[46px] leading-none font-extrabold tracking-[-0.02em] text-ink transition-[color,opacity] duration-200 group-hover:text-accent ${
          view === "idle" ? "opacity-100" : "opacity-0"
        }`}
      >
        Start
      </span>
      <span
        className={`absolute inset-y-0 right-3 flex items-center text-[46px] leading-none font-extrabold tracking-[-0.02em] text-ink transition-[color,opacity] duration-200 group-hover:text-accent ${
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
        style={{ right: 0 }}
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
        style={{ left: barAtLeft ? "0px" : "calc(100% - 4px)" }}
        className="absolute top-1/2 h-[42px] w-[4px] -translate-y-1/2 bg-accent transition-[left] duration-[420ms] ease-[cubic-bezier(0.22,0.61,0.24,1)]"
      />
    </button>
  );
}
