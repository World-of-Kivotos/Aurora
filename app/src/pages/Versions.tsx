// 版本页：已安装清单 + 从官方清单安装新版本（可选加载器）。
// 真调 IPC：入场并行拉取 list_installed 与 list_manifest，两条链路各自独立的加载/错误态，
// 一条挂了不牵连另一条。安装链路遵循“先订阅 onCoreEvent 再 invoke，结束 unlisten”的进度事件范式。

import { useCallback, useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Select } from "../components/Select";
import { useToast } from "../components/Toast";
import { AlertIcon, LayersIcon, PackageIcon, RefreshIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  getConfig,
  installVersion,
  listInstalled,
  listManifest,
  onCoreEvent,
  updateConfig,
  type InstalledVersionDto,
  type LoaderChoice,
  type ManifestDto,
  type ManifestVersionDto,
  type VersionScanDto,
} from "../lib/ipc";

// 清单可达数千条，一次性渲染会卡顿；按类型/搜索过滤后只渲染前 RENDER_CAP 条。
const RENDER_CAP = 100;

type TypeFilter = "all" | "release" | "snapshot";
type LoaderPick = "none" | LoaderChoice;

const TYPE_FILTER_OPTIONS: { value: TypeFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "release", label: "正式版" },
  { value: "snapshot", label: "快照" },
];

const LOADER_OPTIONS: { value: LoaderPick; label: string }[] = [
  { value: "none", label: "无（原版）" },
  { value: "fabric", label: "Fabric" },
  { value: "quilt", label: "Quilt" },
  { value: "forge", label: "Forge" },
  { value: "neoforge", label: "NeoForge" },
];

// Mojang release_type -> 中文标签。快照桶仅取严格 "snapshot"，远古版本归入“全部”另标。
const RELEASE_TYPE_LABEL: Record<string, string> = {
  release: "正式版",
  snapshot: "快照",
  old_beta: "远古 Beta",
  old_alpha: "远古 Alpha",
};

// forge/neoforge 的加载器版本无默认可推断，必须由用户给定具体版本号。
const VERSION_REQUIRED: ReadonlySet<LoaderPick> = new Set<LoaderPick>(["forge", "neoforge"]);

// 版本 id 拆成主段 + 加载器后缀（"1.20.1-fabric" -> {base:"1.20.1", sfx:"-fabric"}），后缀以朱红呈现。
function splitId(id: string): { base: string; sfx: string } {
  const i = id.indexOf("-");
  return i < 0 ? { base: id, sfx: "" } : { base: id.slice(0, i), sfx: id.slice(i) };
}

function loaderText(v: InstalledVersionDto): string {
  if (v.loaders.length === 0) return "—";
  const l = v.loaders[0];
  return l.version ? `${l.kind} ${l.version}` : l.kind;
}

// 安装进度：stage/warning 给文案，download 事件额外给出可渲染的完成比。
interface Progress {
  text: string;
  ratio: number | null;
}

const inputClass =
  "w-full rounded-[3px] border border-ink/16 bg-paper px-3.5 py-2.5 text-[14px] text-ink transition-colors placeholder:text-ink/40 focus:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

