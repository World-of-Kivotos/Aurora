// 下载中心：Mod / 资源包 / 光影 三类资源的检索与安装。
// 游戏本体与 WOK 受管整合包不在这里——Aurora 已收敛为单实例专用启动器，装游戏的唯一入口在启动屏。
//
// 取数一律走 lib/prefetch：启动器起来时已在空闲期把三个 tab 的首屏拉过一遍，命中缓存直接出内容、不闪骨架；
// 未命中（冷启动或换了筛选参数）才走真请求并显示骨架。错误不吞，原样呈现并给重试。
//
// 安装落位：目标固定为 config.selected_version 指向的那一个实例，玩家不再选"装到哪儿"，
// 点安装后直接进入"选哪一版 -> 确认依赖计划"两步。实例不存在时整页只给空态，
// 不让玩家把文件装进一个不存在的目录。

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
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { InstallPlanPreview } from "../components/InstallPlanPreview";
import { ResourceCardSkeleton, Skeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";
import {
  AlertIcon,
  CheckIcon,
  CubeIcon,
  DownloadIcon,
  PackageIcon,
  PaletteIcon,
  RefreshIcon,
  SearchIcon,
  SunIcon,
} from "../components/icons";
import { pageItem, springs } from "../lib/motion";
import {
  getConfig,
  getVersionSettings,
  installMod,
  listInstalled,
  listModVersions,
  matchInstances,
  type Compatibility,
  type InstanceMatch,
  type ModLoader,
  type ModVersionInfo,
  type PlatformId,
  type ReleaseChannel,
  type ResourceType,
  type SearchHit,
  type SearchResultDto,
  type SortField,
  type VersionSettingsDto,
} from "../lib/ipc";
import {
  DEFAULT_SORT,
  TTL,
  defaultSearchKey,
  fetchSearch,
  peek,
  searchKey,
} from "../lib/prefetch";

type TabKey = "mod" | "resourcepack" | "shader";

const TABS: { key: TabKey; label: string; icon: typeof PackageIcon; type: ResourceType }[] = [
  { key: "mod", label: "Mod", icon: PackageIcon, type: "mod" },
  { key: "resourcepack", label: "资源包", icon: PaletteIcon, type: "resource_pack" },
  { key: "shader", label: "光影", icon: SunIcon, type: "shader" },
];

/** 单实例契约：全站唯一的安装目标，由 config.selected_version 指定并须在磁盘扫描里真实存在。 */
interface FixedInstance {
  id: string;
  mcVersion: string;
  loaders: string[];
}

/** 同行控件统一 40px 高——输入框、下拉、按钮共用，杜绝参差。 */
const CTRL = "h-10";

// 万级取整（5859万 而非 5859.1万）——小数位在这里没有信息量，只会把元信息行撑到换行。
function fmtCount(n: number): string {
  if (n >= 1e8) return (n / 1e8).toFixed(1) + "亿";
  if (n >= 1e4) return Math.round(n / 1e4) + "万";
  return String(n);
}

// ---- 通用：带图标的搜索框 ----
function SearchField({
  value,
  onChange,
  onEnter,
  placeholder,
  className = "",
}: {
  value: string;
  onChange: (v: string) => void;
  onEnter?: () => void;
  placeholder: string;
  className?: string;
}) {
  return (
    <div className={`relative ${className}`}>
      <span className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink/60">
        <SearchIcon size={16} />
      </span>
      {/* 输入框走 .surface-sunken：不透明度在暗图与亮图上方向相反，「凹下去」只有墨洗表达得稳定。
          它是寄生层，所以调用方必须把搜索框放进某个自足材质（这里是工具条那块 .surface-panel）。 */}
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && onEnter?.()}
        placeholder={placeholder}
        className={`${CTRL} surface-sunken w-full rounded-control pr-3 pl-9 text-[14px] text-ink outline-none placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2`}
      />
    </div>
  );
}

// ---- 通用：错误条 ----
function ErrorBar({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="surface-panel mb-4 flex items-center gap-3 rounded-panel border border-danger/35 px-4 py-3">
      <span className="text-danger [&_svg]:h-4.5 [&_svg]:w-4.5">
        <AlertIcon />
      </span>
      <span className="flex-1 text-[13px] text-danger">{message}</span>
      <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={onRetry}>
        重试
      </Button>
    </div>
  );
}

