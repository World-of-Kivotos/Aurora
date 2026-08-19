// 初次设定：只问一件事——游戏文件放哪、以及要不要把别处已有的 .minecraft 一并接管。
//
// 刻意不做成「选目录 + 登录 + 装版本」的长向导：步骤一多就会被跳过，而跳过之后启动器是空的。
// 账户与装版本各自的入口本来就在明处，用户按需去做即可；这里只解决「装完打开一片空白」。
//
// 玻璃材质在这一屏的边界情况：配置还没落盘，背景图必然不存在，身后是纯纸底而不是照片。
// 所以这里既不铺 PageBackground，也不假设身下有图——版面自己给 bg-paper 兜底。
// 卡片仍然走 .surface-panel 而不是手写一套 border + 背景：这一档材质在没有图时会自然退化成
// 纸色本身，只剩发丝描边，观感成立；等哪天首屏也铺图，同一段代码不改就变成玻璃。
// 配 .surface-nested 摘掉投影，因为身下是纸不是照片——纸对纸没有高差，也就不该有影子。

import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { Button } from "./Button";
import { Skeleton } from "./Skeleton";
import { AlertIcon, CheckIcon, CubeIcon, LayersIcon } from "./icons";
import { springs } from "../lib/motion";
import {
  completeFirstRun,
  discoverGameDirectories,
  getConfig,
  type NamedDirectory,
} from "../lib/ipc";

interface FirstRunWizardProps {
  /** 设定完成后由外层重新载入配置并放行进入主界面。 */
  onDone: () => void;
}

