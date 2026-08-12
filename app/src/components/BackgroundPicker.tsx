// 设置页的背景选择器：图库缩略图网格 + 导入 + 柔化。
//
// 缩略图直接用图库里那份 1920 宽的图，只靠 CSS 缩到卡片大小。为它们再存一套缩略图当然更省，
// 但图库通常只有几张、且都在本地磁盘上，先按需要什么写什么。真到几十张再谈缓存。

import { useCallback, useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { AlertIcon, DownloadIcon, PaletteIcon } from "./icons";
import { useToast } from "./Toast";
import { useAppearance } from "../lib/appearance-context";
import { canPickFile, libraryBackgroundUrl, MAX_VEIL, pickImageFile } from "../lib/appearance";
import { springs } from "../lib/motion";
import { useMotionPref } from "../lib/motion-pref";
import {
  importBackground,
  listBackgrounds,
  removeBackground,
  setBackground,
  setBackgroundVeil,
  type BackgroundEntry,
} from "../lib/ipc";

function sizeText(entry: BackgroundEntry): string {
  const mb = entry.bytes / 1024 / 1024;
  return `${entry.width}×${entry.height} · ${mb < 0.1 ? "<0.1" : mb.toFixed(1)} MB`;
}

// 长按删除的推进时长。2s 是「够久到能反悔、又不至于让真想删的人等得烦」的那档：
// 误触点一下远不到 2s，覆盖层刚起头就回弹了。
const HOLD_MS = 2000;
// 松手回弹：撤销要立刻有反馈，所以比推进快一个量级。
const HOLD_CANCEL_MS = 200;

/** 删除按钮的叉。长按覆盖层要用同一枚图标再画一遍做遮罩，抽出来免得两处走形。 */
function CrossIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true">
      <path d="M5 5l14 14M19 5L5 19" />
    </svg>
  );
}

