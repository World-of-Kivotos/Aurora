// 下载中心：把「游戏版本」与「Mod/整合包/资源包/光影」并进一个页面（PCL2 式），按用户「要下东西」的心智组织。
// 分段 tab 切换内容；游戏版本 = 已安装管理 + 安装新版本；内容类 = 聚合搜索安装。简约撞色，轻量顶部。

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import { Button } from "../components/Button";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { useToast } from "../components/Toast";
import { PackageIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  installVersion,
  listInstalled,
  listManifest,
  searchResources,
  type LoaderChoice,
  type ManifestDto,
  type ResourceType,
  type SearchResultDto,
  type VersionScanDto,
} from "../lib/ipc";

type TabKey = "version" | "mod" | "modpack" | "resourcepack" | "shader";
const TABS: { key: TabKey; label: string; type?: ResourceType }[] = [
  { key: "version", label: "游戏版本" },
  { key: "mod", label: "Mod", type: "mod" },
  { key: "modpack", label: "整合包", type: "modpack" },
  { key: "resourcepack", label: "资源包", type: "resource_pack" },
  { key: "shader", label: "光影", type: "shader" },
];

const INPUT =
  "w-full rounded-[3px] border border-ink/14 bg-paper px-3.5 py-2.5 text-[14px] text-ink outline-none transition-colors placeholder:text-ink/35 hover:border-ink/30 focus:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

function fmt(n: number) {
  return n >= 1e6 ? (n / 1e6).toFixed(1) + "M" : n >= 1e3 ? (n / 1e3).toFixed(0) + "K" : String(n);
}

