// 下载中心：游戏版本 + Mod/整合包/资源包/光影 合一（PCL2 式心智——用户来这里就是"要拿新东西"）。
// 取数一律走 lib/prefetch：启动器起来时已在空闲期把五个 tab 的首屏拉过一遍，命中缓存直接出内容、不闪骨架；
// 未命中（冷启动或换了筛选参数）才走真请求并显示骨架。错误不吞，原样呈现并给重试。

import { useCallback, useEffect, useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { InstancePicker } from "../components/InstancePicker";
import { InstallPlanPreview } from "../components/InstallPlanPreview";
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
      <span className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink/30">
        <SearchIcon size={16} />
      </span>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && onEnter?.()}
        placeholder={placeholder}
        className={`${CTRL} w-full rounded-[3px] border border-ink/14 bg-paper pr-3 pl-9 text-[14px] text-ink transition-colors outline-none placeholder:text-ink/35 hover:border-ink/30 focus:border-ink focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2`}
      />
    </div>
  );
}

// ---- 通用：分段筛选（墨底选中，撞色）----
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
    <div role="group" aria-label={ariaLabel} className={`${CTRL} flex shrink-0 items-center gap-1 rounded-[3px] bg-ink/[0.05] p-1`}>
      {options.map((o) => {
        const on = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            onClick={() => onChange(o.value)}
            aria-pressed={on}
            className={[
              "relative h-full cursor-pointer rounded-[2px] px-3 text-[13px] font-bold transition-colors",
              on ? "text-paper-on" : "text-ink/45 hover:text-ink/75",
            ].join(" ")}
          >
            {on && (
              <motion.span
                layoutId={`seg-${ariaLabel}`}
                className="absolute inset-0 rounded-[2px] bg-ink"
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
    <div className="mb-4 flex items-center gap-3 rounded-[3px] border border-danger/35 bg-danger/[0.04] px-4 py-3">
      <span className="text-danger [&_svg]:h-[18px] [&_svg]:w-[18px]">
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
      <div className="mb-4 flex items-center gap-3">
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="搜索版本号，如 1.20"
          className="w-[300px] shrink-0"
        />
        <Segmented value={kind} onChange={setKind} options={KIND_OPTIONS} ariaLabel="版本类型" />
        <span className="ml-auto shrink-0 font-mono text-[12px] text-ink/40 tabular-nums">
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
            className="overflow-hidden"
          >
            <div className="mb-4 flex items-center gap-4 rounded-[3px] border border-ink bg-paper-sink px-4 py-3">
              <div className="min-w-0">
                <div className="text-[10px] font-bold tracking-[0.2em] text-ink/40">即将安装</div>
                <div className="mt-0.5 truncate text-[21px] leading-tight font-extrabold tabular-nums">{pick}</div>
              </div>
              <div className="ml-auto w-[150px] shrink-0">
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

      {!manifest ? (
        <div className="grid grid-cols-4 gap-2 max-[1180px]:grid-cols-3">
          {Array.from({ length: 12 }, (_, i) => (
            <VersionCardSkeleton key={i} delay={i * 0.05} />
          ))}
        </div>
      ) : shown.length === 0 ? (
        <EmptyState icon={<CubeIcon />} title="没有匹配的版本" />
      ) : (
        <ul className="m-0 grid list-none grid-cols-4 gap-2 p-0 max-[1180px]:grid-cols-3">
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
                <button
                  type="button"
                  onClick={() => setPick(active ? null : v.id)}
                  aria-pressed={active}
                  className={[
                    "flex w-full cursor-pointer flex-col items-start rounded-[3px] border px-3.5 py-3 text-left transition-colors",
                    "focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
                    active
                      ? "border-ink bg-ink text-paper-on"
                      : "border-ink/10 bg-paper-sink hover:border-ink/35",
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
                    className={`mt-1 font-mono text-[11px] tabular-nums ${active ? "text-paper-on/55" : "text-ink/40"}`}
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

/** 资源图标：优先项目图标，加载失败/缺省回落首字母墨块（保持与卡片同尺寸，不跳版）。 */
function ResIcon({ url, title }: { url: string | null; title: string }) {
  const [failed, setFailed] = useState(false);
  if (!url || failed) {
    return (
      <span className="grid h-12 w-12 shrink-0 place-items-center rounded-[3px] bg-ink text-[17px] font-extrabold text-paper-on">
        {title.slice(0, 1).toUpperCase()}
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      width={48}
      height={48}
      loading="lazy"
      onError={() => setFailed(true)}
      className="h-12 w-12 shrink-0 rounded-[3px] bg-ink/5 object-cover"
    />
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
      className="group flex items-start gap-3.5 rounded-[3px] border border-ink/10 bg-paper-sink p-3.5 transition-colors hover:border-ink/35"
    >
      <ResIcon url={hit.icon_url} title={hit.title} />

      <div className="flex min-w-0 flex-1 flex-col self-stretch">
        <div className="flex items-baseline gap-2">
          <span className="truncate text-[15px] leading-tight font-extrabold">{hit.title}</span>
          {hit.author && <span className="shrink-0 text-[11px] text-ink/40">{hit.author}</span>}
        </div>
        <p className="mt-1 line-clamp-2 text-[12.5px] leading-snug text-ink/55">{hit.description}</p>
        {/* mt-auto 把元信息压到卡片底部：同一行左右卡片的这条线因此始终齐平，与描述占一行还是两行无关。
            nowrap + 只留一个分类，避免标签换行把卡片撑出参差。 */}
        <div className="mt-auto flex flex-nowrap items-center gap-2.5 overflow-hidden pt-2 text-[11px] whitespace-nowrap text-ink/40">
          <span className="inline-flex shrink-0 items-center gap-1 font-mono tabular-nums">
            <DownloadIcon size={12} />
            {fmtCount(hit.downloads)}
          </span>
          <span className="shrink-0 rounded-[2px] bg-ink/[0.07] px-1.5 py-0.5 tracking-wide uppercase">
            {hit.platform}
          </span>
          {hit.categories.slice(0, 1).map((c) => (
            <span key={c} className="truncate rounded-[2px] bg-ink/[0.07] px-1.5 py-0.5">
              {c}
            </span>
          ))}
        </div>
      </div>

      <Button
        variant="secondary"
        // 已装/在装的按钮常亮，避免它随 hover 忽隐忽现让状态看起来不确定。
        className={[
          "shrink-0 transition-opacity",
          state === "idle" ? "opacity-55 group-hover:opacity-100" : "opacity-100",
        ].join(" ")}
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
      <div className="mb-4 flex items-center gap-3">
        <SearchField
          value={draft}
          onChange={setDraft}
          onEnter={() => setQuery(draft)}
          placeholder={`搜索${label}`}
          className="min-w-0 flex-1"
        />
        {showLoaderFilter && (
          <div className="w-[140px] shrink-0">
            <Select<"all" | ModLoader>
              value={loaderFilter}
              onChange={setLoaderFilter}
              options={LOADER_FILTER}
              ariaLabel="加载器筛选"
              className={CTRL}
            />
          </div>
        )}
        <div className="w-[124px] shrink-0">
          <Select<SortField> value={sort} onChange={setSort} options={SORT_OPTIONS} ariaLabel="排序" className={CTRL} />
        </div>
        <Button variant="primary" className={`${CTRL} shrink-0 !py-0`} onClick={() => setQuery(draft)} disabled={loading}>
          搜索
        </Button>
      </div>

      {error && <ErrorBar message={error} onRetry={() => void run()} />}

      {/* 上游部分平台失败时结果仍可用，把失败平台如实列出来而不是假装完整。 */}
      {result && result.errors.length > 0 && (
        <p className="mb-3 text-[12px] text-danger/85">
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
        <EmptyState icon={<PackageIcon />} title={query ? "没有结果，换个关键词试试" : `暂时没有可显示的${label}`} />
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

export function Download() {
  const [tab, setTab] = useState<TabKey>("version");
  const active = TABS.find((t) => t.key === tab)!;

  return (
    <>
      <motion.div variants={pageItem} className="mb-5 flex items-baseline gap-4">
        <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">下载</h1>
        <span className="text-[12px] text-ink/35">游戏版本、Mod 与各类资源</span>
      </motion.div>

      {/* 分段 tab：选中下划线用共享 layoutId，切换时在标签之间滑过去而不是闪现。 */}
      <motion.div variants={pageItem} className="mb-6 flex gap-1 border-b border-ink/10">
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
                "relative -mb-px flex cursor-pointer items-center gap-2 px-3 pb-2.5 text-[14px] transition-colors",
                "focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2",
                on ? "font-extrabold text-ink" : "font-semibold text-ink/40 hover:text-ink/70",
              ].join(" ")}
            >
              <Icon size={16} />
              {t.label}
              {on && (
                <motion.span
                  layoutId="download-tab-underline"
                  className="absolute inset-x-0 -bottom-px h-[2px] bg-accent"
                  transition={springs.tap}
                />
              )}
            </button>
          );
        })}
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
            {active.key === "version" ? <VersionTab /> : <ContentTab type={active.type!} />}
          </motion.div>
        </AnimatePresence>
      </motion.div>
    </>
  );
}
