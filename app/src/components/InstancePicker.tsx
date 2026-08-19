// 安装落位层：点「安装」后先把「装到哪个实例」摊开成判定矩阵，而不是逼玩家先进实例再回头找资源。
// 三档分组直给结论（可以安装 / 可能可行 / 不兼容），默认落在第一档第一项、焦点交给主按钮，
// 一两个实例的场景一次回车就走完。不兼容项刻意不禁用——把强装的代价说清楚，选择权还给玩家。
// 底部常驻真实写入路径：装了却不生效多半是隔离档位没对上，这条回显是最后一道防线，不可省。

import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useNavigate } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { Modal } from "./Modal";
import { Skeleton } from "./Skeleton";
import { AlertIcon, CheckIcon, CubeIcon, RefreshIcon } from "./icons";
import { springs } from "../lib/motion";
import {
  getVersionSettings,
  listModVersions,
  matchInstances,
  type Compatibility,
  type InstanceMatch,
  type ModLoader,
  type ModVersionInfo,
  type PlatformId,
  type ReleaseChannel,
  type VersionSettingsDto,
} from "../lib/ipc";

/// 平台认得的加载器名。实例侧探测到的加载器可能含 OptiFine 这类非 Mod 加载器，
/// 作为过滤条件传给平台只会得到空结果，先按这张表滤一遍。
const KNOWN_LOADERS: ModLoader[] = ["fabric", "quilt", "forge", "neoforge", "liteloader"];

interface InstancePickerProps {
  open: boolean;
  platform: PlatformId;
  projectId: string;
  /** 资源名，标题里显示。 */
  title: string;
  onClose: () => void;
  onConfirm: (versionId: string, modVersionId: string) => void;
}

type Tier = Compatibility["kind"];

const TIER_ORDER: Record<Tier, number> = { match: 0, unknown: 1, mismatch: 2 };

const TIERS: { tier: Tier; label: string; note: string }[] = [
  { tier: "match", label: "可以安装", note: "MC 版本与加载器都对得上" },
  { tier: "unknown", label: "可能可行", note: "平台没给全兼容元数据，判不出行不行" },
  { tier: "mismatch", label: "不兼容", note: "对不上，强装多半加载不了" },
];

const TIER_BADGE: Record<Tier, string> = {
  match: "匹配",
  unknown: "元数据不全",
  mismatch: "不匹配",
};

const CHANNEL_LABEL: Record<ReleaseChannel, string> = {
  release: "正式版",
  beta: "测试版",
  alpha: "预览版",
};

const LOADER_LABEL: Record<string, string> = {
  fabric: "Fabric",
  quilt: "Quilt",
  forge: "Forge",
  neoforge: "NeoForge",
  liteloader: "LiteLoader",
};

// 同一行控件统一 40px 高（设计系统约束），primary 需要 !py-0 才能让高度生效。
const CTRL = "h-10";

/**
 * 后端已按档位排好序，这里再排一次只是让界面分组顺序不依赖后端实现细节。
 * sort 稳定，同档内保持后端给的实例 id 字典序，默认选中项因此可复现。
 */
function orderByTier(list: InstanceMatch[]): InstanceMatch[] {
  return [...list].sort(
    (a, b) => TIER_ORDER[a.compatibility.kind] - TIER_ORDER[b.compatibility.kind],
  );
}

/**
 * 展示用兼容判定，规则与后端 compat::classify 一一对应。
 * 没有「单个版本 × 单个实例」的 IPC，切换版本时的排序与打标只能在此本地算一份；
 * 真正的判定权仍在后端——安装时后端会重新算，两边不一致时以后端为准。
 */
function rankVersion(v: ModVersionInfo, mcVersion: string, instanceLoaders: string[]): Tier {
  const needsLoader = v.loaders.length > 0;
  if (needsLoader && instanceLoaders.length === 0) return "mismatch";
  if (needsLoader && !v.loaders.some((l) => instanceLoaders.includes(l))) return "mismatch";
  if (v.game_versions.length > 0 && !v.game_versions.includes(mcVersion)) return "mismatch";
  if (v.game_versions.length === 0 || v.loaders.length === 0) return "unknown";
  return "match";
}