// ============ 安装落位：固定实例下的版本选择 ============
//
// 单实例模型下没有"装到哪个实例"这一问，只剩"装哪一版"。默认落在后端算好的 best_version，
// 一次回车就走完；需要换版本或强装时再把版本列表摊开。
// 不兼容项刻意不禁用——把强装的代价说清楚，选择权还给玩家。
// 底部常驻真实写入路径：装了却不生效多半是隔离档位没对上，这条回显是最后一道防线，不可省。

/// 平台认得的加载器名。实例侧探测到的加载器可能含 OptiFine 这类非 Mod 加载器，
/// 作为过滤条件传给平台只会得到空结果，先按这张表滤一遍。
const KNOWN_LOADERS: ModLoader[] = ["fabric", "quilt", "forge", "neoforge", "liteloader"];

type Tier = Compatibility["kind"];

const TIER_ORDER: Record<Tier, number> = { match: 0, unknown: 1, mismatch: 2 };

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

function VersionPicker({
  open,
  instance,
  platform,
  projectId,
  title,
  onClose,
  onConfirm,
}: {
  open: boolean;
  instance: FixedInstance;
  platform: PlatformId;
  projectId: string;
  /** 资源名，标题里显示。 */
  title: string;
  onClose: () => void;
  onConfirm: (modVersionId: string) => void;
}) {
  const confirmId = useId();
  const versionListId = useId();
  // 记「上一次按什么条件取过版本列表」，条件没变就不重复请求。空串表示还没取过。
  const versionsRequested = useRef("");

  const [match, setMatch] = useState<InstanceMatch | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // 玩家手动改过版本才有值，没改过就沿用后端算好的 best_version。
  const [chosen, setChosen] = useState<ModVersionInfo | null>(null);

  const [expanded, setExpanded] = useState(false);
  const [versions, setVersions] = useState<ModVersionInfo[] | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  // 版本列表默认只拉「配得上这个实例」的那些。一个热门 Mod 动辄上百个版本，
  // 把跨加载器、跨 MC 版本的全摊出来，玩家要在一片「不匹配」里自己找那几条能用的。
  // 需要强装或换版本时再切到全部。
  const [showAllVersions, setShowAllVersions] = useState(false);

  const [setting, setSetting] = useState<VersionSettingsDto | null>(null);
  const [settingError, setSettingError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      // 兼容矩阵仍由后端算：单实例只是把结果筛到那一条，判定规则不在前端另起一套。
      const list = await matchInstances(platform, projectId);
      const mine = list.find((m) => m.version_id === instance.id);
      if (!mine) {
        setMatch(null);
        setError(`兼容矩阵里没有实例 ${instance.id}，它可能刚被删除或改名。请回启动屏确认游戏还在。`);
        return;
      }
      setMatch(mine);
    } catch (e) {
      setError(String(e));
      setMatch(null);
    } finally {
      setLoading(false);
    }
  }, [platform, projectId, instance.id]);

  // 每次打开都重取：实例可能在上次打开之后被同步或回滚过，沿用旧矩阵会让人看到过时的「已装」标记。
  useEffect(() => {
    if (!open) return;
    setMatch(null);
    setChosen(null);
    setExpanded(false);
    setVersions(null);
    setVersionsError(null);
    versionsRequested.current = "";
    void load();
  }, [open, load]);

  const selectedVersion = chosen ?? match?.best_version ?? null;

  const loadVersions = useCallback(async () => {
    setVersionsLoading(true);
    setVersionsError(null);
    try {
      // 只看兼容时，把实例的 MC 版本与加载器交给后端过滤；实例的加载器里可能混着
      // OptiFine 这类非 Mod 加载器，先归一掉再传，否则过滤条件里带个平台不认识的名字。
      const loaders = showAllVersions
        ? []
        : instance.loaders
            .map((name) => name.toLowerCase())
            .filter((name): name is ModLoader => KNOWN_LOADERS.includes(name as ModLoader));
      const gameVersions = showAllVersions ? [] : [instance.mcVersion];
      setVersions(await listModVersions(platform, projectId, gameVersions, loaders));
    } catch (e) {
      setVersionsError(String(e));
    } finally {
      setVersionsLoading(false);
    }
  }, [platform, projectId, instance.loaders, instance.mcVersion, showAllVersions]);

  // 过滤条件只由「是否只看兼容」决定（实例是固定的），它一变就要重取。
  const versionsKey = String(showAllVersions);
  const ensureVersions = useCallback(() => {
    if (versionsRequested.current === versionsKey) return;
    versionsRequested.current = versionsKey;
    void loadVersions();
  }, [loadVersions, versionsKey]);

  // 展开状态下切开关，立刻按新条件重取，不让旧列表停在屏幕上冒充新结果。
  useEffect(() => {
    if (!expanded) return;
    if (versionsRequested.current === versionsKey) return;
    versionsRequested.current = versionsKey;
    setVersions(null);
    void loadVersions();
  }, [expanded, versionsKey, loadVersions]);

  // 没有任何兼容版本时必须自行指定，直接把列表摊开，省掉一次「点开才发现要选」。
  useEffect(() => {
    if (!match || selectedVersion) return;
    setExpanded(true);
    ensureVersions();
  }, [match, selectedVersion, ensureVersions]);

  // Modal 打开时焦点落在面板上，矩阵到位后转交主按钮，兑现「一次回车走完」。
  useEffect(() => {
    if (!open || !match) return;
    document.getElementById(confirmId)?.focus();
  }, [open, match, confirmId]);

  // 工作目录只取一次：实例固定，路径不会在弹窗生命周期里变。
  useEffect(() => {
    if (!open) return;
    let alive = true;
    setSetting(null);
    setSettingError(null);
    getVersionSettings(instance.id)
      .then((s) => {
        if (alive) setSetting(s);
      })
      .catch((e) => {
        if (alive) setSettingError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [open, instance.id]);

  const ranked = useMemo(() => {
    if (!versions) return [];
    return versions
      .map((v) => ({ v, tier: rankVersion(v, instance.mcVersion, instance.loaders) }))
      .sort((a, b) => TIER_ORDER[a.tier] - TIER_ORDER[b.tier]);
  }, [versions, instance.mcVersion, instance.loaders]);

  const toggleExpanded = () => {
    const next = !expanded;
    setExpanded(next);
    if (next) ensureVersions();
  };

  const retryVersions = () => {
    versionsRequested.current = "";
    ensureVersions();
  };

  const confirm = () => {
    if (!selectedVersion) return;
    onConfirm(selectedVersion.version_id);
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

  const tier: Tier | null = match ? match.compatibility.kind : null;
  const confirmLabel =
    tier === "mismatch" ? "仍然安装" : match?.already_installed ? "更新并安装" : "安装";
  const requiredDeps = selectedVersion
    ? selectedVersion.dependencies.filter((d) => d.kind === "required").length
    : 0;

  const footer = (
    <>
      <div className="mr-auto min-w-0 text-left">
        {settingError ? (
          <span className="text-[12px] break-words text-danger">
            读取工作目录失败：{settingError}
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
        disabled={!selectedVersion}
      >
        {confirmLabel}
      </Button>
    </>
  );

  return (
    <Modal open={open} onClose={onClose} size="lg" title={`安装「${title}」`} footer={footer}>
      {/* match 为空且无错时也走骨架：首帧 effect 还没起跑，否则会闪一帧假的「读不到实例」。 */}
      {loading || (!match && !error) ? (
        <div className="rounded-panel border border-ink/9 p-4">
          {/* 骨架框只留发丝描边、不铺底：Skeleton 自身就是一层墨洗，再垫一层下沉块会把
              单元素墨洗总量推过 8% 的上限（app.css 第五节），而它身下的弹窗面板已经是自足材质。 */}
          <Skeleton className="h-[11px] w-20" />
          <Skeleton className="mt-3 h-[17px] w-2/3" delay={0.08} />
          <Skeleton className="mt-2.5 h-[11px] w-full" delay={0.16} />
          <Skeleton className="mt-1.5 h-[11px] w-4/5" delay={0.24} />
        </div>
      ) : error ? (
        <div className="flex items-start gap-3 rounded-panel border border-danger/35 px-4 py-3">
          {/* 告警框不铺底：它里面站着一颗 secondary 按钮（.surface-control 是寄生层），
              再给外框铺一层下沉墨洗就成了寄生套寄生。危险语义由描边与文字色承担，底交给弹窗面板。 */}
          <span className="mt-px shrink-0 text-danger">
            <AlertIcon size={18} />
          </span>
          <p className="m-0 min-w-0 flex-1 text-[13px] break-words text-danger">{error}</p>
          <Button
            variant="secondary"
            className="shrink-0"
            icon={<RefreshIcon size={14} />}
            onClick={() => void load()}
          >
            重试
          </Button>
        </div>
      ) : (
        match && (
          <div className="min-w-0">
            {/* 详情卡取自足材质而非下沉块：它内部要放按钮与版本行（都是 .surface-control 寄生层），
                寄生层不得寄生在寄生层上。套在弹窗面板里，故用 .surface-nested 摘掉投影。 */}
            <div className="surface-panel surface-nested rounded-panel p-4">
              {/* 安装目标不再可选，但仍要如实回显：玩家得知道文件进的是哪一个实例。 */}
              <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                <span className="text-[10px] font-bold tracking-[0.2em] text-ink/75">安装到</span>
                <span className="min-w-0 truncate text-[13.5px] font-extrabold text-ink" title={match.version_id}>
                  {match.version_id}
                </span>
                <span className="font-mono text-[11px] text-ink/75 tabular-nums">
                  MC {instance.mcVersion} · {loaderText(instance.loaders)}
                </span>
                {match.already_installed && (
                  <span className="flex min-w-0 items-center gap-1 text-[11px] text-ink">
                    <CheckIcon size={12} />
                    <span className="truncate">已装 {match.already_installed}</span>
                  </span>
                )}
              </div>

              <div className="mt-3 border-t border-ink/10 pt-3 text-[10px] font-bold tracking-[0.2em] text-ink/75">
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
                    <dd className="m-0 min-w-0 text-ink">{loaderText(selectedVersion.loaders)}</dd>
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
                  这个实例下没有匹配的版本，需要自行指定要装哪一个。
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
                          className="flex max-h-[32vh] flex-col gap-1.5 overflow-y-auto pr-1"
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
                                onClick={() => setChosen(v)}
                                className={[
                                  "flex w-full cursor-pointer flex-col items-start rounded-control px-3 py-2 text-left",
                                  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                                  // 选中态是满墨实块，不挂材质：材质类是无层样式，会盖掉工具类的 bg-ink。
                                  // 静息/悬停/按下三态一并交给 .surface-control，这里不再自写 hover。
                                  picked ? "bg-ink text-paper-on transition-colors" : "surface-control",
                                ].join(" ")}
                              >
                                <span className="flex w-full items-center gap-2">
                                  <span className="min-w-0 truncate font-mono text-[12px] font-bold">
                                    {v.version_number}
                                  </span>
                                  <span
                                    className={`shrink-0 text-[10px] font-bold tracking-[0.12em] ${
                                      picked ? "text-paper-on/70" : channelTone(v.release_channel)
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

            {match.compatibility.kind === "mismatch" && (
              <div className="mt-3 rounded-panel border border-danger/35 px-4 py-3">
                <div className="flex items-center gap-2 text-[12px] font-bold text-danger">
                  <AlertIcon size={15} />
                  仍然安装的代价
                </div>
                <p className="mt-1.5 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                  {match.compatibility.reason}
                  。文件会照常写进 mods 目录，但游戏大概率加载不了它，也可能让这个实例启动即崩溃。
                  要撤销得回到 Mod 管理里停用或删除该文件。
                </p>
              </div>
            )}

            {match.compatibility.kind === "unknown" && (
              <p className="mt-3 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                平台没给全这个版本的兼容元数据，判不出行不行。可以先装上试，不对劲再从 Mod 管理里停用。
              </p>
            )}

            {match.already_installed && (
              <p className="mt-3 mb-0 text-[12.5px] leading-relaxed text-ink/75">
                这个实例已装 <span className="font-mono">{match.already_installed}</span>
                ，确认后会按所选版本写入 mods 目录。
              </p>
            )}
          </div>
        )
      )}
    </Modal>
  );
}

// ============ 内容类（Mod/资源包/光影）============
const SORT_OPTIONS: { value: SortField; label: string }[] = [
  { value: "relevance", label: "相关度" },
  { value: "downloads", label: "下载量" },
  { value: "updated", label: "最近更新" },
  { value: "follows", label: "收藏数" },
];

const LOADER_FILTER: { value: "all" | ModLoader; label: string }[] = [
  { value: "all", label: "全部加载器" },
  { value: "fabric", label: "Fabric" },
  { value: "quilt", label: "Quilt" },
  { value: "forge", label: "Forge" },
  { value: "neoforge", label: "NeoForge" },
];

/**
 * 资源图标：优先项目图标，加载失败/缺省回落首字母墨块（保持与卡片同尺寸，不跳版）。
 *
 * 图来自第三方，尺寸、透明通道与配色都不受我们控制，所以外面套一层可控的盒子：
 * overflow-hidden 把任何比例的原图裁进 rounded-control 的同心圆角里（省得指望远端图自己是方的），
 * 下沉底则接住大量透明底的 PNG——没有它，透明区会直接漏出背景照片，图标看起来像浮在半空。
 */
function ResIcon({ url, title }: { url: string | null; title: string }) {
  const [failed, setFailed] = useState(false);
  if (!url || failed) {
    return (
      <span className="grid h-12 w-12 shrink-0 place-items-center rounded-control bg-ink text-[17px] font-extrabold text-paper-on">
        {title.slice(0, 1).toUpperCase()}
      </span>
    );
  }
  return (
    <span className="surface-sunken block h-12 w-12 shrink-0 overflow-hidden rounded-control">
      <img
        src={url}
        alt=""
        width={48}
        height={48}
        loading="lazy"
        onError={() => setFailed(true)}
        className="h-full w-full object-cover"
      />
    </span>
  );
}

/** 安装按钮的状态机：状态直接长在按钮上，不靠列表另挂徽标（抄自 Modrinth App）。 */
type InstallState = "idle" | "installing" | "installed";

const INSTALL_LABEL: Record<InstallState, string> = {
  idle: "安装",
  installing: "安装中",
  installed: "已安装",
};

function ResourceCard({
  hit,
  index,
  state,
  onInstall,
}: {
  hit: SearchHit;
  index: number;
  state: InstallState;
  onInstall: () => void;
}) {
  return (
    <motion.li
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ ...springs.soft, delay: Math.min(index, 12) * 0.025 }}
      // 结果卡是全页信息最密的一块，又直接压在照片上，所以取最实的自足材质：
      // 96% 让 12.5px 的描述文字拿到 AAA 余量，投影则说明它是压在图上而不是在图里挖了个洞。
      className="surface-panel-strong flex items-start gap-3.5 rounded-panel p-3.5"
    >
      <ResIcon url={hit.icon_url} title={hit.title} />

      <div className="flex min-w-0 flex-1 flex-col self-stretch">
        <div className="flex items-baseline gap-2">
          <span className="truncate text-[15px] leading-tight font-extrabold">{hit.title}</span>
          {hit.author && <span className="shrink-0 text-[11px] text-ink/75">{hit.author}</span>}
        </div>
        <p className="mt-1 line-clamp-2 text-[12.5px] leading-snug text-ink/75">{hit.description}</p>
        {/* mt-auto 把元信息压到卡片底部：同一行左右卡片的这条线因此始终齐平，与描述占一行还是两行无关。
            nowrap + 只留一个分类，避免标签换行把卡片撑出参差。 */}
        <div className="mt-auto flex flex-nowrap items-center gap-2.5 overflow-hidden pt-2 text-[11px] whitespace-nowrap text-ink/75">
          <span className="inline-flex shrink-0 items-center gap-1 font-mono tabular-nums">
            <DownloadIcon size={12} />
            {fmtCount(hit.downloads)}
          </span>
          {/* 标记块的底也走令牌（.surface-sunken）而不是自己调一个半透明墨色：
              寄生在卡片这层自足材质上是合法的，且两端图上深浅方向一致。 */}
          <span className="surface-sunken shrink-0 rounded-chip px-1.5 py-0.5 tracking-wide uppercase">
            {hit.platform}
          </span>
          {hit.categories.slice(0, 1).map((c) => (
            <span key={c} className="surface-sunken truncate rounded-chip px-1.5 py-0.5">
              {c}
            </span>
          ))}
        </div>
      </div>

      <Button
        variant="secondary"
        // 原先静息态压到 55% 不透明度、悬停才实。玻璃底下这样做不成立：
        // 次按钮的字本就是 ink/75，再乘 0.55 等于 ink/41，对比度掉到 2.6，远在正文线以下。
        // 卡片现在自带描边与投影，安装按钮不靠「淡出」也不会喧宾夺主，索性常亮。
        className="shrink-0"
        icon={state === "installed" ? <CheckIcon size={15} /> : <DownloadIcon size={15} />}
        onClick={onInstall}
        disabled={state !== "idle"}
        title={state === "installed" ? "本次已安装到 World of Kivotos" : undefined}
      >
        {INSTALL_LABEL[state]}
      </Button>
    </motion.li>
  );
}

function ContentTab({ type, instance }: { type: ResourceType; instance: FixedInstance }) {
  const { toast } = useToast();
  // 已提交的检索条件（真正参与取数）；输入框内容单独存，回车/点搜索才提交。
  const [query, setQuery] = useState("");
  const [draft, setDraft] = useState("");
  const [sort, setSort] = useState<SortField>(DEFAULT_SORT);
  const [loaderFilter, setLoaderFilter] = useState<"all" | ModLoader>("all");
  const [result, setResult] = useState<SearchResultDto | null>(
    () => peek<SearchResultDto>(defaultSearchKey(type), TTL.search) ?? null,
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 安装流程两段式：先定版本（picking），再确认依赖计划（planning），最后真正下载。
  // 「装到哪个实例」这一段已随单实例模型取消。
  const [picking, setPicking] = useState<SearchHit | null>(null);
  const [planning, setPlanning] = useState<{ hit: SearchHit; modVersionId: string } | null>(null);
  // 按 平台+工程 记安装态，供按钮状态机使用。仅本次会话有效——真实已装状态由落位层从卷宗读。
  const [installState, setInstallState] = useState<Record<string, InstallState>>({});

  const loaders = useMemo<ModLoader[]>(() => (loaderFilter === "all" ? [] : [loaderFilter]), [loaderFilter]);
  // 加载器筛选只对 Mod 有意义，资源包与光影不挂加载器。
  const showLoaderFilter = type === "mod";

  const run = useCallback(async () => {
    // 先同步探一次缓存：命中就直接落内容，连一帧骨架都不出（预取铺好的热路径走这里）。
    const hit = peek<SearchResultDto>(searchKey(type, sort, query, loaders, []), TTL.search);
    if (hit) {
      setResult(hit);
      setError(null);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setResult(await fetchSearch(type, sort, query, loaders, []));
    } catch (e) {
      setError(String(e));
      setResult(null);
    } finally {
      setLoading(false);
    }
  }, [type, sort, query, loaders]);

  // 条件变化即取数；首帧若已被预取命中，run 会走 peek 分支同步返回，不会闪骨架。
  useEffect(() => {
    void run();
  }, [run]);

  const hits = result?.hits ?? [];
  const label = TABS.find((t) => t.type === type)?.label ?? "";

  const keyOf = (h: SearchHit) => h.platform + ":" + h.project_id;

  // 依赖计划确认后的真正安装。后端已按计划批量下 staging、校验通过才原子移入，
  // 并在落盘后写卷宗与历史，所以这里只管发起与反馈。
  const runInstall = useCallback(async () => {
    if (!planning) return;
    const { hit, modVersionId } = planning;
    const key = keyOf(hit);
    setPlanning(null);
    setInstallState((s) => ({ ...s, [key]: "installing" }));
    try {
      const outcome = await installMod(instance.id, hit.platform, hit.project_id, modVersionId);
      setInstallState((s) => ({ ...s, [key]: "installed" }));
      toast(`已安装 ${outcome.file_name}`, "success");
    } catch (e) {
      // 装失败要退回可重试态，否则按钮会永远卡在「安装中」。
      setInstallState((s) => ({ ...s, [key]: "idle" }));
      toast(String(e), "error");
    }
  }, [planning, instance.id, toast]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* relative z-20 是必须的，不是保险：面板带 backdrop-filter 就自成层叠上下文，
          Select 的下拉浮层再高的 z-index 也只在这块面板内部有效；
          不把整块面板抬到结果卡片之上，下拉就会被下面那排同样自成上下文的卡片盖住。
          shrink-0：它是本页的固定件，长内容只准滚在下面那块结果区里。 */}
      <div className="surface-panel relative z-20 mb-4 flex shrink-0 items-center gap-2 rounded-panel p-1.5">
        <SearchField
          value={draft}
          onChange={setDraft}
          onEnter={() => setQuery(draft)}
          placeholder={`搜索${label}`}
          className="min-w-0 flex-1"
        />
        {showLoaderFilter && (
          <div className="w-35 shrink-0">
            <Select<"all" | ModLoader>
              value={loaderFilter}
              onChange={setLoaderFilter}
              options={LOADER_FILTER}
              ariaLabel="加载器筛选"
              className={CTRL}
            />
          </div>
        )}
        <div className="w-31 shrink-0">
          <Select<SortField> value={sort} onChange={setSort} options={SORT_OPTIONS} ariaLabel="排序" className={CTRL} />
        </div>
        <Button variant="primary" className={`${CTRL} shrink-0 !py-0`} onClick={() => setQuery(draft)} disabled={loading}>
          搜索
        </Button>
      </div>

      {error && <ErrorBar message={error} onRetry={() => void run()} />}

      {/* 上游部分平台失败时结果仍可用，把失败平台如实列出来而不是假装完整。 */}
      {result && result.errors.length > 0 && (
        <p className="surface-panel mb-3 rounded-panel px-4 py-2.5 text-[12px] text-danger/85">
          {result.errors.map((e) => `${e.platform}：${e.message}`).join("；")}
        </p>
      )}

      {/* 结果区：全站白名单里允许滚的那一块。搜索命中数由上游决定，是本页唯一没有上界的内容。
          min-h-0 缺一不可，否则这块会被结果网格顶高，滚动条又长回外壳去。 */}
      <div className="min-h-0 flex-1 overflow-y-auto pr-1">
        {loading && !result ? (
          <ul className="m-0 grid list-none grid-cols-2 gap-2.5 p-0 max-[1180px]:grid-cols-1">
            {Array.from({ length: 6 }, (_, i) => (
              <li key={i}>
                <ResourceCardSkeleton delay={i * 0.08} />
              </li>
            ))}
          </ul>
        ) : hits.length === 0 && !error ? (
          // 空态本身只是一段字，压在照片上没底会看不见，所以由调用方给它垫一块面板。
          <div className="surface-panel rounded-panel px-5 py-2">
            <EmptyState icon={<PackageIcon />} title={query ? "没有结果，换个关键词试试" : `暂时没有可显示的${label}`} />
          </div>
        ) : (
          <ul className="m-0 grid list-none grid-cols-2 gap-2.5 p-0 max-[1180px]:grid-cols-1">
            {hits.map((h, i) => (
              <ResourceCard
                key={keyOf(h)}
                hit={h}
                index={i}
                state={installState[keyOf(h)] ?? "idle"}
                onInstall={() => setPicking(h)}
              />
            ))}
          </ul>
        )}
      </div>

      {/* 第一段：定版本。实例已经是固定的那一个，这里只解决「装哪一版」。 */}
      {picking && (
        <VersionPicker
          open
          instance={instance}
          platform={picking.platform}
          projectId={picking.project_id}
          title={picking.title}
          onClose={() => setPicking(null)}
          onConfirm={(modVersionId) => {
            setPlanning({ hit: picking, modVersionId });
            setPicking(null);
          }}
        />
      )}

      {/* 第二段：依赖清单预览。CurseForge 至今没做这块（CF-I-7577），是实打实的差异点。 */}
      <Modal
        open={!!planning}
        onClose={() => setPlanning(null)}
        size="lg"
        title={planning ? `安装「${planning.hit.title}」` : ""}
      >
        {planning && (
          <InstallPlanPreview
            versionId={instance.id}
            platform={planning.hit.platform}
            projectId={planning.hit.project_id}
            modVersionId={planning.modVersionId}
            onCancel={() => setPlanning(null)}
            onConfirm={() => void runInstall()}
          />
        )}
      </Modal>
    </div>
  );
}

export function Download() {
  const navigate = useNavigate();
  const [tab, setTab] = useState<TabKey>("mod");
  // undefined = 还没解析完；null = 解析完了但游戏没装。两者的界面完全不同，不能合并成一个假值。
  const [instance, setInstance] = useState<FixedInstance | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  const resolveInstance = useCallback(async () => {
    setError(null);
    try {
      // selected_version 只是配置里的一个 id，磁盘上未必还在（手删、迁目录都会让它悬空），
      // 所以必须与真实扫描 join 过才算「实例就位」。
      const [cfg, scan] = await Promise.all([getConfig(), listInstalled()]);
      const found = cfg.selected_version
        ? scan.versions.find((v) => v.id === cfg.selected_version)
        : undefined;
      setInstance(
        found
          ? {
              id: found.id,
              mcVersion: found.mc_version,
              loaders: found.loaders.map((l) => l.kind.toLowerCase()),
            }
          : null,
      );
    } catch (e) {
      setError(String(e));
      setInstance(null);
    }
  }, []);

  useEffect(() => {
    void resolveInstance();
  }, [resolveInstance]);

  const active = TABS.find((t) => t.key === tab)!;
  const ready = !!instance;

  return (
    <>
      {/* 抬头与 tab 合成同一块面板：原先那条贯通的分割线现在由面板自己的下缘充当，
          下划线正落在这条缘上——少画一条线，也省掉「线在图上飘着」的那种廉价感。
          实例没就位时不出 tab，那时下缘没有下划线要托，改由面板自己补足下内边距。 */}
      <motion.div
        variants={pageItem}
        className={`surface-panel mb-5 shrink-0 rounded-panel px-5 pt-4 ${ready ? "" : "pb-4"}`}
      >
        <div className="flex items-baseline gap-4">
          <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">下载</h1>
          <span className="text-[12px] text-ink/75">Mod、资源包与光影</span>
        </div>

        {/* 分段 tab：选中下划线用共享 layoutId，切换时在标签之间滑过去而不是闪现。 */}
        {ready && (
          <div className="mt-3.5 flex gap-1">
            {TABS.map((t) => {
              const on = t.key === tab;
              const Icon = t.icon;
              return (
                <button
                  key={t.key}
                  type="button"
                  onClick={() => setTab(t.key)}
                  aria-current={on ? "page" : undefined}
                  className={[
                    "relative flex cursor-pointer items-center gap-2 px-3 pb-3 text-[14px] transition-colors",
                    "focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
                    on ? "font-extrabold text-ink" : "font-semibold text-ink/75 hover:text-ink",
                  ].join(" ")}
                >
                  <Icon size={16} />
                  {t.label}
                  {on && (
                    <motion.span
                      layoutId="download-tab-underline"
                      className="absolute inset-x-0 bottom-0 h-0.5 bg-accent"
                      transition={springs.tap}
                    />
                  )}
                </button>
              );
            })}
          </div>
        )}
      </motion.div>

      {/* 抬头与 tab 定高不动，正文吃掉剩余高度：这条 flex 链一路传到 ContentTab 的结果区，
          中间每一层都得带 min-h-0，漏一层结果区就滚不起来（app.css 第六节）。 */}
      <motion.div variants={pageItem} className="flex min-h-0 flex-1 flex-col">
        {error && <ErrorBar message={`读取游戏实例失败：${error}`} onRetry={() => void resolveInstance()} />}

        {instance === undefined ? (
          <div className="surface-panel rounded-panel p-4">
            <Skeleton className="h-[11px] w-28" />
            <Skeleton className="mt-3 h-[15px] w-2/5" delay={0.08} />
          </div>
        ) : instance === null ? (
          // 装 Mod 得有个 mods 目录可写。游戏还没装时给死路一条比给个必然失败的按钮诚实。
          <div className="surface-panel rounded-panel px-5 py-2">
            <EmptyState
              icon={<CubeIcon />}
              title="还没有安装游戏。Mod、资源包与光影都要写进 World of Kivotos 的目录，先回启动屏把整合包装上。"
              action={{ label: "去启动屏安装", onClick: () => navigate("/") }}
            />
          </div>
        ) : (
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={tab}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={springs.tap}
              className="flex min-h-0 flex-1 flex-col"
            >
              <ContentTab type={active.type} instance={instance} />
            </motion.div>
          </AnimatePresence>
        )}
      </motion.div>
    </>
  );
}
