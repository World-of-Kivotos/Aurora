// 启动控件：Start -> 竖线从右扫到左「钉住不动」-> 沿草书单笔路径按笔顺「慢慢手写」出 Aurora（进度跟真实启动）
//  -> 进程起 + 停留 2s -> Stop（竖线扫回右）。点 Stop 竖线先左移手势再复位 Start。日志不在此显示（后台存）。
//
// 这颗按钮是启动屏右下角唯一的主操作位，所以它还兼「把游戏装上」的两种形态（install 属性）：
//   absent  —— 游戏没装上，字样从 Start 换成 Download，点下去开装；
//   running —— 安装在途，整颗按钮变成进度条：左上百分比、左下当前在干什么。
// 语义按玩家眼里的下一步走：一颗按下去必然失败的 Start 比没有按钮更糟。
//
// 材质：一律没有。这颗按钮（含进度条形态）是裸字直接压在照片上，字色随 onDark 反相。
// 一块纸压在图上无论做得多透，都还是在图里挖了一块出来——启动屏整版留给图，
// 是字去适应它压着的那片图，而不是拿一块底把图盖住。进度条形态同样不准套玻璃或纸片。
//
// 手写用 SVG stroke-dashoffset 沿单线草书路径（scriptc，见 auroraPath.ts）描绘 = 一笔笔写出来。
// 进度模型：竖线扫左(SWEEP) 后开始写；慢速爬升到 90%(CREEP)；进程未起则停在 90%（真实进度感）；
// 进程起后补满 100%(FINAL)；写满停留 HOLD 再切 Stop。逐帧直接改 DOM，避免 React 重渲染。

import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { SYNC_STAGE_LABEL, syncProgressRatio, type ModpackSyncProgress } from "../lib/modpack-ui";
import { AURORA_H, AURORA_PATH, AURORA_VIEWBOX, AURORA_W } from "./auroraPath";

export type LaunchPhase = "idle" | "launching" | "spawned";

/**
 * 装机形态。回调挂在需要它的那一支里，而不是与 install 平级的可选属性——
 * 平级写法会留下「说了要装、却没给装的入口」这种拼不出来的组合。
 */
export type LaunchInstallState =
  | { kind: "absent"; onInstall: () => void }
  | { kind: "running"; progress: ModpackSyncProgress };