// 认不出的加载器名原样显示，不改写成「未知」——玩家看到真串才好排查。
function loaderText(loaders: string[]): string {
  if (loaders.length === 0) return "原版";
  return loaders.map((l) => LOADER_LABEL[l] ?? l).join(" / ");
}

function formatSize(bytes: number | null): string {
  if (bytes === null) return "大小未标注";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}

function gameVersionText(list: string[]): string {
  if (list.length === 0) return "未标注 MC 版本";
  if (list.length <= 4) return list.join("、");
  return `${list.slice(0, 4).join("、")} 等 ${list.length} 个`;
}

// working_dir 的分隔符跟随后端所在平台，不臆断成反斜杠。
function modsDirOf(workingDir: string): string {
  const sep = workingDir.includes("\\") ? "\\" : "/";
  return `${workingDir.replace(/[\\/]+$/, "")}${sep}mods`;
}

// 非正式通道必须显眼：玩家有权知道自己在装预览版。
// 显眼不能靠朱红字：accent 作文字在控件底上只有 2.92~3.16，全档过不了正文的 4.5。
// 改成实心块 + 纸色字（4.77，底不透明所以与背景图无关），与 UpdatePanel 的通道徽标同一套写法。
function channelTone(channel: ReleaseChannel): string {
  return channel === "release"
    ? "text-ink/75"
    : "rounded-chip bg-accent px-1.5 py-0.5 text-paper-on";
}