export function BackgroundPicker() {
  const { toast } = useToast();
  const { appearance, applyAppearance } = useAppearance();
  // 长按覆盖层是纯 CSS transition，不归全局 MotionConfig 管辖，所以这里要自己读一次偏好。
  const { reduceMotion } = useMotionPref();
  const [library, setLibrary] = useState<BackgroundEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 滑条要跟手，不能每拖一格都等一次 IPC 往返；本地镜像立即响应，松手才落盘。
  const [veilInput, setVeilInput] = useState(appearance.veil);
  // 当前正被按住待确认删除的文件名。用单一文件名而非布尔值，天然保证同一时刻只有一个进度在跑：
  // 手指挪到另一张图的删除按钮上，上一张的进度会随状态切走而回弹清零。
  const [holdFile, setHoldFile] = useState<string | null>(null);
  const holdTimer = useRef<number | null>(null);
  // 指针路径已经自带长按确认，随之而来的那次 click 必须吞掉，否则等于点一下就删。
  const pointerHeld = useRef(false);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setLibrary(await listBackgrounds());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // 外部改动（比如别处清空了背景）要同步回滑条，否则拖过的值会一直显示旧的。
  useEffect(() => setVeilInput(appearance.veil), [appearance.veil]);

  /** 包一层忙碌态与错误展示：这几个操作都是「改一次、拿回最新外观、刷图库」。 */
  const run = useCallback(
    async (action: () => Promise<void>) => {
      setBusy(true);
      setError(null);
      try {
        await action();
      } catch (e) {
        setError(String(e));
        toast(String(e), "error");
      } finally {
        setBusy(false);
      }
    },
    [toast],
  );

  const doImport = () =>
    void run(async () => {
      const path = await pickImageFile();
      if (!path) return;
      applyAppearance(await importBackground(path));
      await reload();
      toast("背景已更换", "success");
    });

  const doSelect = (file: string) =>
    void run(async () => {
      applyAppearance(await setBackground(file));
      await reload();
    });

  const doClear = () =>
    void run(async () => {
      applyAppearance(await setBackground(null));
      await reload();
    });

  const doRemove = (file: string) =>
    void run(async () => {
      applyAppearance(await removeBackground(file));
      await reload();
      toast(`已删除 ${file}`, "success");
    });

  const cancelHold = useCallback(() => {
    if (holdTimer.current !== null) {
      window.clearTimeout(holdTimer.current);
      holdTimer.current = null;
    }
    setHoldFile(null);
  }, []);

  // 卸载时若还有没走完的长按，计时器必须掐掉：否则离开设置页两秒后会凭空删掉一张图。
  useEffect(() => cancelHold, [cancelHold]);

  const startHold = (file: string) => {
    cancelHold();
    pointerHeld.current = true;
    setHoldFile(file);
    holdTimer.current = window.setTimeout(() => {
      holdTimer.current = null;
      setHoldFile(null);
      doRemove(file);
      // 确认窗口对所有人一样长。「减少动态效果」是无障碍偏好，只该关掉动画，
      // 若连带把破坏性操作的确认时长一起压短，等于对最需要容错的那批用户单独退回「点一下就删」。
    }, HOLD_MS);
  };

  const commitVeil = (value: number) =>
    void run(async () => {
      applyAppearance(await setBackgroundVeil(value));
    });

  return (
    <div className="py-[18px] first:pt-0 last:pb-0">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="text-[15px] font-bold">主页背景</div>
          <div className="mt-1 text-[12.5px] text-ink/60">
            图铺在主页内容区，右下角那块信息收进纸片，字始终在纸上。其余页面保持纸面不变。
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2.5">
          {appearance.background && (
            <Button variant="secondary" onClick={doClear} disabled={busy}>
              恢复纯纸面
            </Button>
          )}
          <Button
            variant="primary"
            icon={<DownloadIcon size={16} />}
            onClick={doImport}
            disabled={busy || !canPickFile()}
          >
            选择图片
          </Button>
        </div>
      </div>

      {!canPickFile() && (
        <p className="mt-2.5 text-[12px] text-ink/60">
          浏览器预览模式下无法调用系统文件框，请在安装后的启动器里选图。
        </p>
      )}

      {error && (
        <div className="mt-3 flex items-center gap-2.5 text-[12.5px] text-danger">
          <span className="[&_svg]:h-4 [&_svg]:w-4">
            <AlertIcon />
          </span>
          <span className="min-w-0 flex-1">{error}</span>
        </div>
      )}

      {/* 占位文案只在「从没拿到过图库」时出现。reload() 每次都会置 loading，若增删后整棵 ul 随之卸载，
          AnimatePresence 就没有对象可 diff，退场与补位全部作废（Account 页同理，那边靠 accounts === null 区分）。 */}
      {loading && library.length === 0 ? (
        <p className="mt-4 text-[12.5px] text-ink/60">正在读取图库…</p>
      ) : library.length === 0 ? (
        <div className="mt-4">
          <EmptyState
            icon={<PaletteIcon />}
            title="图库是空的。选一张图片，它会被复制进启动器目录，之后随 Aurora 文件夹一起搬家。"
          />
        </div>
      ) : (
        <ul className="mt-4 grid grid-cols-[repeat(auto-fill,minmax(168px,1fr))] gap-3">
          {/* 导入与删除会改变网格的项数。不做进出场时新卡片凭空闪现、邻居硬跳位，
              看上去像页面出了错；layout 让剩下的卡片平滑补位，缩放淡入淡出交代清楚「哪一张变了」。 */}
          <AnimatePresence initial={false}>
            {library.map((entry) => {
              const active = entry.file === appearance.background;
              const holding = holdFile === entry.file;
              return (
                <motion.li
                  key={entry.file}
                  layout
                  initial={{ opacity: 0, scale: 0.96 }}
                  animate={{ opacity: 1, scale: 1 }}
                  exit={{ opacity: 0, scale: 0.96 }}
                  transition={springs.settle}
                  className="group relative"
                >
                  <button
                    type="button"
                    onClick={() => doSelect(entry.file)}
                    disabled={busy || active}
                    aria-current={active}
                    className={[
                      "block w-full cursor-pointer overflow-hidden rounded-[3px] border text-left transition-colors",
                      "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                      active
                        ? "border-accent"
                        : "border-ink/12 hover:border-ink/40 disabled:cursor-default",
                    ].join(" ")}
                  >
                    <img
                      src={libraryBackgroundUrl(entry.file)}
                      alt=""
                      loading="lazy"
                      className="block h-[94px] w-full bg-paper-sink object-cover"
                    />
                    <div className="bg-paper px-2.5 py-2">
                      <div className="truncate text-[12.5px] font-bold" title={entry.file}>
                        {entry.file}
                      </div>
                      <div className="mt-0.5 font-mono text-[11px] text-ink/60 tabular-nums">
                        {sizeText(entry)}
                      </div>
                    </div>
                  </button>

                  {active && (
                    <span className="absolute top-1.5 left-1.5 rounded-[2px] bg-accent px-1.5 py-0.5 text-[10px] font-bold tracking-[0.1em] text-paper-on">
                      使用中
                    </span>
                  )}

                  {/* 删除只在悬停时出现：它是低频且不可逆的操作，常驻会让网格显得处处是按钮。
                      也正因为不可逆，指针路径改成长按确认：图库删除是全仓库唯一没有确认弹窗的破坏性操作，
                      但为一张缩略图弹一次模态框太重，把「确认」摊进按住的那两秒里，代价只由真想删的人付。 */}
                  <button
                    type="button"
                    onClick={() => {
                      // 只有键盘 Enter/Space 与辅助技术派发的合成 click 会走到这里——指针路径的 click
                      // 紧跟在长按之后，必须吞掉。键盘用户按不出「长按」，硬套会让他们彻底删不掉图，
                      // 所以键盘路径保留一次直接触发，宁可少一道确认也不能少一条可达路径。
                      if (pointerHeld.current) {
                        pointerHeld.current = false;
                        return;
                      }
                      doRemove(entry.file);
                    }}
                    onKeyDown={() => {
                      // 长按走完时按钮会随 busy 立即禁用，那次 click 永远不会来，闸门就滞留在开着的状态。
                      // 键盘激活前先归零，否则用指针删过一次之后，键盘用户的下一次 Enter 会被白吞掉。
                      pointerHeld.current = false;
                    }}
                    // 只认主键：pointerdown 对右键、中键一样会派发，不拦的话按住右键两秒也能删掉图。
                    onPointerDown={(e) => e.button === 0 && startHold(entry.file)}
                    onPointerUp={cancelHold}
                    onPointerLeave={cancelHold}
                    onPointerCancel={cancelHold}
                    disabled={busy}
                    aria-label={`删除 ${entry.file}`}
                    className={[
                      "absolute top-1.5 right-1.5 grid h-6 w-6 cursor-pointer place-items-center rounded-[2px]",
                      "bg-paper/90 text-ink/60 opacity-0 transition-opacity",
                      "group-hover:opacity-100 focus-visible:opacity-100",
                      // 长按期间撤掉 hover 的整块 danger 填充：底色与推进中的覆盖层同为 danger 时，
                      // 进度就完全看不见了，而「看得见还剩多久」正是长按确认的全部意义。
                      holding ? "" : "hover:bg-danger hover:text-paper-on",
                      "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
                    ].join(" ")}
                  >
                    <CrossIcon />

                    {/* 进度覆盖层：danger 填充自左向右推满即确认。图标在这一层用同一条 clip-path
                        再画一遍，红底扫到哪段哪段就翻成纸上色，避免推进途中图标被同色吞掉。 */}
                    <span
                      aria-hidden="true"
                      className="pointer-events-none absolute inset-0 grid place-items-center rounded-[2px] bg-danger text-paper-on transition-[clip-path]"
                      style={{
                        clipPath: holding ? "inset(0 0 0 0)" : "inset(0 100% 0 0)",
                        // 减少动效时只抹掉扫过的过程，红底仍随按住立刻铺满、松手立刻退去：
                        // 按住期间有没有生效必须看得出来，能省的只是那段推进动画本身。
                        transitionDuration: reduceMotion
                          ? "0ms"
                          : `${holding ? HOLD_MS : HOLD_CANCEL_MS}ms`,
                        transitionTimingFunction: holding ? "linear" : "ease-out",
                      }}
                    >
                      <CrossIcon />
                    </span>
                  </button>
                </motion.li>
              );
            })}
          </AnimatePresence>
        </ul>
      )}

      {/* 柔化只在有背景时才有意义，没图时藏起来而不是摆一个拖了没反应的滑条。 */}
      {appearance.background && (
        <div className="mt-5 flex items-center gap-4 border-t border-ink/9 pt-4">
          <div className="min-w-0">
            <div className="text-[13.5px] font-bold">柔化</div>
            <div className="mt-0.5 text-[12px] text-ink/60">图太花时压一层纸色</div>
          </div>
          <input
            type="range"
            className="ink-range ml-auto w-[180px]"
            min={0}
            max={MAX_VEIL}
            step={5}
            value={veilInput}
            aria-label="背景柔化强度"
            disabled={busy}
            onChange={(e) => setVeilInput(Number(e.target.value))}
            onPointerUp={() => commitVeil(veilInput)}
            onKeyUp={() => commitVeil(veilInput)}
          />
          <span className="w-[42px] shrink-0 text-right font-mono text-[12px] text-ink/60 tabular-nums">
            {veilInput}%
          </span>
        </div>
      )}
    </div>
  );
}
