// 下载中心：游戏版本 + Mod/整合包/资源包/光影 合一（PCL2 式心智——用户来这里就是"要拿新东西"）。
// 取数一律走 lib/prefetch：启动器起来时已在空闲期把五个 tab 的首屏拉过一遍，命中缓存直接出内容、不闪骨架；
// 未命中（冷启动或换了筛选参数）才走真请求并显示骨架。错误不吞，原样呈现并给重试。

import { useCallback, useEffect, useMemo, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { InstancePicker } from "../components/InstancePicker";
import { InstallPlanPreview } from "../components/InstallPlanPreview";
import { ModpackInstallFlow, type ModpackInstallState } from "../components/ModpackInstallFlow";
import { ResourceCardSkeleton, VersionCardSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";
import {
  AlertIcon,
  BoxesIcon,
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
  installMod,
  installManagedModpack,
  installVersion,
  listInstalled,
  type LoaderChoice,
  type ManifestDto,
  type ModLoader,
  type ResourceType,
  type SearchHit,
  type SearchResultDto,
  type SortField,
} from "../lib/ipc";
import { parseModpackSyncError } from "../lib/modpack-ui";
import { managedModpackPointerFromSearch } from "../lib/modpack-navigation";
import {
  DEFAULT_SORT,
  MANIFEST_KEY,
  TTL,
  defaultSearchKey,
  fetchManifest,
  fetchSearch,
  invalidate,
  peek,
  searchKey,
} from "../lib/prefetch";

type TabKey = "version" | "mod" | "modpack" | "resourcepack" | "shader";

const TABS: { key: TabKey; label: string; icon: typeof CubeIcon; type?: ResourceType }[] = [
  { key: "version", label: "游戏版本", icon: CubeIcon },
  { key: "mod", label: "Mod", icon: PackageIcon, type: "mod" },
  { key: "modpack", label: "整合包", icon: BoxesIcon, type: "modpack" },
  { key: "resourcepack", label: "资源包", icon: PaletteIcon, type: "resource_pack" },
  { key: "shader", label: "光影", icon: SunIcon, type: "shader" },
];

const BUILT_IN_MODPACK = {
  label: "WOK 地址",
  pointer_url: "https://api.mcwok.cn/api/v1/pack/latest",
};

const INITIAL_MODPACK_INSTALL_PROGRESS = {
  stage: "resolving_manifest",
  completed_files: 0,
  total_files: 0,
  downloaded_bytes: 0,
  total_bytes: null,
  current_file: null,
} as const;

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

// ---- 通用：分段筛选（下沉轨 + 液态玻璃选中页）----
function Segmented<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
  ariaLabel: string;
}) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className={`${CTRL} surface-sunken flex shrink-0 items-center gap-1 rounded-control p-1`}
    >
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            onClick={() => onChange(o.value)}
            aria-pressed={on}
            className={[
              "relative h-full cursor-pointer rounded-chip px-3 text-[13px] font-bold transition-colors",
              "focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
              on ? "text-ink" : "text-ink/75 hover:text-ink",
            ].join(" ")}
          >
            {/* 分段控件的选中页是液态玻璃四个准用位之一。选中态因此从「墨底反白」改成
                「玻璃浮片 + 满墨字」：反白那套在照片上会随图明暗忽轻忽重，玻璃片则始终跟着底走。
                套在下沉轨里，按「嵌套即摘影」加 surface-nested——与 InstanceDetail、Settings
                那两处分段控件是同一个部件的三份拷贝，投影档位必须一致，否则同一种控件三页三个样。
                圆角是唯一允许不同的一项，因为它按同心圆角链算：这里的轨是 rounded-control 配 p-1，
                内页 10-4=6 取 chip；Settings 那处的轨是 rounded-panel 配 p-1.5，内页 16-6=10 取 control。
                两者都在链上，看着不一样恰恰是对的，别把它们「统一」成同一个半径。 */}
            {on && (
              <motion.span
                layoutId={`seg-${ariaLabel}`}
                className="surface-liquid surface-nested absolute inset-0 rounded-chip"
                transition={springs.tap}
              />
            )}
            <span className="relative">{o.label}</span>
          </button>
        );
      })}
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

// ============ 游戏版本 ============
const LOADER_OPTIONS: { value: "none" | LoaderChoice; label: string }[] = [
  { value: "none", label: "原版" },
  { value: "fabric", label: "Fabric" },
  { value: "quilt", label: "Quilt" },
  { value: "forge", label: "Forge" },
  { value: "neoforge", label: "NeoForge" },
];