interface LaunchControlProps {
  phase: LaunchPhase;
  disabled?: boolean;
  onStart: () => void;
  onStop: () => void;
  /** 非空即接管这颗按钮：游戏还没装上，或安装正在跑。为空才是启动语义。 */
  install?: LaunchInstallState | null;
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
// Download 比 Start 长三个字身，按 Start 那个盒宽会有半个词落在盒外点不着。
// 盒子右缘不动、只往左长，所以竖线与字样右缘的位置在两种字样间不会跳。
const DOWNLOAD_W = 268;
const PROGRESS_W = 320; // 进度条形态：够放下一行阶段说明而不至于把整版留白吃掉

function easeOut(t: number): number {
  return 1 - Math.pow(1 - t, 3);
}

/** 压在照片上的那个块体字。三种字样共用同一套版位与字色规则，只有可见性各管各的。 */
function BareWord({
  text,
  visible,
  onDark,
}: {
  text: string;
  visible: boolean;
  onDark?: boolean;
}) {
  return (
    <span
      style={{ right: TEXT_RIGHT }}
      className={`absolute inset-y-0 flex items-center text-[46px] leading-none font-extrabold tracking-[-0.02em] ${onDark ? "text-paper-on" : "text-ink"} transition-[color,opacity] duration-200 group-hover:text-accent ${
        visible ? "opacity-100" : "opacity-0"
      }`}
    >
      {text}
    </span>
  );
}

/**
 * 安装进度形态：整颗按钮就是进度条本身。
 *
 * 版位按玩家读它的顺序排：左上百分比（这次还要多久）、中间朱红进度轨、左下当前在干什么。
 * 三行都左对齐，右缘与 Start 的竖规对齐，换形态时右下角这一列不会左右晃。
 *
 * 安装期间没有任何可点的动作，所以这里是 div 不是 button：渲染成禁用按钮等于告诉读屏
 * 「这有个按钮但你不能按」，而事实是此刻这块区域是一份状态播报。
 */
function InstallProgress({
  progress,
  onDark,
}: {
  progress: ModpackSyncProgress;
  onDark: boolean;
}) {
  // 字节优先、文件数兜底；两者都没有说明清单还没解出来，此时任何数字都是编的。
  const determinate =
    (progress.total_bytes !== null && progress.total_bytes > 0) || progress.total_files > 0;
  const percent = Math.round(syncProgressRatio(progress) * 100);
  const activity = progress.current_file
    ? `${SYNC_STAGE_LABEL[progress.stage]} · ${progress.current_file}`
    : SYNC_STAGE_LABEL[progress.stage];
  const fg = onDark ? "text-paper-on" : "text-ink";
  // 轨底不是材质，是一道压淡的同色线：与右侧那根朱红竖规同一套「线」的语言。
  const track = onDark ? "bg-paper-on/25" : "bg-ink/15";

  return (
    <div
      style={{ width: PROGRESS_W, height: 62 }}
      className={`flex flex-col justify-center gap-1.5 ${fg}`}
      role="progressbar"
      aria-label="安装进度"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={determinate ? percent : undefined}
      aria-valuetext={activity}
    >
      <span className="font-mono text-[26px] leading-none font-extrabold tabular-nums">
        {determinate ? `${percent}%` : "--%"}
      </span>

      <span className={`relative h-1 w-full overflow-hidden ${track}`} aria-hidden>
        {determinate ? (
          <span
            className="absolute inset-y-0 left-0 bg-accent transition-[width] duration-300"
            style={{ width: `${percent}%` }}
          />
        ) : (
          // 不确定态：一段自左向右滑过的朱红。清单还没解出来时用运动、而不是一个假的 0%
          // 来表达「在动，但还不知道要多久」——写 0% 是给了一个此刻根本没有的数。
          <motion.span
            className="absolute inset-y-0 left-0 w-1/3 bg-accent"
            animate={{ x: ["-100%", "300%"] }}
            transition={{ duration: 1.15, repeat: Infinity, ease: "easeInOut" }}
          />
        )}
      </span>

      <span className="truncate font-mono text-[11.5px] tracking-[0.02em]" title={activity}>
        {activity}
      </span>
    </div>
  );
}

export function LaunchControl({
  phase,
  disabled,
  onStart,
  onStop,
  install,
  onDark,
}: LaunchControlProps) {
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

  if (install?.kind === "running") {
    return <InstallProgress progress={install.progress} onDark={!!onDark} />;
  }

  const view = install ? "download" : showStop ? "running" : phase === "idle" ? "idle" : "writing";
  // 竖线在左：写字期间 / 点 Stop 的左移手势期间。其余（idle、Download、运行态 Stop）在右。
  const barAtLeft = view === "writing" || stopping;

  const handleClick = () => {
    if (view === "download") {
      // install 在这一支必然是 absent（running 已在上面提前返回），窄化由 view 保证不了，故就地判。
      if (install?.kind === "absent") install.onInstall();
    } else if (view === "idle") {
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
      // 禁用只对启动语义成立：没账户不能启动，但「把游戏装上」从来不需要账户。
      disabled={disabled && view === "idle"}
      aria-label={
        view === "running" ? "结束游戏" : view === "download" ? "安装游戏" : "开始游戏"
      }
      style={{
        width: (view === "download" ? DOWNLOAD_W : W_PX + BAR_GAP) + PAD * 2,
        height: 62,
      }}
      // 不套任何材质：这颗按钮是裸字直接压在照片上。一块纸压在图上无论做得多透，
      // 都还是在图里挖了一块出来——启动屏整版留给图，字自己适应它压着的那片图，
      // 这正是 onDark 与 plateMode 那套判定存在的理由。
      // 按下反馈走与 Button 同一条 whileTap 缩放：全站唯一入口的那颗按钮不该点下去没反应。
      whileTap={{ scale: 0.98 }}
      transition={springs.tap}
      className="group relative inline-flex items-center focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
    >
      {/* Start / Stop / Download：块体字，右对齐。字色随底图明暗反相，判定见 appearance.ts 的 plateMode。
          三个词各自独立淡入淡出而不是共用一个改文案的节点：换词那一刻两个词要交叠着过，
          共用节点会在不透明度还是 1 的时候把字瞬间换掉。 */}
      <BareWord text="Start" visible={view === "idle"} onDark={onDark} />
      <BareWord text="Stop" visible={view === "running"} onDark={onDark} />
      <BareWord text="Download" visible={view === "download"} onDark={onDark} />

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

      {/* 竖线：右(idle/Download/运行) <-> 左(写字/停止手势)，CSS 过渡平移 */}
      <span
        aria-hidden
        style={{ left: barAtLeft ? `${PAD}px` : `calc(100% - ${PAD + 4}px)` }}
        className="absolute top-1/2 h-[42px] w-[4px] -translate-y-1/2 bg-accent transition-[left] duration-[420ms] ease-[cubic-bezier(0.22,0.61,0.24,1)]"
      />
    </motion.button>
  );
}
