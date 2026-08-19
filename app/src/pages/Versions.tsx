// 版本管理：已安装实例列表 + 设为当前启动版本。单独成页，后续在此扩展实例详情（Mod / 存档 / 独立设置 / 诊断）。
// 安装新版本在「下载」页；此页只管「管理已有的」。

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { motion } from "framer-motion";
import { EmptyState } from "../components/EmptyState";
import { Button } from "../components/Button";
import { useToast } from "../components/Toast";
import { AlertIcon, CheckIcon, LayersIcon, PackageIcon, RefreshIcon } from "../components/icons";
import { pageItem, springs } from "../lib/motion";
import {
  getConfig,
  listInstalled,
  listMods,
  updateConfig,
  type InstalledVersionDto,
  type VersionScanDto,
} from "../lib/ipc";

function splitId(id: string) {
  const i = id.indexOf("-");
  return i < 0 ? { base: id, sfx: "" } : { base: id.slice(0, i), sfx: id.slice(i) };
}
function loaderText(v: InstalledVersionDto) {
  const l = v.loaders[0];
  return !l ? "原版" : l.version ? `${l.kind} ${l.version}` : l.kind;
}

/**
 * 每个实例的摘要数字，取不到就留空——它是锦上添花，不该拖垮或阻断列表本身。
 *
 * 这里只放本地磁盘就能算出的数，绝不放需要联网的。曾经把「可更新数」也摆在这：
 * 进一次版本页就对每个实例查一遍更新，而每次更新检查又要按已装 Mod 逐个问平台，
 * 于是 实例数 x 每实例 Mod 数 的请求瞬间打出去，直接被 Modrinth 限流（HTTP 429）。
 * 可更新数改到实例卷宗页，由用户主动进入时才查。
 */
interface InstanceStats {
  mods: number;
}