// 资源图标：优先项目 icon_url，加载失败/缺省回落名字首字母墨块。
function ResIcon({ url, title }: { url: string | null; title: string }) {
  const [ok, setOk] = useState(!!url);
  if (!ok || !url) {
    return (
      <span className="grid h-11 w-11 shrink-0 place-items-center rounded-[3px] bg-ink text-[16px] font-extrabold text-paper-on">
        {title.slice(0, 1).toUpperCase()}
      </span>
    );
  }
  return (
    <img
      src={url}
      alt=""
      width={44}
      height={44}
      loading="lazy"
      onError={() => setOk(false)}
      className="h-11 w-11 shrink-0 rounded-[3px] object-cover"
    />
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

function VersionTab() {
  const { toast } = useToast();
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [manifest, setManifest] = useState<ManifestDto | null>(null);
  const [search, setSearch] = useState("");
  const [pick, setPick] = useState<string | null>(null);
  const [loader, setLoader] = useState<"none" | LoaderChoice>("none");
  const [installing, setInstalling] = useState(false);

  const load = useCallback(async () => {
    const [sc, mf] = await Promise.all([listInstalled(), listManifest()]);
    setScan(sc);
    setManifest(mf);
  }, []);
  useEffect(() => {
    void load();
  }, [load]);

  const installedIds = useMemo(() => new Set((scan?.versions ?? []).map((v) => v.id)), [scan]);
  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    return (manifest?.versions ?? [])
      .filter((v) => v.release_type === "release" && (!q || v.id.includes(q)))
      .slice(0, 80);
  }, [manifest, search]);

  const doInstall = async () => {
    if (!pick) return;
    setInstalling(true);
    try {
      await installVersion(pick, loader === "none" ? undefined : loader);
      toast(`已安装 ${pick}`, "success");
      setPick(null);
      await load();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setInstalling(false);
    }
  };

  return (
    <div className="min-w-0">
      {/* 选中后：顶部一条安装配置（各窗宽通用） */}
      {pick && (
        <div className="mb-5 flex items-center gap-4 rounded-[3px] border border-ink/10 bg-paper-sink px-4 py-3">
          <div className="min-w-0">
            <div className="text-[10px] font-bold tracking-[0.2em] text-ink/40">目标版本</div>
            <div className="mt-0.5 truncate text-[22px] font-extrabold tabular-nums">{pick}</div>
          </div>
          <div className="ml-auto w-[150px] shrink-0">
            <Select<"none" | LoaderChoice>
              value={loader}
              onChange={setLoader}
              options={LOADER_OPTIONS}
              ariaLabel="加载器"
            />
          </div>
          <Button variant="primary" className="shrink-0" onClick={() => void doInstall()} disabled={installing}>
            {installing ? "安装中…" : "安装"}
          </Button>
        </div>
      )}

      <input
        value={search}
        onChange={(e) => setSearch(e.target.value)}
        placeholder="搜索版本号安装，如 1.20"
        className={INPUT}
      />
      {manifest ? (
        <ul className="mt-3 m-0 list-none p-0">
          {filtered.map((v) => {
            const active = v.id === pick;
            return (
              <li key={v.id}>
                <button
                  type="button"
                  onClick={() => setPick(v.id)}
                  className={[
                    "flex w-full items-center justify-between border-b border-ink/8 px-2 py-2.5 text-left transition-colors",
                    active ? "bg-ink text-paper-on" : "hover:bg-ink/[0.03]",
                  ].join(" ")}
                >
                  <span className="text-[16px] font-bold tabular-nums">{v.id}</span>
                  {installedIds.has(v.id) && (
                    <span className={active ? "text-[10px] text-paper-on/70" : "text-[10px] font-bold text-accent"}>
                      已安装
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="mt-3 font-mono text-[12px] text-ink/40">正在拉取版本清单…</p>
      )}
    </div>
  );
}

// ============ 内容类（Mod/整合包/资源包/光影）============
const SORT_OPTIONS = [
  { value: "relevance" as const, label: "相关度" },
  { value: "downloads" as const, label: "下载量" },
  { value: "updated" as const, label: "最近更新" },
];

function ContentTab({ type }: { type: ResourceType }) {
  const { toast } = useToast();
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<"relevance" | "downloads" | "updated">("relevance");
  const [result, setResult] = useState<SearchResultDto | null>(null);
  const [loading, setLoading] = useState(false);

  const run = useCallback(async () => {
    setLoading(true);
    try {
      setResult(
        await searchResources({ query, resourceType: type, loaders: [], gameVersions: [], sort, limit: 30, offset: 0 }),
      );
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setLoading(false);
    }
  }, [query, type, sort, toast]);

  // 切换 tab 或排序时自动搜一次
  useEffect(() => {
    void run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [type, sort]);

  const hits = result?.hits ?? [];
  return (
    <div>
      <div className="mb-5 flex items-center gap-3">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void run()}
          placeholder={`搜索${TABS.find((t) => t.type === type)?.label ?? ""}…`}
          className={INPUT}
        />
        <div className="w-[120px] shrink-0">
          <Select value={sort} onChange={setSort} options={SORT_OPTIONS} ariaLabel="排序" />
        </div>
        <Button variant="primary" className="h-10 shrink-0 !py-0" onClick={() => void run()} disabled={loading}>
          搜索
        </Button>
      </div>

      {loading ? (
        <p className="font-mono text-[12px] text-ink/40">搜索中…</p>
      ) : hits.length === 0 ? (
        <EmptyState icon={<PackageIcon />} title="没有结果，换个关键词试试" />
      ) : (
        <ul className="m-0 grid list-none grid-cols-2 gap-2.5 p-0 max-[1120px]:grid-cols-1">
          {hits.map((h) => (
            <li
              key={h.platform + h.project_id}
              className="flex items-start gap-3.5 rounded-[3px] border border-ink/10 bg-paper-sink p-3.5"
            >
              <ResIcon url={h.icon_url} title={h.title} />
              <div className="min-w-0 flex-1">
                <div className="flex items-baseline gap-2">
                  <span className="truncate text-[15px] font-bold">{h.title}</span>
                  {h.author && <span className="shrink-0 text-[11px] text-ink/40">{h.author}</span>}
                </div>
                <p className="mt-0.5 line-clamp-2 text-[12.5px] leading-snug text-ink/55">{h.description}</p>
                <div className="mt-1.5 flex items-center gap-2 text-[11px] text-ink/40">
                  <span className="font-mono tabular-nums">{fmt(h.downloads)} 下载</span>
                  <span className="rounded-[2px] bg-ink/6 px-1.5 py-0.5 uppercase tracking-wide">{h.platform}</span>
                </div>
              </div>
              <Button variant="secondary" className="shrink-0" onClick={() => toast("安装需选择目标实例（接线中）", "error")}>
                安装
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

export function Download() {
  const [tab, setTab] = useState<TabKey>("version");
  const active = TABS.find((t) => t.key === tab)!;

  return (
    <>
      {/* 轻量顶部 */}
      <motion.div variants={pageItem} className="mb-5 flex items-baseline gap-4">
        <h1 className="text-[20px] font-extrabold tracking-[-0.01em]">下载</h1>
        <span className="text-[12px] text-ink/35">游戏版本、Mod 与各类资源</span>
      </motion.div>

      {/* 分段 tab */}
      <motion.div variants={pageItem} className="mb-7 flex gap-6 border-b border-ink/10">
        {TABS.map((t) => {
          const on = t.key === tab;
          return (
            <button
              key={t.key}
              type="button"
              onClick={() => setTab(t.key)}
              className={[
                "relative -mb-px pb-2.5 text-[15px] transition-colors",
                on ? "font-extrabold text-ink" : "font-semibold text-ink/40 hover:text-ink/70",
              ].join(" ")}
            >
              {t.label}
              {on && <span className="absolute inset-x-0 -bottom-px h-[2px] bg-accent" />}
            </button>
          );
        })}
      </motion.div>

      <motion.div variants={pageItem} className="min-h-0 flex-1">
        {active.key === "version" ? <VersionTab /> : <ContentTab type={active.type!} />}
      </motion.div>
    </>
  );
}
