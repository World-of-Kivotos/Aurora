// 资源页：目录式“选题墙”——搜索并浏览 Mod/资源包等，操作显性外露；本地已装模组可启禁。
// 真调 IPC：searchResources 拉取跨平台聚合结果，listInstalled/listMods/setModEnabled 管理本地模组。
// 诚实缺口：从搜索结果直接安装需 mod-version-id，当前 IPC 无“取某项目版本列表”端点，故“安装”按钮禁用不伪装。
// 外链打开走 @tauri-apps/plugin-opener 的 openUrl（非 invoke 命令，不属于 ipc.ts 契约面）。

import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Select } from "../components/Select";
import { Toggle } from "../components/Toggle";
import { useToast } from "../components/Toast";
import { AlertIcon, PackageIcon, RefreshIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  listInstalled,
  listMods,
  searchResources,
  setModEnabled,
  type InstalledMod,
  type ModLoader,
  type PlatformId,
  type ResourceType,
  type SearchHit,
  type SortField,
  type VersionScanDto,
} from "../lib/ipc";

const RESOURCE_TYPE_OPTIONS: { value: ResourceType; label: string }[] = [
  { value: "mod", label: "模组" },
  { value: "modpack", label: "整合包" },
  { value: "resource_pack", label: "资源包" },
  { value: "shader", label: "光影" },
  { value: "data_pack", label: "数据包" },
  { value: "plugin", label: "插件" },
];

const SORT_OPTIONS: { value: SortField; label: string }[] = [
  { value: "relevance", label: "相关度" },
  { value: "downloads", label: "下载量" },
  { value: "follows", label: "关注数" },
  { value: "newest", label: "最新发布" },
  { value: "updated", label: "最近更新" },
];

// "any" 是前端占位值，代表不加加载器过滤；其余映射到后端 ModLoader 联合。
type LoaderFilter = "any" | ModLoader;
const LOADER_OPTIONS: { value: LoaderFilter; label: string }[] = [
  { value: "any", label: "全部加载器" },
  { value: "fabric", label: "Fabric" },
  { value: "quilt", label: "Quilt" },
  { value: "forge", label: "Forge" },
  { value: "neoforge", label: "NeoForge" },
  { value: "liteloader", label: "LiteLoader" },
];

const PLATFORM_LABEL: Record<PlatformId, string> = {
  modrinth: "Modrinth",
  curseforge: "CurseForge",
};