type VersionKind = "release" | "snapshot" | "old";
const KIND_OPTIONS: { value: VersionKind; label: string }[] = [
  { value: "release", label: "正式版" },
  { value: "snapshot", label: "快照" },
  { value: "old", label: "远古" },
];

/** 清单动辄 900+ 项，一次全渲染没意义也拖帧；截断到这个数，并在工具条如实告知被截了多少。 */
const VERSION_LIMIT = 60;

function VersionTab() {
  const { toast } = useToast();
  // 预取命中就以缓存作首帧初值，直接跳过骨架。
  const [manifest, setManifest] = useState<ManifestDto | null>(
    () => peek<ManifestDto>(MANIFEST_KEY, TTL.manifest) ?? null,
  );
  const [error, setError] = useState<string | null>(null);
  const [installedIds, setInstalledIds] = useState<Set<string>>(new Set());
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState<VersionKind>("release");
  const [pick, setPick] = useState<string | null>(null);
  const [loader, setLoader] = useState<"none" | LoaderChoice>("none");
  const [installing, setInstalling] = useState(false);

  const loadManifest = useCallback(async () => {
    setError(null);
    try {
      setManifest(await fetchManifest());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadInstalled = useCallback(async () => {
    const scan = await listInstalled();
    setInstalledIds(new Set(scan.versions.map((v) => v.id)));
  }, []);

  useEffect(() => {
    if (!manifest) void loadManifest();
    void loadInstalled();
  }, [manifest, loadManifest, loadInstalled]);

  const all = manifest?.versions ?? [];
  const matched = useMemo(() => {
    const q = search.trim().toLowerCase();
    return all.filter((v) => {
      const inKind =
        kind === "old" ? v.release_type.startsWith("old_") : v.release_type === kind;
      return inKind && (!q || v.id.toLowerCase().includes(q));
    });
  }, [all, kind, search]);
  const shown = matched.slice(0, VERSION_LIMIT);

  const doInstall = async () => {
    if (!pick) return;
    setInstalling(true);
    try {
      await installVersion(pick, loader === "none" ? undefined : loader);
      toast(`已安装 ${pick}`, "success");
      setPick(null);
      // 新版本落地后，清单里的"已安装"标记与版本页都得重取。
      invalidate(MANIFEST_KEY);
      await Promise.all([loadManifest(), loadInstalled()]);
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="min-w-0">
      {/* 工具条自成一块面板：里面的搜索框与分段轨都是寄生层，没有自足材质垫底就等于铺在照片上。
          p-1.5 不是随手取的间距——16（panel）- 6 = 10（control），内外两圈圆角才是同心的。 */}
      <div className="surface-panel mb-4 flex items-center gap-2 rounded-panel p-1.5">
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="搜索版本号，如 1.20"
          className="w-75 shrink-0"
        />
        <Segmented value={kind} onChange={setKind} options={KIND_OPTIONS} ariaLabel="版本类型" />
        <span className="ml-auto shrink-0 px-2 font-mono text-[12px] text-ink/75 tabular-nums">
          {manifest
            ? matched.length > VERSION_LIMIT
              ? `${matched.length} 个匹配 · 显示前 ${VERSION_LIMIT}`
              : `${matched.length} 个版本`
            : "读取清单中"}
        </span>
      </div>

      {/* 选中某版本才出现的安装条：高度滑入，不选就不占位。 */}
      <AnimatePresence initial={false}>
        {pick && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={springs.tap}
            // 高度动画得靠 overflow-hidden，但它会把面板压在照片上的那圈投影一起剪掉；
            // 用等量的负外边距 + 内边距把裁剪框向外让开，投影就完整了，收起时仍然真正 0 高。
            // z-20：面板带 backdrop-filter 即自成层叠上下文，加载器下拉浮层只有整块被抬起来，
            // 才不会被下面同样自成上下文的版本格盖住。
            className="relative z-20 -mx-3 overflow-hidden px-3"
          >
            <div className="surface-panel mb-4 flex items-center gap-4 rounded-panel px-4 py-3">
              <div className="min-w-0">
                <div className="text-[10px] font-bold tracking-[0.2em] text-ink/75">即将安装</div>
                <div className="mt-0.5 truncate text-[21px] leading-tight font-extrabold tabular-nums">{pick}</div>
              </div>
              <div className="ml-auto w-37.5 shrink-0">
                <Select<"none" | LoaderChoice>
                  value={loader}
                  onChange={setLoader}
                  options={LOADER_OPTIONS}
                  ariaLabel="加载器"
                  className={CTRL}
                />
              </div>
              <Button
                variant="primary"
                className={`${CTRL} shrink-0 !py-0`}
                icon={<DownloadIcon size={16} />}
                onClick={() => void doInstall()}
                disabled={installing}
              >
                {installing ? "安装中" : "安装"}
              </Button>
              <Button variant="secondary" className={`${CTRL} shrink-0`} onClick={() => setPick(null)}>
                取消
              </Button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {error && <ErrorBar message={error} onRetry={() => void loadManifest()} />}

      {/* 版本格一屏能到 60 个。它们做成寄生的 .surface-control 而不是各自一块玻璃：
          60 个 backdrop-filter 是白烧 GPU，而墨洗零采样成本，还顺带让整片格子共用一块托底。
          托底取 .surface-panel-strong——这是要逐格比对版本号的地方，96% 那档买的是扫读余量。 */}
      <div className="surface-panel-strong rounded-panel p-1.5">
        {!manifest ? (
          <div className="grid grid-cols-4 gap-1.5 max-[1180px]:grid-cols-3">
            {Array.from({ length: 12 }, (_, i) => (
              <VersionCardSkeleton key={i} delay={i * 0.05} />
            ))}
          </div>
        ) : shown.length === 0 ? (
          <div className="px-2">
            <EmptyState icon={<CubeIcon />} title="没有匹配的版本" />
          </div>
        ) : (
          <ul className="m-0 grid list-none grid-cols-4 gap-1.5 p-0 max-[1180px]:grid-cols-3">
            {shown.map((v, i) => {
              const active = v.id === pick;
              const installed = installedIds.has(v.id);
              return (
                <motion.li
                  key={v.id}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ ...springs.soft, delay: Math.min(i, 14) * 0.018 }}
                >
                  {/* 选中态保留实墨块：它是这一格「被选走了」的唯一信号，玻璃档之间的差别太细，
                      在一片同色格子里读不出来。未选中才交给 .surface-control 的静息/悬停。 */}
                  <button
                    type="button"
                    onClick={() => setPick(active ? null : v.id)}
                    aria-pressed={active}
                    className={[
                      "flex w-full cursor-pointer flex-col items-start rounded-control px-3.5 py-3 text-left",
                      "focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
                      active ? "bg-ink text-paper-on" : "surface-control",
                    ].join(" ")}
                  >
                    <span className="flex w-full items-center gap-2">
                      <span className="truncate text-[17px] leading-tight font-extrabold tabular-nums">{v.id}</span>
                      {installed && (
                        <span
                          title="已安装"
                          className={`ml-auto shrink-0 ${active ? "text-paper-on/70" : "text-accent"}`}
                        >
                          <CheckIcon size={14} />
                        </span>
                      )}
                    </span>
                    <span
                      className={`mt-1 font-mono text-[11px] tabular-nums ${active ? "text-paper-on/55" : "text-ink/75"}`}
                    >
                      {v.release_time.slice(0, 10)}
                    </span>
                  </button>
                </motion.li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

// ============ 内容类（Mod/整合包/资源包/光影）============
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
        title={state === "installed" ? "本次已安装到所选实例" : undefined}
      >
        {INSTALL_LABEL[state]}
      </Button>
    </motion.li>
  );
}

function ContentTab({ type }: { type: ResourceType }) {
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

  // 安装流程两段式：先选实例与版本（picking），再确认依赖计划（planning），最后真正下载。
  const [picking, setPicking] = useState<SearchHit | null>(null);
  const [planning, setPlanning] = useState<{ hit: SearchHit; versionId: string; modVersionId: string } | null>(
    null,
  );
  // 按 平台+工程 记安装态，供按钮状态机使用。仅本次会话有效——跨实例的真实已装状态由落位层逐实例标注。
  const [installState, setInstallState] = useState<Record<string, InstallState>>({});

  const loaders = useMemo<ModLoader[]>(() => (loaderFilter === "all" ? [] : [loaderFilter]), [loaderFilter]);
  // 加载器筛选只对 Mod / 整合包有意义，资源包与光影不挂加载器。
  const showLoaderFilter = type === "mod" || type === "modpack";

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
    const { hit, versionId, modVersionId } = planning;
    const key = keyOf(hit);
    setPlanning(null);
    setInstallState((s) => ({ ...s, [key]: "installing" }));
    try {
      const outcome = await installMod(versionId, hit.platform, hit.project_id, modVersionId);
      setInstallState((s) => ({ ...s, [key]: "installed" }));
      toast(`已安装 ${outcome.file_name} 到 ${versionId}`, "success");
    } catch (e) {
      // 装失败要退回可重试态，否则按钮会永远卡在「安装中」。
      setInstallState((s) => ({ ...s, [key]: "idle" }));
      toast(String(e), "error");
    }
  }, [planning, toast]);

  return (
    <div>
      {/* relative z-20 是必须的，不是保险：面板带 backdrop-filter 就自成层叠上下文，
          Select 的下拉浮层再高的 z-index 也只在这块面板内部有效；
          不把整块面板抬到结果卡片之上，下拉就会被下面那排同样自成上下文的卡片盖住。 */}
      <div className="surface-panel relative z-20 mb-4 flex items-center gap-2 rounded-panel p-1.5">
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

      {/* 第一段：选实例与版本。后端一次算出兼容矩阵，用户不必先导航进实例。 */}
      {picking && (
        <InstancePicker
          open
          platform={picking.platform}
          projectId={picking.project_id}
          title={picking.title}
          onClose={() => setPicking(null)}
          onConfirm={(versionId, modVersionId) => {
            setPlanning({ hit: picking, versionId, modVersionId });
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
            versionId={planning.versionId}
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

function ManagedModpackInstallTab({ initialPointerUrl }: { initialPointerUrl: string | null }) {
  const navigate = useNavigate();
  const [state, setState] = useState<ModpackInstallState>({ kind: "idle" });

  const install = useCallback(async (pointerUrl: string) => {
    setState({
      kind: "running",
      pointer_url: pointerUrl,
      target_version: "latest",
      progress: INITIAL_MODPACK_INSTALL_PROGRESS,
    });

    try {
      const outcome = await installManagedModpack(pointerUrl, (progress) => {
        setState({
          kind: "running",
          pointer_url: pointerUrl,
          target_version: "latest",
          progress,
        });
      });
      setState({
        kind: "complete",
        pointer_url: pointerUrl,
        instance_id: outcome.instance_id,
        installed_version: outcome.installed_version,
      });
    } catch (e) {
      const structured = parseModpackSyncError(e);
      if (structured) {
        setState({
          kind: "failed",
          pointer_url: pointerUrl,
          target_version: structured.target_version,
          problem: {
            kind: "sync",
            stage: structured.stage,
            failure: structured.failure,
          },
        });
      } else {
        setState({
          kind: "failed",
          pointer_url: pointerUrl,
          target_version: null,
          problem: {
            kind: "setup",
            failure: {
              stage: "resolving_manifest",
              title: "无法开始安装",
              detail: String(e),
              action: "确认整合包地址可访问后重试；若仍失败，请把错误详情发给整合包维护者。",
            },
          },
        });
      }
    }
  }, []);

  return (
    <ModpackInstallFlow
      builtIn={BUILT_IN_MODPACK}
      initialPointerUrl={initialPointerUrl ?? undefined}
      state={state}
      onInstall={(pointerUrl) => void install(pointerUrl)}
      onOpenInstance={(instanceId) => navigate(`/versions/${encodeURIComponent(instanceId)}`)}
    />
  );
}

export function Download() {
  const [searchParams] = useSearchParams();
  const initialPointerUrl = managedModpackPointerFromSearch(searchParams);
  const [tab, setTab] = useState<TabKey>(() =>
    searchParams.get("tab") === "modpack" ? "modpack" : "version",
  );
  const active = TABS.find((t) => t.key === tab)!;

  return (
    <>
      {/* 抬头与 tab 合成同一块面板：原先那条贯通的分割线现在由面板自己的下缘充当，
          下划线正落在这条缘上——少画一条线，也省掉「线在图上飘着」的那种廉价感。 */}
      <motion.div variants={pageItem} className="surface-panel mb-5 rounded-panel px-5 pt-4">
        <div className="flex items-baseline gap-4">
          <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">下载</h1>
          <span className="text-[12px] text-ink/75">游戏版本、Mod 与各类资源</span>
        </div>

        {/* 分段 tab：选中下划线用共享 layoutId，切换时在标签之间滑过去而不是闪现。 */}
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
      </motion.div>

      <motion.div variants={pageItem} className="min-h-0 flex-1">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={tab}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={springs.tap}
          >
            {active.key === "version" ? (
              <VersionTab />
            ) : active.key === "modpack" ? (
              <ManagedModpackInstallTab initialPointerUrl={initialPointerUrl} />
            ) : (
              <ContentTab type={active.type!} />
            )}
          </motion.div>
        </AnimatePresence>
      </motion.div>
    </>
  );
}
