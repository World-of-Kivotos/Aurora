// 设置页的背景选择器：图库缩略图网格 + 导入 + 柔化。
//
// 缩略图直接用图库里那份 1920 宽的图，只靠 CSS 缩到卡片大小。为它们再存一套缩略图当然更省，
// 但图库通常只有几张、且都在本地磁盘上，先按需要什么写什么。真到几十张再谈缓存。

import { useCallback, useEffect, useState } from "react";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { AlertIcon, DownloadIcon, PaletteIcon } from "./icons";
import { useToast } from "./Toast";
import { useAppearance } from "../lib/appearance-context";
import { canPickFile, libraryBackgroundUrl, MAX_VEIL, pickImageFile } from "../lib/appearance";
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

export function BackgroundPicker() {
  const { toast } = useToast();
  const { appearance, applyAppearance } = useAppearance();
  const [library, setLibrary] = useState<BackgroundEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 滑条要跟手，不能每拖一格都等一次 IPC 往返；本地镜像立即响应，松手才落盘。
  const [veilInput, setVeilInput] = useState(appearance.veil);

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
        <p className="mt-2.5 text-[12px] text-ink/45">
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

      {loading ? (
        <p className="mt-4 text-[12.5px] text-ink/45">正在读取图库…</p>
      ) : library.length === 0 ? (
        <div className="mt-4">
          <EmptyState
            icon={<PaletteIcon />}
            title="图库是空的。选一张图片，它会被复制进启动器目录，之后随 Aurora 文件夹一起搬家。"
          />
        </div>
      ) : (
        <ul className="mt-4 grid grid-cols-[repeat(auto-fill,minmax(168px,1fr))] gap-3">
          {library.map((entry) => {
            const active = entry.file === appearance.background;
            return (
              <li key={entry.file} className="group relative">
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
                    <div className="mt-0.5 font-mono text-[11px] text-ink/50 tabular-nums">
                      {sizeText(entry)}
                    </div>
                  </div>
                </button>

                {active && (
                  <span className="absolute top-1.5 left-1.5 rounded-[2px] bg-accent px-1.5 py-0.5 text-[10px] font-bold tracking-[0.1em] text-paper-on">
                    使用中
                  </span>
                )}

                {/* 删除只在悬停时出现：它是低频且不可逆的操作，常驻会让网格显得处处是按钮。 */}
                <button
                  type="button"
                  onClick={() => doRemove(entry.file)}
                  disabled={busy}
                  aria-label={`删除 ${entry.file}`}
                  className={[
                    "absolute top-1.5 right-1.5 grid h-6 w-6 cursor-pointer place-items-center rounded-[2px]",
                    "bg-paper/90 text-ink/60 opacity-0 transition-opacity",
                    "group-hover:opacity-100 focus-visible:opacity-100",
                    "hover:bg-danger hover:text-paper-on",
                    "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
                  ].join(" ")}
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2} strokeLinecap="round" aria-hidden="true">
                    <path d="M5 5l14 14M19 5L5 19" />
                  </svg>
                </button>
              </li>
            );
          })}
        </ul>
      )}

      {/* 柔化只在有背景时才有意义，没图时藏起来而不是摆一个拖了没反应的滑条。 */}
      {appearance.background && (
        <div className="mt-5 flex items-center gap-4 border-t border-ink/9 pt-4">
          <div className="min-w-0">
            <div className="text-[13.5px] font-bold">柔化</div>
            <div className="mt-0.5 text-[12px] text-ink/55">图太花时压一层纸色</div>
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