// 千分位太占宽，选题墙里用 1.2K / 3.4M 紧凑记法；保留一位小数并去掉多余的 .0。
function formatCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, "")}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(/\.0$/, "")}K`;
  return String(n);
}

function modDisplayName(mod: InstalledMod): string {
  return mod.metadata?.name ?? mod.file_name;
}

export function Resources() {
  const { toast } = useToast();

  // ---- 搜索区状态 ----
  const [query, setQuery] = useState("");
  const [resourceType, setResourceType] = useState<ResourceType>("mod");
  const [sort, setSort] = useState<SortField>("relevance");
  const [loader, setLoader] = useState<LoaderFilter>("any");

  const [hits, setHits] = useState<SearchHit[]>([]);
  const [searchErrors, setSearchErrors] = useState<{ platform: PlatformId; message: string }[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);
  const [searched, setSearched] = useState(false);

  const runSearch = useCallback(async () => {
    setSearching(true);
    setSearchError(null);
    try {
      const result = await searchResources({
        query: query.trim() || undefined,
        resourceType,
        sort,
        loaders: loader === "any" ? undefined : [loader],
        limit: 30,
      });
      setHits(result.hits);
      setSearchErrors(result.errors);
      setSearched(true);
    } catch (e) {
      // 错误自然冒泡到页面统一展示，不吞。
      setSearchError(String(e));
    } finally {
      setSearching(false);
    }
  }, [query, resourceType, sort, loader]);

  const openHitPage = useCallback(
    async (url: string) => {
      try {
        await openUrl(url);
      } catch (e) {
        toast(String(e), "error");
      }
    },
    [toast],
  );

  // ---- 本地模组管理状态 ----
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [instancesError, setInstancesError] = useState<string | null>(null);
  const [instance, setInstance] = useState("");
  const [mods, setMods] = useState<InstalledMod[]>([]);
  const [modsLoading, setModsLoading] = useState(false);
  const [modsError, setModsError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);

  const loadInstances = useCallback(async () => {
    setInstancesError(null);
    try {
      setScan(await listInstalled());
    } catch (e) {
      setInstancesError(String(e));
    }
  }, []);

  useEffect(() => {
    void loadInstances();
  }, [loadInstances]);

  const loadMods = useCallback(async (versionId: string) => {
    setModsLoading(true);
    setModsError(null);
    try {
      setMods(await listMods(versionId));
    } catch (e) {
      setModsError(String(e));
    } finally {
      setModsLoading(false);
    }
  }, []);

  const handleInstanceChange = useCallback(
    (id: string) => {
      setInstance(id);
      setMods([]);
      if (id) void loadMods(id);
    },
    [loadMods],
  );

  // 启禁后文件名会随 .disabled 后缀变化，故不做乐观改名，直接重载该实例模组列表拿最新态。
  const handleToggleMod = useCallback(
    async (mod: InstalledMod, next: boolean) => {
      setPending(mod.file_name);
      try {
        await setModEnabled(instance, mod.file_name, next);
        toast(`${modDisplayName(mod)} 已${next ? "启用" : "禁用"}`, "success");
        await loadMods(instance);
      } catch (e) {
        toast(String(e), "error");
      } finally {
        setPending(null);
      }
    },
    [instance, loadMods, toast],
  );

  const instanceOptions = (scan?.versions ?? []).map((v) => ({ value: v.id, label: v.id }));
  const currentTypeLabel =
    RESOURCE_TYPE_OPTIONS.find((o) => o.value === resourceType)?.label ?? "资源";

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title="资源" subtitle="搜索并安装 Mod、资源包等" />
      </motion.div>

      {/* 搜索台：输入 + 类型/排序/加载器过滤，操作外露 */}
      <motion.section variants={pageItem} aria-label="搜索资源" className="mb-[34px]">
        <div className="flex flex-wrap items-end gap-3">
          <div className="min-w-[220px] flex-1">
            <label
              htmlFor="resource-query"
              className="mb-1.5 block text-[10.5px] font-bold tracking-[0.16em] text-ink/40"
            >
              关键词
            </label>
            <input
              id="resource-query"
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void runSearch();
              }}
              placeholder={`搜索${currentTypeLabel}…`}
              className="w-full rounded-[3px] border border-ink/16 bg-paper px-3.5 py-2.5 text-[14px] text-ink transition-colors placeholder:text-ink/45 hover:border-ink/40 focus-visible:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
            />
          </div>
          <div className="w-[140px]">
            <label className="mb-1.5 block text-[10.5px] font-bold tracking-[0.16em] text-ink/40">
              类型
            </label>
            <Select
              value={resourceType}
              onChange={setResourceType}
              options={RESOURCE_TYPE_OPTIONS}
              ariaLabel="资源类型"
            />
          </div>
          <div className="w-[130px]">
            <label className="mb-1.5 block text-[10.5px] font-bold tracking-[0.16em] text-ink/40">
              排序
            </label>
            <Select value={sort} onChange={setSort} options={SORT_OPTIONS} ariaLabel="排序方式" />
          </div>
          <div className="w-[150px]">
            <label className="mb-1.5 block text-[10.5px] font-bold tracking-[0.16em] text-ink/40">
              加载器
            </label>
            <Select
              value={loader}
              onChange={setLoader}
              options={LOADER_OPTIONS}
              ariaLabel="加载器过滤"
            />
          </div>
          <Button
            variant="primary"
            onClick={() => void runSearch()}
            disabled={searching}
            className="py-2.5"
          >
            {searching ? "搜索中…" : "搜索"}
          </Button>
        </div>

        {searchError && (
          <Card className="mt-4 flex items-center gap-4 border-danger/40">
            <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
              <AlertIcon />
            </span>
            <span className="flex-1 text-[13px] text-danger">{searchError}</span>
            <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void runSearch()}>
              重试
            </Button>
          </Card>
        )}

        {/* 部分平台失败：结果仍可用，仅提示降级 */}
        {searchErrors.length > 0 && (
          <p className="mt-3 font-mono text-[11.5px] text-ink/40">
            {searchErrors.map((e) => `${PLATFORM_LABEL[e.platform]} 未返回：${e.message}`).join("；")}
          </p>
        )}

        <div className="mt-6">
          {searching ? (
            <p className="font-mono text-[12px] text-ink/60">正在检索…</p>
          ) : !searched ? (
            <EmptyState icon={<PackageIcon />} title="输入关键词并点击搜索，浏览社区资源" />
          ) : hits.length === 0 ? (
            <EmptyState icon={<PackageIcon />} title="没有匹配的结果，换个关键词或过滤条件试试" />
          ) : (
            <ul className="m-0 grid list-none grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4 p-0">
              {hits.map((hit) => (
                <li key={`${hit.platform}-${hit.project_id}`}>
                  <Card className="flex h-full flex-col gap-3">
                    <div className="flex items-start gap-3">
                      {hit.icon_url ? (
                        <img
                          src={hit.icon_url}
                          alt=""
                          className="h-12 w-12 shrink-0 rounded-[3px] border border-ink/9 object-cover"
                        />
                      ) : (
                        <span className="grid h-12 w-12 shrink-0 place-items-center rounded-[3px] bg-ink text-paper-on [&_svg]:h-6 [&_svg]:w-6">
                          <PackageIcon />
                        </span>
                      )}
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-[15px] font-extrabold leading-tight">
                          {hit.title}
                        </div>
                        <div className="mt-1 truncate text-[12px] text-ink/60">
                          {hit.author ?? "未知作者"}
                        </div>
                      </div>
                      <span className="shrink-0 rounded-[2px] border border-ink/16 px-2 py-[3px] text-[10px] font-bold tracking-[0.1em] text-ink/60">
                        {PLATFORM_LABEL[hit.platform]}
                      </span>
                    </div>

                    <p className="line-clamp-2 min-h-[2.4em] text-[12.5px] leading-snug text-ink/60">
                      {hit.description}
                    </p>

                    <div className="mt-auto flex items-center justify-between gap-3 pt-1">
                      <span className="font-mono text-[11.5px] text-ink/40 tabular-nums">
                        {formatCount(hit.downloads)} 次下载
                      </span>
                      <div className="flex items-center gap-2">
                        <Button
                          variant="secondary"
                          disabled={!hit.page_url}
                          onClick={() => hit.page_url && void openHitPage(hit.page_url)}
                        >
                          打开页面
                        </Button>
                        <Button
                          variant="secondary"
                          disabled
                          title="选择实例与版本后安装（即将支持）"
                        >
                          安装
                        </Button>
                      </div>
                    </div>
                  </Card>
                </li>
              ))}
            </ul>
          )}
        </div>
      </motion.section>

      {/* 本地模组管理：选实例 → 列出已装模组 → 逐项启禁 */}
      <motion.section variants={pageItem} aria-label="本地模组">
        <div className="mb-4 flex flex-wrap items-end justify-between gap-4 border-b border-ink/16 pb-[11px]">
          <div>
            <h2 className="text-[19px] font-extrabold">本地模组</h2>
            <p className="mt-1 text-[12.5px] text-ink/60">选择实例以启用或禁用已安装的模组</p>
          </div>
          <div className="w-[240px]">
            <Select
              value={instance}
              onChange={handleInstanceChange}
              options={instanceOptions}
              placeholder="选择实例"
              disabled={instanceOptions.length === 0}
              ariaLabel="选择实例"
            />
          </div>
        </div>

        {instancesError ? (
          <Card className="flex items-center gap-4 border-danger/40">
            <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
              <AlertIcon />
            </span>
            <span className="flex-1 text-[13px] text-danger">{instancesError}</span>
            <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void loadInstances()}>
              重试
            </Button>
          </Card>
        ) : instanceOptions.length === 0 ? (
          <EmptyState icon={<PackageIcon />} title="尚未安装任何版本，先在版本页安装一个实例" />
        ) : !instance ? (
          <EmptyState icon={<PackageIcon />} title="从上方选择一个实例查看其已装模组" />
        ) : modsError ? (
          <Card className="flex items-center gap-4 border-danger/40">
            <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
              <AlertIcon />
            </span>
            <span className="flex-1 text-[13px] text-danger">{modsError}</span>
            <Button
              variant="secondary"
              icon={<RefreshIcon />}
              onClick={() => void loadMods(instance)}
            >
              重试
            </Button>
          </Card>
        ) : modsLoading ? (
          <p className="font-mono text-[12px] text-ink/60">正在读取模组…</p>
        ) : mods.length === 0 ? (
          <EmptyState icon={<PackageIcon />} title="该实例还没有安装任何模组" />
        ) : (
          <ul className="m-0 list-none p-0">
            {mods.map((mod) => (
              <li
                key={mod.file_name}
                className="flex items-center justify-between gap-6 border-b border-ink/9 py-[15px] last:border-b-0"
              >
                <div className="min-w-0">
                  <div className="flex items-baseline gap-2">
                    <span className="truncate text-[15px] font-bold">{modDisplayName(mod)}</span>
                    {mod.metadata?.version && (
                      <span className="shrink-0 font-mono text-[11.5px] text-ink/40 tabular-nums">
                        {mod.metadata.version}
                      </span>
                    )}
                    {!mod.enabled && (
                      <span className="shrink-0 rounded-[2px] border border-ink/16 px-1.5 py-0.5 text-[10px] font-bold tracking-[0.1em] text-ink/45">
                        已禁用
                      </span>
                    )}
                  </div>
                  <div className="mt-1 truncate font-mono text-[11.5px] text-ink/40">
                    {mod.file_name}
                  </div>
                </div>
                <Toggle
                  ariaLabel={`${modDisplayName(mod)} 启用开关`}
                  checked={mod.enabled}
                  disabled={pending === mod.file_name}
                  onChange={(next) => void handleToggleMod(mod, next)}
                />
              </li>
            ))}
          </ul>
        )}
      </motion.section>
    </>
  );
}