export function InstancePicker({
  open,
  platform,
  projectId,
  title,
  onClose,
  onConfirm,
}: InstancePickerProps) {
  const navigate = useNavigate();
  const confirmId = useId();
  const versionListId = useId();
  const listRef = useRef<HTMLDivElement>(null);
  // 版本列表按「每次打开只取一次」的语义控制，用 ref 而非 state，避免取数状态回流触发重复请求。
  // 记「上一次按什么条件取过版本列表」，条件没变就不重复请求。空串表示还没取过。
  const versionsRequested = useRef("");

  const [matches, setMatches] = useState<InstanceMatch[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  // 玩家手动改过版本的实例才进这张表，没改过的沿用后端算好的 best_version。
  const [chosen, setChosen] = useState<Record<string, ModVersionInfo>>({});

  const [expanded, setExpanded] = useState(false);
  const [versions, setVersions] = useState<ModVersionInfo[] | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  // 版本列表默认只拉「配得上选中实例」的那些。一个热门 Mod 动辄上百个版本，
  // 把跨加载器、跨 MC 版本的全摊出来，玩家要在一片「不匹配」里自己找那几条能用的。
  // 需要强装或换版本时再切到全部。
  const [showAllVersions, setShowAllVersions] = useState(false);

  const [settings, setSettings] = useState<Record<string, VersionSettingsDto>>({});
  const [settingsError, setSettingsError] = useState<Record<string, string>>({});

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = orderByTier(await matchInstances(platform, projectId));
      setMatches(list);
      setSelectedId(list.length > 0 ? list[0].version_id : null);
    } catch (e) {
      setError(String(e));
      setMatches(null);
    } finally {
      setLoading(false);
    }
  }, [platform, projectId]);

  // 每次打开都重取：实例可能在上次打开之后被装过或删过，沿用旧矩阵会让人装错地方。
  useEffect(() => {
    if (!open) return;
    setMatches(null);
    setSelectedId(null);
    setChosen({});
    setExpanded(false);
    setVersions(null);
    setVersionsError(null);
    versionsRequested.current = "";
    void load();
  }, [open, load]);

  const groups = useMemo(
    () =>
      TIERS.map((t) => ({
        ...t,
        items: (matches ?? []).filter((m) => m.compatibility.kind === t.tier),
      })),
    [matches],
  );

  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);

  const selected = useMemo(
    () => flat.find((m) => m.version_id === selectedId) ?? null,
    [flat, selectedId],
  );

  const selectedVersion = selected ? (chosen[selected.version_id] ?? selected.best_version) : null;

  const loadVersions = useCallback(async () => {
    if (!selected) return;
    setVersionsLoading(true);
    setVersionsError(null);
    try {
      // 只看兼容时，把选中实例的 MC 版本与加载器交给后端过滤；实例的加载器里可能混着
      // OptiFine 这类非 Mod 加载器，先归一掉再传，否则过滤条件里带个平台不认识的名字。
      const loaders = showAllVersions
        ? []
        : selected.loaders
            .map((name) => name.toLowerCase())
            .filter((name): name is ModLoader => KNOWN_LOADERS.includes(name as ModLoader));
      const gameVersions = showAllVersions ? [] : [selected.mc_version];
      setVersions(await listModVersions(platform, projectId, gameVersions, loaders));
    } catch (e) {
      setVersionsError(String(e));
    } finally {
      setVersionsLoading(false);
    }
  }, [platform, projectId, selected, showAllVersions]);

  // 过滤条件由「选中实例 + 是否只看兼容」共同决定，任一变化都要重取。
  const versionsKey = selected ? `${selected.version_id}:${showAllVersions}` : "";
  const ensureVersions = useCallback(() => {
    if (versionsRequested.current === versionsKey) return;
    versionsRequested.current = versionsKey;
    void loadVersions();
  }, [loadVersions, versionsKey]);

  // 展开状态下切实例或切开关，立刻按新条件重取，不让旧列表停在屏幕上冒充新结果。
  useEffect(() => {
    if (!expanded || !selected) return;
    if (versionsRequested.current === versionsKey) return;
    versionsRequested.current = versionsKey;
    setVersions(null);
    void loadVersions();
  }, [expanded, selected, versionsKey, loadVersions]);

  // 该实例下没有任何兼容版本时必须自行指定，直接把列表摊开，省掉一次「点开才发现要选」。
  useEffect(() => {
    if (!selected || selectedVersion) return;
    setExpanded(true);
    ensureVersions();
  }, [selected, selectedVersion, ensureVersions]);

  // 上下键切换实例；焦点在版本列表里时让它自己管，不抢。
  useEffect(() => {
    if (!open || flat.length === 0) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      if ((e.target as HTMLElement | null)?.closest("[data-version-list]")) return;
      e.preventDefault();
      const at = flat.findIndex((m) => m.version_id === selectedId);
      const next =
        e.key === "ArrowDown" ? Math.min(at + 1, flat.length - 1) : Math.max(at - 1, 0);
      setSelectedId(flat[next].version_id);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open, flat, selectedId]);

  useEffect(() => {
    if (!selectedId) return;
    listRef.current
      ?.querySelector(`[data-instance-id="${CSS.escape(selectedId)}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [selectedId]);

  // Modal 打开时焦点落在面板上，矩阵到位后转交主按钮，兑现「一次回车走完」。
  useEffect(() => {
    if (!open || flat.length === 0) return;
    document.getElementById(confirmId)?.focus();
  }, [open, flat.length, confirmId]);

  // 工作目录按实例缓存；失败也记下来，避免同一个实例被反复重试。
  useEffect(() => {
    if (!selected) return;
    const id = selected.version_id;
    if (settings[id] || settingsError[id]) return;
    let alive = true;
    getVersionSettings(id)
      .then((s) => {
        if (alive) setSettings((m) => ({ ...m, [id]: s }));
      })
      .catch((e) => {
        if (alive) setSettingsError((m) => ({ ...m, [id]: String(e) }));
      });
    return () => {
      alive = false;
    };
  }, [selected, settings, settingsError]);

  const ranked = useMemo(() => {
    if (!selected || !versions) return [];
    return versions
      .map((v) => ({ v, tier: rankVersion(v, selected.mc_version, selected.loaders) }))
      .sort((a, b) => TIER_ORDER[a.tier] - TIER_ORDER[b.tier]);
  }, [selected, versions]);

  const selectInstance = (id: string) => {
    setSelectedId(id);
    // 选完把焦点带回主按钮，下一次回车即完成，不必再 Tab 一圈。
    document.getElementById(confirmId)?.focus();
  };

  const toggleExpanded = () => {
    const next = !expanded;
    setExpanded(next);
    if (next) ensureVersions();
  };

  const retryVersions = () => {
    versionsRequested.current = "";
    ensureVersions();
  };

  const chooseVersion = (v: ModVersionInfo) => {
    if (!selected) return;
    setChosen((m) => ({ ...m, [selected.version_id]: v }));
  };

  const confirm = () => {
    if (!selected || !selectedVersion) return;
    onConfirm(selected.version_id, selectedVersion.version_id);
  };

  const onVersionKeyDown = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const nodes = Array.from(
      e.currentTarget.querySelectorAll<HTMLButtonElement>("[data-version-option]"),
    );
    if (nodes.length === 0) return;
    e.preventDefault();
    const at = nodes.indexOf(document.activeElement as HTMLButtonElement);
    const next = e.key === "ArrowDown" ? Math.min(at + 1, nodes.length - 1) : Math.max(at - 1, 0);
    nodes[next].focus();
  };

  const tier: Tier | null = selected ? selected.compatibility.kind : null;
  const confirmLabel =
    tier === "mismatch" ? "仍然安装" : selected?.already_installed ? "更新并安装" : "安装";
  const requiredDeps = selectedVersion
    ? selectedVersion.dependencies.filter((d) => d.kind === "required").length
    : 0;

  const setting = selected ? settings[selected.version_id] : undefined;
  const settingErr = selected ? settingsError[selected.version_id] : undefined;

  const footer = (
    <>
      <div className="mr-auto min-w-0 text-left">
        {!selected ? (
          <span className="text-[12px] text-ink/75">未选中实例</span>
        ) : settingErr ? (
          <span className="text-[12px] break-words text-danger">
            读取工作目录失败：{settingErr}
          </span>
        ) : !setting ? (
          <span className="text-[12px] text-ink/75">正在解析工作目录…</span>
        ) : (
          <>
            <div
              className="truncate font-mono text-[12px] text-ink/75"
              title={modsDirOf(setting.working_dir)}
            >
              将写入 {modsDirOf(setting.working_dir)}
            </div>
            <div className="mt-0.5 text-[11px] text-ink/75">
              {setting.isolated
                ? setting.forced_by_local_data
                  ? "版本隔离：开（该版本目录下已有本地数据，强制隔离）"
                  : "版本隔离：开，文件只属于这个实例"
                : "版本隔离：关，与其它未隔离实例共用同一个 mods 目录"}
            </div>
          </>
        )}
      </div>
      <Button variant="secondary" className={`${CTRL} shrink-0`} onClick={onClose}>
        取消
      </Button>
      <Button
        id={confirmId}
        variant="primary"
        className={`${CTRL} shrink-0 !py-0`}
        onClick={confirm}
        disabled={!selected || !selectedVersion}
      >
        {confirmLabel}
      </Button>
    </>
  );

  return (
    <Modal open={open} onClose={onClose} size="xl" title={`安装「${title}」`} footer={footer}>
      {/* matches 为空且无错时也走骨架：首帧 effect 还没起跑，否则会闪一帧「没有实例」的假空态。 */}
      {loading || (!matches && !error) ? (
        <div className="grid grid-cols-2 gap-5 max-[860px]:grid-cols-1">
          {/* 骨架框只留发丝描边、不铺底：Skeleton 自身就是一层墨洗，再垫一层下沉块会把
              单元素墨洗总量推过 8% 的上限（app.css 第五节），而它身下的弹窗面板已经是自足材质。 */}
          <div className="flex flex-col gap-2">
            {Array.from({ length: 4 }, (_, i) => (
              <div key={i} className="rounded-panel border border-ink/9 px-3.5 py-3">
                <Skeleton className="h-[15px] w-2/5" delay={i * 0.06} />
                <Skeleton className="mt-2 h-[11px] w-3/5" delay={i * 0.06 + 0.08} />
              </div>
            ))}
          </div>
          <div className="rounded-panel border border-ink/9 p-4">
            <Skeleton className="h-[11px] w-20" />
            <Skeleton className="mt-3 h-[17px] w-2/3" delay={0.08} />
            <Skeleton className="mt-2.5 h-[11px] w-full" delay={0.16} />
            <Skeleton className="mt-1.5 h-[11px] w-4/5" delay={0.24} />
          </div>
        </div>
      ) : error ? (
        <div className="flex items-start gap-3 rounded-panel border border-danger/35 px-4 py-3">
          {/* 告警框不铺底：它里面站着一颗 secondary 按钮（.surface-control 是寄生层），
              再给外框铺一层下沉墨洗就成了寄生套寄生。危险语义由描边与文字色承担，底交给弹窗面板。 */}
          <span className="mt-px shrink-0 text-danger">
            <AlertIcon size={18} />
          </span>
          <p className="m-0 min-w-0 flex-1 text-[13px] break-words text-danger">
            读取实例矩阵失败：{error}
          </p>
          <Button
            variant="secondary"
            className="shrink-0"
            icon={<RefreshIcon size={14} />}
            onClick={() => void load()}
          >
            重试
          </Button>
        </div>
      ) : flat.length === 0 ? (
        <EmptyState
          icon={<CubeIcon />}
          title="还没有任何已安装的游戏实例。先装一个游戏版本，再回来给它装 Mod。"
          action={{
            label: "去下载游戏版本",
            onClick: () => {
              onClose();
              navigate("/download");
            },
          }}
        />
      ) : (
        <div className="grid grid-cols-2 gap-5 max-[860px]:grid-cols-1">
          {/* 左：实例矩阵，三档分组 */}
          <div
            ref={listRef}
            role="group"
            aria-label="选择目标实例"
            className="max-h-[52vh] min-w-0 overflow-y-auto pr-1"
          >
            {groups
              .filter((g) => g.items.length > 0)
              .map((g) => (
                <section key={g.tier} className="mb-4 last:mb-0">
                  <header className="mb-2 flex items-baseline gap-2">
                    <h3 className="m-0 text-[11px] font-bold tracking-[0.18em] text-ink/75">
                      {g.label}
                    </h3>
                    <span className="text-[11px] text-ink/75">{g.note}</span>
                  </header>
                  <ul className="m-0 flex list-none flex-col gap-1.5 p-0">
                    {g.items.map((m) => {
                      const on = m.version_id === selectedId;
                      return (
                        <li key={m.version_id}>
                          <button
                            type="button"
                            data-instance-id={m.version_id}
                            aria-pressed={on}
                            onClick={() => selectInstance(m.version_id)}
                            className={[
                              "flex w-full cursor-pointer flex-col items-start rounded-control px-3 py-2 text-left",
                              "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                              // 选中态是满墨实块，不挂材质：材质类是无层样式，会盖掉工具类的 bg-ink。
                              // 静息/悬停/按下三态一并交给 .surface-control，这里不再自写 hover。
                              on ? "bg-ink text-paper-on transition-colors" : "surface-control",
                            ].join(" ")}
                          >
                            <span className="flex w-full items-center gap-2">
                              <span className="min-w-0 truncate text-[13.5px] leading-tight font-extrabold">
                                {m.version_id}
                              </span>
                              {m.already_installed && (
                                <span
                                  title={`已装 ${m.already_installed}`}
                                  // 未选中态原为朱红字，在控件底上只有 2.92，正文门槛不达标；
                                  // 这行是「这个实例已经装了哪一版」的事实陈述，改回满墨即可，不必抢眼。
                                  className={`ml-auto flex min-w-0 shrink items-center gap-1 text-[11px] ${
                                    on ? "text-paper-on/70" : "text-ink"
                                  }`}
                                >
                                  <CheckIcon size={12} />
                                  <span className="truncate">已装 {m.already_installed}</span>
                                </span>
                              )}
                            </span>
                            <span
                              className={`mt-0.5 truncate font-mono text-[10.5px] tabular-nums ${
                                on ? "text-paper-on/55" : "text-ink/75"
                              }`}
                            >
                              MC {m.mc_version} · {loaderText(m.loaders)}
                            </span>
                            {m.compatibility.kind === "mismatch" && (
                              <span
                                // 满档 danger 而不是 danger/85：后者压在控件底上实算 4.38，差 0.12 过不了正文。
                                className={`mt-1 text-[11px] ${
                                  on ? "text-paper-on/70" : "text-danger"
                                }`}
                              >
                                {m.compatibility.reason}
                              </span>
                            )}
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </section>
              ))}
          </div>

          {/* 右：选中实例将装入的版本 + 版本切换 + 代价说明 */}
          <div className="min-w-0">
            {selected && (
              <>
                {/* 详情卡取自足材质而非下沉块：它内部要放按钮与版本行（都是 .surface-control 寄生层），
                    寄生层不得寄生在寄生层上。套在弹窗面板里，故用 .surface-nested 摘掉投影。 */}
                <div className="surface-panel surface-nested rounded-panel p-4">
                  <div className="text-[10px] font-bold tracking-[0.2em] text-ink/75">
                    将安装的版本
                  </div>

                  {selectedVersion ? (
                    <>
                      <div className="mt-1.5 flex items-baseline gap-2">
                        <span
                          className="min-w-0 truncate text-[17px] leading-tight font-extrabold text-ink"
                          title={selectedVersion.name}
                        >
                          {selectedVersion.name}
                        </span>
                        <span
                          className={`shrink-0 text-[10px] font-bold tracking-[0.12em] ${channelTone(
                            selectedVersion.release_channel,
                          )}`}
                        >
                          {CHANNEL_LABEL[selectedVersion.release_channel]}
                        </span>
                      </div>
                      <div
                        className="mt-1 truncate font-mono text-[12px] text-ink/75"
                        title={selectedVersion.version_number}
                      >
                        {selectedVersion.version_number}
                      </div>
                      {/* 标签与值的层级改由墨阶拉开（ink/75 对满墨）而不是再往下切灰度：
                          玻璃上的正文下限就是 ink/75，标签一档已经踩在线上，只能把值往上提。 */}
                      <dl className="mt-3 mb-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-[12px]">
                        <dt className="text-ink/75">MC 版本</dt>
                        <dd className="m-0 min-w-0 text-ink">
                          {gameVersionText(selectedVersion.game_versions)}
                        </dd>
                        <dt className="text-ink/75">加载器</dt>
                        <dd className="m-0 min-w-0 text-ink">
                          {loaderText(selectedVersion.loaders)}
                        </dd>
                        <dt className="text-ink/75">文件</dt>
                        <dd
                          className="m-0 min-w-0 truncate font-mono text-ink"
                          title={selectedVersion.file_name}
                        >
                          {selectedVersion.file_name}
                        </dd>
                        <dt className="text-ink/75">大小</dt>
                        <dd className="m-0 min-w-0 text-ink tabular-nums">
                          {formatSize(selectedVersion.file_size)}
                        </dd>
                        <dt className="text-ink/75">发布</dt>
                        <dd className="m-0 min-w-0 text-ink tabular-nums">
                          {selectedVersion.date_published
                            ? selectedVersion.date_published.slice(0, 10)
                            : "日期未标注"}
                        </dd>
                      </dl>
                    </>
                  ) : (
                    <p className="mt-2 mb-0 text-[13px] text-ink/75">
                      该实例下没有匹配的版本，需要自行指定要装哪一个。
                    </p>
                  )}

                  <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-2">
                    <Button
                      variant="secondary"
                      onClick={toggleExpanded}
                      aria-expanded={expanded}
                      aria-controls={versionListId}
                    >
                      {expanded ? "收起版本列表" : "切换版本"}
                    </Button>
                    {expanded && (
                      <button
                        type="button"
                        onClick={() => setShowAllVersions((v) => !v)}
                        className="cursor-pointer text-[11px] text-ink/75 underline-offset-2 transition-colors hover:text-ink hover:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                      >
                        {showAllVersions ? "只看配得上的" : "显示全部版本"}
                      </button>
                    )}
                    {requiredDeps > 0 && (
                      <span className="text-[11px] text-ink/75">
                        {requiredDeps} 项必需依赖会随本次安装一并装入
                      </span>
                    )}
                  </div>

                  <AnimatePresence initial={false}>
                    {expanded && (
                      <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: "auto", opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={springs.tap}
                        className="overflow-hidden"
                      >
                        <div className="mt-3 border-t border-ink/10 pt-3">
                          {versionsLoading && !versions ? (
                            <div className="flex flex-col gap-1.5">
                              {Array.from({ length: 3 }, (_, i) => (
                                <Skeleton key={i} className="h-[46px] w-full" delay={i * 0.06} />
                              ))}
                            </div>
                          ) : versionsError ? (
                            <div className="flex items-start gap-3">
                              <p className="m-0 min-w-0 flex-1 text-[12px] break-words text-danger">
                                读取版本列表失败：{versionsError}
                              </p>
                              <Button
                                variant="secondary"
                                className="shrink-0"
                                icon={<RefreshIcon size={14} />}
                                onClick={retryVersions}
                              >
                                重试
                              </Button>
                            </div>
                          ) : ranked.length === 0 ? (
                            <p className="m-0 text-[12px] text-ink/75">
                              {showAllVersions
                                ? "该工程在平台上没有可用版本。"
                                : "没有配得上这个实例的版本。点上方「显示全部版本」可自行挑一个强装。"}
                            </p>
                          ) : (
                            <div
                              id={versionListId}
                              data-version-list
                              role="radiogroup"
                              aria-label="选择要安装的版本"
                              onKeyDown={onVersionKeyDown}
                              className="flex max-h-[26vh] flex-col gap-1.5 overflow-y-auto pr-1"
                            >
                              {ranked.map(({ v, tier: vTier }) => {
                                const picked = v.version_id === selectedVersion?.version_id;
                                return (
                                  <button
                                    key={v.version_id}
                                    type="button"
                                    data-version-option
                                    role="radio"
                                    aria-checked={picked}
                                    onClick={() => chooseVersion(v)}
                                    className={[
                                      "flex w-full cursor-pointer flex-col items-start rounded-control px-3 py-2 text-left",
                                      "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                                      picked
                                        ? "bg-ink text-paper-on transition-colors"
                                        : "surface-control",
                                    ].join(" ")}
                                  >
                                    <span className="flex w-full items-center gap-2">
                                      <span className="min-w-0 truncate font-mono text-[12px] font-bold">
                                        {v.version_number}
                                      </span>
                                      <span
                                        className={`shrink-0 text-[10px] font-bold tracking-[0.12em] ${
                                          picked
                                            ? "text-paper-on/70"
                                            : channelTone(v.release_channel)
                                        }`}
                                      >
                                        {CHANNEL_LABEL[v.release_channel]}
                                      </span>
                                      <span
                                        className={`ml-auto shrink-0 text-[10px] ${
                                          picked
                                            ? "text-paper-on/60"
                                            : vTier === "mismatch"
                                              ? // danger/80 在控件底上只有 4.02，10px 常规字重不吃大字豁免，一律走满档。
                                                "text-danger"
                                              : "text-ink/75"
                                        }`}
                                      >
                                        {TIER_BADGE[vTier]}
                                      </span>
                                    </span>
                                    <span
                                      className={`mt-0.5 w-full truncate text-[11px] ${
                                        picked ? "text-paper-on/55" : "text-ink/75"
                                      }`}
                                    >
                                      {gameVersionText(v.game_versions)} · {loaderText(v.loaders)}
                                    </span>
                                  </button>
                                );
                              })}
                            </div>
                          )}
                        </div>
                      </motion.div>
                    )}
                  </AnimatePresence>
                </div>

                {selected.compatibility.kind === "mismatch" && (
                  <div className="mt-3 rounded-panel border border-danger/35 px-4 py-3">
                    <div className="flex items-center gap-2 text-[12px] font-bold text-danger">
                      <AlertIcon size={15} />
                      仍然安装的代价
                    </div>
                    <p className="mt-1.5 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                      {selected.compatibility.reason}
                      。文件会照常写进 mods 目录，但游戏大概率加载不了它，也可能让这个实例启动即崩溃。
                      要撤销得回到 Mod 管理里停用或删除该文件。
                    </p>
                  </div>
                )}

                {selected.compatibility.kind === "unknown" && (
                  <p className="mt-3 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                    平台没给全这个版本的兼容元数据，判不出行不行。可以先装上试，不对劲再从 Mod
                    管理里停用。
                  </p>
                )}

                {selected.already_installed && (
                  <p className="mt-3 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                    该实例已装 <span className="font-mono">{selected.already_installed}</span>
                    ，确认后会按所选版本写入 mods 目录。
                  </p>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </Modal>
  );
}