export function Versions() {
  const { toast } = useToast();

  // ---- 已安装链路 ----
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [scanLoading, setScanLoading] = useState(true);
  const [scanError, setScanError] = useState<string | null>(null);
  // 当前选中的启动版本 id（config），与主页共享；点击已安装行写回 config。
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);

  // ---- 清单链路 ----
  const [manifest, setManifest] = useState<ManifestDto | null>(null);
  const [manifestLoading, setManifestLoading] = useState(true);
  const [manifestError, setManifestError] = useState<string | null>(null);

  // ---- 安装配置 ----
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("release");
  const [search, setSearch] = useState("");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loader, setLoader] = useState<LoaderPick>("none");
  const [loaderVersion, setLoaderVersion] = useState("");
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<Progress | null>(null);

  const loadInstalled = useCallback(async () => {
    setScanLoading(true);
    setScanError(null);
    try {
      setScan(await listInstalled());
    } catch (e) {
      // 错误自然冒泡到页面统一展示，不吞。
      setScanError(String(e));
    } finally {
      setScanLoading(false);
    }
  }, []);

  const loadManifest = useCallback(async () => {
    setManifestLoading(true);
    setManifestError(null);
    try {
      setManifest(await listManifest());
    } catch (e) {
      setManifestError(String(e));
    } finally {
      setManifestLoading(false);
    }
  }, []);

  // 读当前启动版本选择。失败不阻断本页主功能，仅提示；「当前」标记暂缺。
  const loadConfig = useCallback(async () => {
    try {
      const c = await getConfig();
      setSelectedVersion(c.selected_version);
    } catch (e) {
      toast(String(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void loadInstalled();
    void loadManifest();
    void loadConfig();
  }, [loadInstalled, loadManifest, loadConfig]);

  // 设为当前启动版本：乐观更新 + 写回 config，失败回滚。
  const setCurrent = useCallback(
    async (id: string) => {
      const prev = selectedVersion;
      setSelectedVersion(id);
      try {
        await updateConfig({ selectedVersion: id });
        toast("已设为当前启动版本", "success");
      } catch (e) {
        setSelectedVersion(prev);
        toast(String(e), "error");
      }
    },
    [selectedVersion, toast],
  );

  const installedIds = useMemo(
    () => new Set((scan?.versions ?? []).map((v) => v.id)),
    [scan],
  );

  const filtered = useMemo<ManifestVersionDto[]>(() => {
    const list = manifest?.versions ?? [];
    const q = search.trim().toLowerCase();
    return list.filter((v) => {
      if (typeFilter === "release" && v.release_type !== "release") return false;
      if (typeFilter === "snapshot" && v.release_type !== "snapshot") return false;
      if (q && !v.id.toLowerCase().includes(q)) return false;
      return true;
    });
  }, [manifest, typeFilter, search]);

  const shown = filtered.slice(0, RENDER_CAP);
  const overflow = filtered.length - shown.length;

  const selectVersion = useCallback((id: string) => {
    setSelectedId(id);
    setLoader("none");
    setLoaderVersion("");
  }, []);

  const versionRequired = VERSION_REQUIRED.has(loader);
  const installDisabled =
    installing ||
    !selectedId ||
    (loader !== "none" && versionRequired && loaderVersion.trim() === "");

  const handleInstall = useCallback(async () => {
    if (!selectedId) return;
    setInstalling(true);
    setProgress(null);
    // 先订阅进度事件流，再 invoke，确保安装期间的 stage/warning/download 不漏。
    const unlisten = await onCoreEvent((ev) => {
      if (ev.kind === "stage") {
        setProgress((prev) => ({ text: ev.message, ratio: prev?.ratio ?? null }));
      } else if (ev.kind === "warning") {
        setProgress((prev) => ({ text: `告警：${ev.message}`, ratio: prev?.ratio ?? null }));
      } else if (ev.kind === "download") {
        setProgress({
          text: `下载资源 ${ev.finished}/${ev.total}`,
          ratio: ev.total > 0 ? ev.finished / ev.total : null,
        });
      }
    });
    try {
      const choice = loader === "none" ? undefined : loader;
      const lv = loaderVersion.trim() === "" ? undefined : loaderVersion.trim();
      const outcome = await installVersion(selectedId, choice, lv);
      const installedName = outcome.loader?.id ?? outcome.vanilla.id;
      toast(`已安装 ${installedName}`, "success");
      await loadInstalled();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      unlisten();
      setInstalling(false);
      setProgress(null);
    }
  }, [selectedId, loader, loaderVersion, toast, loadInstalled]);

  const versions = scan?.versions ?? [];
  const broken = scan?.broken ?? [];
  const installedTotal = versions.length + broken.length;
  // 有效当前版本：选中项仍已安装则用它，否则回落扫描首项（与主页解析一致）。
  const effectiveCurrentId =
    selectedVersion && versions.some((v) => v.id === selectedVersion)
      ? selectedVersion
      : (versions[0]?.id ?? null);

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader
          title="版本"
          subtitle="安装与管理游戏版本"
          right={
            <>
              <div className="text-[10px] font-bold tracking-[0.22em] text-ink/40">已安装</div>
              <div className="mt-1.5 font-mono text-[12px] tracking-[0.08em] text-ink/60 tabular-nums">
                {scanLoading ? "扫描中" : `${String(installedTotal).padStart(2, "0")} 项`}
              </div>
            </>
          }
        />
      </motion.div>

      {/* ---- 已安装区 ---- */}
      <motion.section variants={pageItem} aria-label="已安装版本" className="mb-[38px]">
        <div className="mb-0.5 flex items-baseline justify-between border-b border-ink/16 pb-[11px]">
          <h2 className="text-[19px] font-extrabold">已安装</h2>
          <button
            type="button"
            onClick={() => void loadInstalled()}
            disabled={scanLoading}
            className="inline-flex items-center gap-1.5 text-[11px] font-bold tracking-[0.12em] text-ink/50 transition-colors hover:text-ink disabled:pointer-events-none disabled:opacity-45 [&_svg]:h-3.5 [&_svg]:w-3.5"
          >
            <RefreshIcon />
            刷新
          </button>
        </div>

        {scanError ? (
          <Card className="mt-4 flex items-center gap-4 border-danger/40">
            <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
              <AlertIcon />
            </span>
            <span className="flex-1 text-[13px] text-danger">{scanError}</span>
            <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void loadInstalled()}>
              重试
            </Button>
          </Card>
        ) : installedTotal > 0 ? (
          <ul className="m-0 list-none p-0">
            {versions.map((v) => {
              const s = splitId(v.id);
              const isCurrent = v.id === effectiveCurrentId;
              return (
                <li key={v.id}>
                  <button
                    type="button"
                    onClick={() => void setCurrent(v.id)}
                    aria-pressed={isCurrent}
                    title="设为当前启动版本"
                    className="flex w-full cursor-pointer items-baseline justify-between gap-6 border-b border-ink/9 py-[13px] text-left transition-colors last:border-b-0 hover:bg-paper-sink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                  >
                    <span className="flex items-baseline gap-2.5">
                      <span className="flex items-baseline gap-0.5 text-[24px] font-bold tracking-[-0.01em] tabular-nums">
                        {s.base}
                        {s.sfx && <span className="font-semibold text-ink/40">{s.sfx}</span>}
                      </span>
                      {isCurrent && (
                        <span className="shrink-0 self-center rounded-[2px] bg-accent/12 px-1.5 py-0.5 text-[10px] font-bold tracking-[0.1em] text-accent">
                          当前
                        </span>
                      )}
                    </span>
                    <span className="flex shrink-0 items-center gap-[14px]">
                      <span
                        className={
                          v.is_release
                            ? "rounded-[2px] border border-ink/16 px-[9px] py-1 text-[10.5px] font-bold tracking-[0.14em] text-ink/60"
                            : "rounded-[2px] bg-ink px-[9px] py-1 text-[10.5px] font-bold tracking-[0.14em] text-paper-on"
                        }
                      >
                        {v.is_release ? "正式版" : "快照"}
                      </span>
                      <span className="min-w-[118px] text-right font-mono text-[12px] text-ink/40 tabular-nums">
                        {loaderText(v)}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
            {broken.map((b) => {
              const s = splitId(b.id);
              return (
                <li
                  key={b.id}
                  className="flex items-baseline justify-between gap-6 border-b border-ink/9 py-[13px] last:border-b-0"
                >
                  <span className="flex items-baseline gap-0.5 text-[24px] font-bold tracking-[-0.01em] tabular-nums text-danger">
                    {s.base}
                    {s.sfx && <span className="font-semibold">{s.sfx}</span>}
                  </span>
                  <span className="flex shrink-0 items-center gap-[14px]">
                    <span className="rounded-[2px] border border-danger/50 px-[9px] py-1 text-[10.5px] font-bold tracking-[0.14em] text-danger">
                      损坏
                    </span>
                    <span className="min-w-[118px] text-right font-mono text-[12px] text-danger tabular-nums">
                      {b.reason}
                    </span>
                  </span>
                </li>
              );
            })}
          </ul>
        ) : (
          <div className="mt-2">
            <EmptyState
              icon={<LayersIcon />}
              title={scanLoading ? "正在扫描版本…" : "还没有安装任何版本，从下方清单挑一个开始"}
            />
          </div>
        )}
      </motion.section>

      {/* ---- 安装新版本区 ---- */}
      <motion.section variants={pageItem} aria-label="安装新版本">
        <div className="mb-[18px] flex items-baseline justify-between border-b border-ink/16 pb-[11px]">
          <h2 className="text-[19px] font-extrabold">安装新版本</h2>
          {manifest && (
            <span className="font-mono text-[11px] tracking-[0.14em] text-ink/40 tabular-nums">
              匹配 {String(filtered.length).padStart(3, "0")} 项
            </span>
          )}
        </div>

        {manifestError ? (
          <Card className="flex items-center gap-4 border-danger/40">
            <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
              <AlertIcon />
            </span>
            <span className="flex-1 text-[13px] text-danger">{manifestError}</span>
            <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void loadManifest()}>
              重试
            </Button>
          </Card>
        ) : (
          <div className="grid grid-cols-[1.1fr_0.9fr] gap-8 max-[960px]:grid-cols-1">
            {/* 左栏：过滤 + 清单 */}
            <div className="min-w-0">
              <div className="mb-3 flex items-center gap-3">
                <div className="w-[128px] shrink-0">
                  <Select<TypeFilter>
                    value={typeFilter}
                    onChange={setTypeFilter}
                    options={TYPE_FILTER_OPTIONS}
                    ariaLabel="版本类型过滤"
                  />
                </div>
                <input
                  type="text"
                  value={search}
                  onChange={(e) => setSearch(e.target.value)}
                  placeholder="搜索版本号，如 1.20"
                  aria-label="搜索版本号"
                  className={inputClass}
                />
              </div>

              {manifestLoading ? (
                <p className="py-6 font-mono text-[12px] text-ink/50">正在拉取版本清单…</p>
              ) : shown.length > 0 ? (
                <>
                  <ul className="m-0 max-h-[420px] list-none overflow-y-auto p-0">
                    {shown.map((v) => {
                      const active = v.id === selectedId;
                      const already = installedIds.has(v.id);
                      return (
                        <li key={v.id}>
                          <button
                            type="button"
                            disabled={installing}
                            onClick={() => selectVersion(v.id)}
                            className={[
                              "flex w-full items-center justify-between gap-4 border-b border-ink/9 px-2.5 py-[11px] text-left transition-colors",
                              "disabled:pointer-events-none disabled:opacity-45",
                              active ? "bg-ink text-paper-on" : "hover:bg-paper-sink",
                            ].join(" ")}
                          >
                            <span className="flex items-baseline gap-2.5 truncate">
                              <span className="text-[16px] font-bold tracking-[-0.01em] tabular-nums">
                                {v.id}
                              </span>
                              {already && (
                                <span
                                  className={[
                                    "shrink-0 text-[10px] font-bold tracking-[0.14em]",
                                    active ? "text-paper-on/70" : "text-accent",
                                  ].join(" ")}
                                >
                                  已安装
                                </span>
                              )}
                            </span>
                            <span
                              className={[
                                "shrink-0 text-[10.5px] font-bold tracking-[0.14em]",
                                active ? "text-paper-on/70" : "text-ink/45",
                              ].join(" ")}
                            >
                              {RELEASE_TYPE_LABEL[v.release_type] ?? v.release_type}
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                  {overflow > 0 && (
                    <p className="pt-3 font-mono text-[11px] text-ink/40">
                      仅显示前 {RENDER_CAP} 项，另有 {overflow} 项，请用搜索或类型过滤缩小范围
                    </p>
                  )}
                </>
              ) : (
                <div className="pt-2">
                  <EmptyState icon={<PackageIcon />} title="没有匹配的版本，试试调整过滤或搜索" />
                </div>
              )}
            </div>

            {/* 右栏：安装配置 */}
            <div className="min-w-0">
              {selectedId ? (
                <Card className="flex flex-col gap-5">
                  <div>
                    <div className="mb-1.5 text-[10px] font-bold tracking-[0.22em] text-ink/40">
                      目标版本
                    </div>
                    <div className="flex items-baseline gap-2 text-[30px] font-extrabold tracking-[-0.02em] tabular-nums">
                      {splitId(selectedId).base}
                      {splitId(selectedId).sfx && (
                        <span className="text-accent">{splitId(selectedId).sfx}</span>
                      )}
                      {installedIds.has(selectedId) && (
                        <span className="text-[11px] font-bold tracking-[0.14em] text-accent">
                          已安装
                        </span>
                      )}
                    </div>
                  </div>

                  <div>
                    <label className="mb-1.5 block text-[12px] font-bold tracking-[0.08em] text-ink/60">
                      加载器
                    </label>
                    <Select<LoaderPick>
                      value={loader}
                      onChange={(next) => {
                        setLoader(next);
                        setLoaderVersion("");
                      }}
                      options={LOADER_OPTIONS}
                      disabled={installing}
                      ariaLabel="加载器"
                    />
                  </div>

                  {loader !== "none" && (
                    <div>
                      <label className="mb-1.5 block text-[12px] font-bold tracking-[0.08em] text-ink/60">
                        加载器版本
                        {versionRequired ? (
                          <span className="ml-2 font-normal text-danger">必填</span>
                        ) : (
                          <span className="ml-2 font-normal text-ink/40">留空则用最新</span>
                        )}
                      </label>
                      <input
                        type="text"
                        value={loaderVersion}
                        onChange={(e) => setLoaderVersion(e.target.value)}
                        disabled={installing}
                        placeholder={versionRequired ? "如 47.2.20" : "可留空"}
                        aria-label="加载器版本"
                        className={`${inputClass} disabled:pointer-events-none disabled:opacity-45`}
                      />
                      {versionRequired && (
                        <p className="mt-1.5 text-[11px] text-ink/45">
                          Forge / NeoForge 无法自动推断版本，需填写具体版本号。
                        </p>
                      )}
                    </div>
                  )}

                  {installing && progress && (
                    <div>
                      <p className="mb-2 font-mono text-[12px] text-ink/60">{progress.text}</p>
                      <div className="h-[3px] w-full overflow-hidden rounded-[2px] bg-ink/10">
                        <motion.div
                          className="h-full bg-accent"
                          initial={false}
                          animate={{
                            width:
                              progress.ratio === null ? "40%" : `${Math.round(progress.ratio * 100)}%`,
                          }}
                        />
                      </div>
                    </div>
                  )}

                  <Button
                    variant="primary"
                    icon={<PackageIcon />}
                    onClick={() => void handleInstall()}
                    disabled={installDisabled}
                    className="w-full"
                  >
                    {installing ? "安装中…" : "安装"}
                  </Button>
                </Card>
              ) : (
                <Card>
                  <EmptyState icon={<PackageIcon />} title="从左侧清单选择一个版本以配置安装" />
                </Card>
              )}
            </div>
          </div>
        )}
      </motion.section>
    </>
  );
}