export function FirstRunWizard({ onDone }: FirstRunWizardProps) {
  // 默认游戏目录来自后端：便携模式下就在 exe 旁边，否则在 %LOCALAPPDATA%\Aurora 下。
  const [defaultDir, setDefaultDir] = useState<string | null>(null);
  const [dirInput, setDirInput] = useState("");
  const [found, setFound] = useState<NamedDirectory[] | null>(null);
  const [adopted, setAdopted] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(async () => {
    setError(null);
    try {
      // 两件事互不依赖，一起发：探测要遍历几个固定位置，比读配置慢。
      const [config, discovered] = await Promise.all([getConfig(), discoverGameDirectories()]);
      setDefaultDir(config.game_dir);
      setDirInput(config.game_dir);
      setFound(discovered);
      // 默认全部勾上：扫到就说明玩家真的在用，逐个去勾反而是负担。
      setAdopted(new Set(discovered.map((d) => d.path)));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const toggle = (path: string) => {
    setAdopted((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const submit = async () => {
    const target = dirInput.trim();
    if (!target) {
      setError("游戏目录不能为空");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const extras = (found ?? []).filter((d) => adopted.has(d.path));
      await completeFirstRun(target, extras);
      onDone();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  const loading = defaultDir === null || found === null;

  return (
    <div className="flex min-h-screen items-center justify-center bg-paper px-6 py-10">
      <motion.div
        initial={{ opacity: 0, y: 10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={springs.soft}
        className="surface-panel surface-nested w-full max-w-[608px] rounded-panel p-8"
      >
        <div className="mb-7">
          <div className="text-[11px] font-bold tracking-[0.22em] text-ink/75">初次设定</div>
          <h1 className="mt-2 text-[26px] leading-tight font-extrabold tracking-[-0.01em]">
            游戏文件放在哪
          </h1>
          <p className="mt-2 text-[13px] leading-relaxed text-ink/75">
            版本、存档与 Mod 都会落在这个目录里。之后可以在设置里随时改，也能添加更多文件夹。
          </p>
        </div>

        {error && (
          <div className="mb-5 flex items-start gap-3 rounded-panel border border-danger/35 px-4 py-3">
            <span className="shrink-0 text-danger">
              <AlertIcon size={18} />
            </span>
            <span className="min-w-0 flex-1 text-[13px] break-words text-danger">{error}</span>
          </div>
        )}

        {loading ? (
          <div className="flex flex-col gap-2.5">
            <Skeleton className="h-[52px] w-full" />
            <Skeleton className="h-[76px] w-full" delay={0.08} />
          </div>
        ) : (
          <>
            <label className="block">
              <span className="text-[12px] font-bold text-ink/75">游戏目录</span>
              {/* 输入框走 .surface-sunken：描边与凹陷都焊在材质里，组件不再自画 border，
                  也就没有了「悬停加深一档、聚焦再加深一档」那套只在这一个输入框上成立的手感。
                  聚焦反馈统一交给焦点环——文本框在鼠标点击时同样命中 :focus-visible，不会丢反馈。 */}
              <input
                type="text"
                value={dirInput}
                onChange={(e) => setDirInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") void submit();
                }}
                spellCheck={false}
                className="surface-sunken mt-1.5 h-11 w-full rounded-control px-3.5 font-mono text-[13px] text-ink outline-none focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
              />
              {dirInput.trim() !== defaultDir && (
                <button
                  type="button"
                  onClick={() => setDirInput(defaultDir ?? "")}
                  className="mt-1.5 cursor-pointer text-[11px] text-ink/75 underline-offset-2 transition-colors hover:text-ink hover:underline"
                >
                  用回默认位置
                </button>
              )}
            </label>

            {found.length > 0 && (
              <div className="mt-7">
                <div className="flex items-baseline gap-2">
                  <span className="text-[12px] font-bold text-ink/75">发现了其它启动器的文件夹</span>
                  <span className="font-mono text-[11px] text-ink/75 tabular-nums">
                    {adopted.size}/{found.length}
                  </span>
                </div>
                <p className="mt-1 text-[12px] leading-relaxed text-ink/75">
                  收下之后可以直接切过去玩，里面的存档和 Mod 不会被移动，也不会被改。
                </p>
                <ul className="m-0 mt-3 flex list-none flex-col gap-1.5 p-0">
                  {found.map((d) => {
                    const on = adopted.has(d.path);
                    return (
                      <li key={d.path}>
                        <button
                          type="button"
                          onClick={() => toggle(d.path)}
                          aria-pressed={on}
                          className={[
                            "flex w-full cursor-pointer items-center gap-3 rounded-control px-3 py-2.5 text-left",
                            "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                            // 勾选态是满墨实块，不挂材质：材质类无层级、会盖掉工具类的 bg-ink。
                            on ? "bg-ink text-paper-on transition-colors" : "surface-control",
                          ].join(" ")}
                        >
                          {/* 未勾选的勾不再用 ink/30：那一档只留给 aria-hidden 的纯装饰，
                              而这枚勾承载「收不收这个文件夹」的状态，图标门槛 3:1 至少要 ink/60。 */}
                          <span className={on ? "text-paper-on" : "text-ink/60"}>
                            <CheckIcon size={15} />
                          </span>
                          <span className="min-w-0 flex-1">
                            <span className="block text-[13.5px] leading-tight font-bold">
                              {d.name}
                            </span>
                            <span
                              className={`mt-0.5 block truncate font-mono text-[11px] ${
                                on ? "text-paper-on/55" : "text-ink/75"
                              }`}
                            >
                              {d.path}
                            </span>
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            )}

            {found.length === 0 && (
              <div className="surface-sunken mt-7 flex items-center gap-3 rounded-panel px-4 py-3.5">
                <span className="shrink-0 text-ink/60">
                  <LayersIcon size={18} />
                </span>
                <span className="text-[12.5px] text-ink/75">
                  没有找到其它启动器的文件夹。之后可以在设置里手动添加。
                </span>
              </div>
            )}

            <div className="mt-8 flex items-center gap-3">
              <Button
                variant="primary"
                icon={<CubeIcon size={16} />}
                onClick={() => void submit()}
                disabled={saving}
              >
                {saving ? "正在准备" : "开始使用"}
              </Button>
              <span className="text-[11.5px] text-ink/75">目录不存在会自动创建</span>
            </div>
          </>
        )}
      </motion.div>
    </div>
  );
}