export function Versions() {
  const { toast } = useToast();
  const navigate = useNavigate();
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<Record<string, InstanceStats>>({});
  // 任一时刻只有一行带徽标，单个 ref 就够。
  const badgeRef = useRef<HTMLSpanElement>(null);
  // 徽标是否走 layoutId 迁移。旧行滚出可视区时不走：内容区是 overflow-auto 的裁剪盒，
  // 此时 layoutId 会让徽标从裁剪边缘窜进来，看到的是一道红色残影而不是「它从那儿移过来」。
  const [migrateBadge, setMigrateBadge] = useState(true);

  const load = useCallback(async () => {
    setError(null);
    try {
      const [sc, cfg] = await Promise.all([listInstalled(), getConfig()]);
      setScan(sc);
      setSelected(cfg.selected_version);
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  // 摘要单独取：listMods 是本地目录扫描、不走网络，并发拿没问题。
  // 依赖键用 NUL 连接——版本 id 可能含空格（如 World of Kivotos 2.0 beta），
  // 用空格或逗号做分隔符在极端命名下会产生歧义。
  // 任一实例失败只影响那一行的数字，不写进 error 也不阻断列表渲染。
  const versionIds = scan?.versions.map((v) => v.id).join("\0") ?? "";
  useEffect(() => {
    if (!versionIds) return;
    let cancelled = false;
    const ids = versionIds.split("\0");
    void Promise.all(
      ids.map(async (id) => {
        const mods = await listMods(id).then((m) => m.length).catch(() => null);
        return [id, mods] as const;
      }),
    ).then((rows) => {
      if (cancelled) return;
      const next: Record<string, InstanceStats> = {};
      for (const [id, mods] of rows) {
        if (mods !== null) next[id] = { mods };
      }
      setStats(next);
    });
    return () => {
      cancelled = true;
    };
  }, [versionIds]);

  const versions = scan?.versions ?? [];
  const broken = scan?.broken ?? [];
  const current = selected && versions.some((v) => v.id === selected) ? selected : (versions[0]?.id ?? null);

  /**
   * 旧徽标此刻是否完整落在滚动视口内。
   *
   * 目标行必然可见（用户刚点了它行内的按钮），所以只需判断来源行。
   * 滚动容器是 AppShell 的 main，用 closest 就近找而不是写死选择器链，
   * 免得外壳布局一动这里就悄悄失效。
   */
  const badgeInView = () => {
    const el = badgeRef.current;
    const scroller = el?.closest("main");
    if (!el || !scroller) return false;
    const badge = el.getBoundingClientRect();
    const box = scroller.getBoundingClientRect();
    return badge.top >= box.top && badge.bottom <= box.bottom;
  };

  const setAsCurrent = async (id: string) => {
    const prev = selected;
    // 与 setSelected 同一批次，迁移决策在挂载新徽标的那次渲染里就已生效。
    setMigrateBadge(badgeInView());
    setSelected(id);
    try {
      await updateConfig({ selectedVersion: id });
      toast("已设为当前启动版本", "success");
    } catch (e) {
      setSelected(prev);
      toast(String(e), "error");
    }
  };

  // 版本 id 可能含空格、中文与 # 等字符，进 hash 路由必须编码，否则路由解析会截断。
  const openDetail = (id: string) => navigate(`/versions/${encodeURIComponent(id)}`);

  return (
    <>
      <motion.div variants={pageItem} className="mb-6 flex items-baseline justify-between">
        <div className="flex items-baseline gap-4">
          <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">版本</h1>
          <span className="text-[12px] text-ink/60">管理已安装的实例，点进查看内容与变更史</span>
        </div>
        <button
          type="button"
          onClick={() => void load()}
          className="inline-flex items-center gap-1.5 text-[12px] font-semibold text-ink/60 transition-colors hover:text-ink [&_svg]:h-3.5 [&_svg]:w-3.5"
        >
          <RefreshIcon />
          刷新
        </button>
      </motion.div>

      {error && (
        <motion.div
          variants={pageItem}
          className="mb-5 flex items-center gap-3 rounded-panel border border-danger/40 px-4 py-3 text-[13px] text-danger"
        >
          <AlertIcon size={18} />
          <span className="flex-1">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void load()}>
            重试
          </Button>
        </motion.div>
      )}

      {versions.length + broken.length === 0 ? (
        <motion.div variants={pageItem}>
          <EmptyState icon={<LayersIcon />} title="还没有安装任何版本，去「下载」装一个" />
        </motion.div>
      ) : (
        <motion.ul variants={pageItem} className="m-0 list-none p-0">
          {versions.map((v) => {
            const s = splitId(v.id);
            const isCur = v.id === current;
            const st = stats[v.id];
            return (
              <li key={v.id} className="border-b border-ink/8 last:border-b-0">
                {/* 整行进详情，行内单独一个控件设为当前——两个动作分开，不再靠同一次点击猜意图。 */}
                <div className="group flex items-center gap-4 transition-colors hover:bg-ink/[0.03]">
                  <button
                    type="button"
                    onClick={() => openDetail(v.id)}
                    className="flex min-w-0 flex-1 cursor-pointer items-center justify-between gap-6 py-[15px] text-left"
                  >
                    <span className="flex min-w-0 items-center gap-3">
                      <span className="truncate text-[21px] font-bold tracking-[-0.01em] tabular-nums">
                        {s.base}
                      </span>
                      {s.sfx && <span className="shrink-0 text-[14px] font-semibold text-ink/60">{s.sfx}</span>}
                      {/* 与侧栏当前项竖规同一套语言：layoutId 让徽标从旧行滑到新行，
                          把「哪个版本会被启动」这个答案的转移变成看得见的过程，而不是瞬移后要自己找。 */}
                      {isCur && (
                        <motion.span
                          ref={badgeRef}
                          layoutId={migrateBadge ? "current-version-badge" : undefined}
                          transition={springs.soft}
                          className="shrink-0 rounded-chip bg-accent/12 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em] text-accent"
                        >
                          当前
                        </motion.span>
                      )}
                    </span>
                    <span className="flex shrink-0 items-center gap-4 text-[12px] text-ink/60">
                      {st && st.mods > 0 && (
                        <span className="inline-flex items-center gap-1">
                          <PackageIcon size={13} />
                          {st.mods}
                        </span>
                      )}
                      <span>{v.mc_version !== v.id ? v.mc_version : ""}</span>
                      <span className="font-mono">{loaderText(v)}</span>
                    </span>
                  </button>
                  {/* 已是当前的行显示静态标记，避免出现一个点了没反应的按钮。
                      槽位定宽：勾标记比「设为当前」窄约 28px，不定宽的话切换时左侧 flex-1 会跟着改宽，
                      新旧两行的元数据列同时横跳，正好盖过徽标迁移那点平滑感。 */}
                  <div className="mr-1 flex w-17 shrink-0 items-center justify-end">
                    {isCur ? (
                      <span
                        aria-hidden="true"
                        className="px-2 text-accent [&_svg]:h-4 [&_svg]:w-4"
                        title="已是当前启动版本"
                      >
                        <CheckIcon />
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={() => void setAsCurrent(v.id)}
                        title="设为当前启动版本"
                        className="cursor-pointer rounded-chip px-2 py-1 text-[11px] font-bold whitespace-nowrap text-ink/0 transition-colors group-hover:text-ink/60 hover:!text-accent focus-visible:text-ink/60 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
                      >
                        设为当前
                      </button>
                    )}
                  </div>
                </div>
              </li>
            );
          })}
          {broken.map((b) => (
            <li key={b.id} className="border-b border-ink/8 last:border-b-0">
              <div className="flex items-center justify-between gap-6 py-[15px]">
                <span className="text-[21px] font-bold text-danger tabular-nums">{b.id}</span>
                <span className="flex items-center gap-2 text-[12px] text-danger/80">
                  <span className="rounded-chip border border-danger/50 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em]">
                    损坏
                  </span>
                  <span className="font-mono">{b.reason}</span>
                </span>
              </div>
            </li>
          ))}
        </motion.ul>
      )}
    </>
  );
}
