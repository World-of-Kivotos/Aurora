// 实例卷宗页：一个已装实例的三面——概览（身份 / 版本级设置 / 待处理清单）、内容（Mod 清单）、变更史（事件流与回滚）。
// 地基纪律：磁盘是权威、卷宗只是索引。内容 tab 以 listMods 的扫盘结果为准，listLedger 只负责把身份贴上去；
// 卷宗里有、磁盘上没有的条目一概不显示（那是残留索引，不是已装内容）。
// 崩溃诊断刻意不单独占 tab：它是「上次运行留下的线索」而非常驻功能区，只在概览顶部出一条提示条。

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { openPath } from "@tauri-apps/plugin-opener";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { Select } from "../components/Select";
import { Skeleton } from "../components/Skeleton";
import { Toggle } from "../components/Toggle";
import { UpdatePanel } from "../components/UpdatePanel";
import { useToast } from "../components/Toast";
import {
  AlertIcon,
  CubeIcon,
  DownloadIcon,
  LayersIcon,
  PackageIcon,
  RefreshIcon,
  SearchIcon,
} from "../components/icons";
import { pageItem, springs } from "../lib/motion";
import {
  backupSize,
  checkUpdates,
  getVersionSettings,
  identifyInstalledMods,
  lastCrash,
  listHistory,
  listInstalled,
  listLedger,
  listMods,
  rollback,
  rollbackChecks,
  setModEnabled,
  setVersionSettings,
  type CrashReport,
  type History as ChangeHistory,
  type HistoryEvent,
  type InstalledMod,
  type InstalledVersionDto,
  type IsolationOverride,
  type Ledger,
  type LedgerEntry,
  type RollbackCheck,
  type UpdateCandidate,
  type VersionSettingsDto,
} from "../lib/ipc";

type TabKey = "overview" | "content" | "history";
type ModFilter = "all" | "enabled" | "disabled" | "updatable";

const TABS: { key: TabKey; label: string; icon: typeof CubeIcon }[] = [
  { key: "overview", label: "概览", icon: CubeIcon },
  { key: "content", label: "内容", icon: PackageIcon },
  { key: "history", label: "变更史", icon: LayersIcon },
];

/** 同行控件统一 40px 高，与下载页一致。 */
const CTRL = "h-10";

const inputCls =
  "w-full rounded-[3px] border border-ink/16 bg-paper px-3.5 py-2.5 text-[14px] text-ink transition-colors placeholder:text-ink/60 focus-visible:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

const ISOLATION_OPTIONS: { value: IsolationOverride; label: string }[] = [
  { value: "follow_global", label: "跟随全局" },
  { value: "enabled", label: "强制隔离" },
  { value: "disabled", label: "强制不隔离" },
];

const DISABLED_SUFFIX = ".disabled";

/** 磁盘文件名 → 卷宗键。禁用只是给文件加了后缀，身份没变，join 前必须先把后缀剥掉。 */
function ledgerKey(fileName: string): string {
  return fileName.endsWith(DISABLED_SUFFIX)
    ? fileName.slice(0, -DISABLED_SUFFIX.length)
    : fileName;
}

function fmtTime(unixSec: number): string {
  const d = new Date(unixSec * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function loaderText(v: InstalledVersionDto): string {
  if (v.loaders.length === 0) return "原版";
  return v.loaders.map((l) => (l.version ? `${l.kind} ${l.version}` : l.kind)).join(" · ");
}

// ---- 通用小件（本页自用；下载页那两个是页内私有函数，无法跨文件复用，样式按同一套 token 对齐）----

function SearchField({
  value,
  onChange,
  placeholder,
  className = "",
}: {
  value: string;
  onChange: (v: string) => void;
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
        placeholder={placeholder}
        className={`${CTRL} w-full rounded-[3px] border border-ink/14 bg-paper pr-3 pl-9 text-[14px] text-ink transition-colors outline-none placeholder:text-ink/60 hover:border-ink/30 focus:border-ink focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2`}
      />
    </div>
  );
}

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
      className={`${CTRL} flex shrink-0 items-center gap-1 rounded-[3px] bg-ink/[0.05] p-1`}
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
              "relative h-full cursor-pointer rounded-[2px] px-3 text-[13px] font-bold transition-colors",
              on ? "text-paper-on" : "text-ink/60 hover:text-ink/75",
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

/** 元信息小标签：发丝底、无色彩语义，用于 MC 版本 / 加载器 / 平台这类事实性标注。 */
function Tag({ children, tone = "plain" }: { children: ReactNode; tone?: "plain" | "accent" }) {
  const cls =
    tone === "accent"
      ? "bg-accent/12 text-accent"
      : "bg-ink/[0.07] text-ink/60";
  return (
    <span className={`shrink-0 rounded-[2px] px-1.5 py-0.5 text-[11px] font-semibold ${cls}`}>
      {children}
    </span>
  );
}

function SectionTitle({ title, note }: { title: string; note?: string }) {
  return (
    <h2 className="mb-3 flex items-baseline gap-3">
      <span className="text-[11px] font-bold tracking-[0.22em] text-ink/60">{title}</span>
      {note && <span className="text-[12px] text-ink/60">{note}</span>}
    </h2>
  );
}

function SettingRow({
  title,
  desc,
  control,
}: {
  title: string;
  desc: string;
  control: ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-ink/9 py-[18px] first:pt-0 last:border-b-0 last:pb-0">
      <div className="min-w-0">
        <div className="text-[15px] font-bold">{title}</div>
        <div className="mt-1 text-[12.5px] text-ink/60">{desc}</div>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

// ============ 崩溃提示条 ============
// 文案纪律：规则命中只是线索不是定论，一律写「日志指向 X」，绝不写「X 导致崩溃」。
function CrashBanner({ report }: { report: CrashReport }) {
  const { toast } = useToast();
  const [open, setOpen] = useState(false);
  const findings = report.diagnoses.length + report.suspects.length;
  const alarming = findings > 0;

  const openLog = async () => {
    if (!report.log_path) return;
    try {
      await openPath(report.log_path);
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    <div
      className={[
        "rounded-[3px] border px-4 py-3",
        alarming ? "border-danger/35 bg-danger/[0.04]" : "border-ink/12 bg-paper-sink",
      ].join(" ")}
    >
      <div className="flex items-center gap-3">
        <span className={alarming ? "text-danger" : "text-ink/60"}>
          <AlertIcon size={18} />
        </span>
        <span className={`flex-1 text-[13px] ${alarming ? "text-danger" : "text-ink/60"}`}>
          {alarming
            ? `上次运行的日志里读出 ${findings} 条线索`
            : "上次运行的日志已归档，未读出已知的异常线索"}
        </span>
        {findings > 0 && (
          <Button variant="secondary" onClick={() => setOpen((v) => !v)}>
            {open ? "收起" : "展开详情"}
          </Button>
        )}
        {report.log_path && (
          <Button variant="secondary" onClick={() => void openLog()}>
            打开日志
          </Button>
        )}
      </div>

      {report.log_path && (
        <p className="mt-2 truncate font-mono text-[11px] text-ink/60" title={report.log_path}>
          {report.log_path}
        </p>
      )}

      <AnimatePresence initial={false}>
        {open && findings > 0 && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={springs.tap}
            className="overflow-hidden"
          >
            <div className="mt-3 border-t border-ink/10 pt-3">
              {report.diagnoses.length > 0 && (
                <ul className="m-0 list-none p-0">
                  {report.diagnoses.map((d, i) => (
                    <li key={`${d.category}-${i}`} className="border-b border-ink/8 py-2.5 first:pt-0 last:border-b-0 last:pb-0">
                      <div className="flex items-baseline gap-2">
                        <span className="text-[14px] font-bold text-ink">{d.summary}</span>
                        <Tag>{d.category}</Tag>
                      </div>
                      <p className="mt-1 text-[12.5px] text-ink/60">{d.advice}</p>
                      {d.detail && (
                        <p className="mt-1 font-mono text-[11.5px] text-ink/60">{d.detail}</p>
                      )}
                      <p className="mt-1 truncate font-mono text-[11px] text-ink/60" title={d.matched}>
                        {d.matched}
                      </p>
                    </li>
                  ))}
                </ul>
              )}

              {report.suspects.length > 0 && (
                <div className="mt-3">
                  <SectionTitle title="日志指向的文件" />
                  <ul className="m-0 list-none p-0">
                    {report.suspects.map((s) => (
                      <li key={s.mod_id} className="py-1 text-[13px] text-ink/75">
                        日志指向 <span className="font-mono">{s.file_name ?? s.mod_id}</span>
                        {s.file_name && <span className="ml-2 text-[11.5px] text-ink/60">{s.mod_id}</span>}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

// ============ 概览 ============
interface OverviewProps {
  versionId: string;
  identity: InstalledVersionDto | null;
  settings: VersionSettingsDto;
  crash: CrashReport | null;
  updatableCount: number;
  updatesLoading: boolean;
  updatesError: string | null;
  onSaveSettings: (patch: {
    description?: string | null;
    favorite?: boolean;
    isolation?: IsolationOverride;
  }) => Promise<void>;
  onRetryUpdates: () => void;
  onGoUpdates: () => void;
}

function OverviewTab({
  versionId,
  identity,
  settings,
  crash,
  updatableCount,
  updatesLoading,
  updatesError,
  onSaveSettings,
  onRetryUpdates,
  onGoUpdates,
}: OverviewProps) {
  const [descDraft, setDescDraft] = useState(settings.description ?? "");
  const [saving, setSaving] = useState(false);

  // 保存成功后后端回传的是权威值，用它覆盖草稿；打字期间 settings 不变，不会被回写打断。
  useEffect(() => {
    setDescDraft(settings.description ?? "");
  }, [settings.description]);

  const commit = async (patch: Parameters<OverviewProps["onSaveSettings"]>[0]) => {
    setSaving(true);
    try {
      await onSaveSettings(patch);
    } finally {
      setSaving(false);
    }
  };

  const commitDesc = () => {
    const next = descDraft.trim() === "" ? null : descDraft;
    if (next === settings.description) return;
    void commit({ description: next });
  };

  // 待处理清单当前只有「可更新的 Mod」这一个来源。每新增一类都必须自带可执行动作，
  // 否则不许进这张清单——只报问题不给动作的条目等于把活推回给玩家。
  const pendingCount = updatableCount > 0 ? 1 : 0;

  return (
    <div className="min-w-0">
      {crash && (
        <div className="mb-5">
          <CrashBanner report={crash} />
        </div>
      )}

      {/* 身份条：这个实例「是谁、文件落在哪」，常驻概览顶部。 */}
      <div className="rounded-[3px] border border-ink/12 bg-paper-sink px-[18px] py-4">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-2">
          <span className="text-[21px] leading-tight font-extrabold tracking-[-0.01em] tabular-nums">
            {versionId}
          </span>
          {identity ? (
            <>
              <Tag>MC {identity.mc_version}</Tag>
              <Tag>{loaderText(identity)}</Tag>
            </>
          ) : (
            <Tag tone="accent">不在已安装列表中</Tag>
          )}
          <Tag tone={settings.isolated ? "accent" : "plain"}>
            {settings.isolated ? "隔离" : "共享"}
          </Tag>
        </div>

        <div className="mt-3 flex items-baseline gap-3">
          <span className="shrink-0 text-[11px] font-bold tracking-[0.18em] text-ink/60">工作目录</span>
          <span className="min-w-0 flex-1 font-mono text-[12px] break-all text-ink/75">
            {settings.working_dir}
          </span>
        </div>

        {settings.forced_by_local_data && (
          <p className="mt-2 text-[12px] text-accent">
            因目录内已有存档或 Mod 被强制隔离——此时把覆盖设为「不隔离」也不会生效。
          </p>
        )}
      </div>

      {/* 版本级设置：整体覆盖语义，读出完整对象改完写回。 */}
      <div className="mt-7">
        <SectionTitle title="版本级设置" />
        <Card>
          <SettingRow
            title="描述"
            desc="给这个实例一句备注，只在启动器里显示"
            control={
              <input
                value={descDraft}
                onChange={(e) => setDescDraft(e.target.value)}
                onBlur={commitDesc}
                onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                disabled={saving}
                placeholder="例如：生存服专用"
                aria-label="实例描述"
                className={`${inputCls} w-[280px]`}
              />
            }
          />
          <SettingRow
            title="收藏"
            desc="收藏的实例排在版本列表前面"
            control={
              <Toggle
                checked={settings.favorite}
                onChange={(next) => void commit({ favorite: next })}
                disabled={saving}
                ariaLabel="收藏该实例"
              />
            }
          />
          <SettingRow
            title="版本隔离"
            desc="覆盖全局隔离档位，决定存档与 Mod 落在版本目录还是共享的 .minecraft 根目录"
            control={
              <div className="w-[190px]">
                <Select<IsolationOverride>
                  value={settings.isolation}
                  onChange={(next) => void commit({ isolation: next })}
                  options={ISOLATION_OPTIONS}
                  ariaLabel="版本隔离覆盖"
                  disabled={saving}
                  className={CTRL}
                />
              </div>
            }
          />
        </Card>
      </div>

      {/* 健康度：只列「有动作可做」的事，不做 0-100 打分——分数不可执行，清单可以。 */}
      <div className="mt-7">
        <SectionTitle
          title="待处理"
          note={updatesLoading ? "正在检查 Mod 更新" : `${pendingCount} 项`}
        />
        <Card>
          {updatesError ? (
            <div className="flex items-center gap-3">
              <span className="text-danger">
                <AlertIcon size={18} />
              </span>
              <span className="flex-1 text-[13px] text-danger">检查更新失败：{updatesError}</span>
              <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={onRetryUpdates}>
                重试
              </Button>
            </div>
          ) : updatesLoading ? (
            <div className="flex items-center gap-3">
              <Skeleton className="h-[15px] w-40" />
              <Skeleton className="ml-auto h-9 w-20" />
            </div>
          ) : updatableCount === 0 ? (
            <p className="text-[13px] text-ink/60">暂无待处理项。</p>
          ) : (
            <ul className="m-0 list-none p-0">
              <li className="flex items-center justify-between gap-4">
                <span className="text-[14px] text-ink/75">
                  <span className="font-bold tabular-nums">{updatableCount}</span> 个 Mod 可更新
                </span>
                <Button variant="secondary" icon={<DownloadIcon size={15} />} onClick={onGoUpdates}>
                  查看
                </Button>
              </li>
            </ul>
          )}
        </Card>
      </div>
    </div>
  );
}

// ============ 内容 ============
/** 一行 = 一个磁盘上的文件（权威），外加卷宗补的身份与更新检查的结果。 */
interface ModRow {
  mod: InstalledMod;
  /** 卷宗键（已剥 .disabled）。 */
  key: string;
  entry: LedgerEntry | null;
  update: UpdateCandidate | null;
}

interface ContentProps {
  versionId: string;
  rows: ModRow[];
  filter: ModFilter;
  onFilterChange: (f: ModFilter) => void;
  onReload: () => Promise<void>;
}

function ContentTab({ versionId, rows, filter, onFilterChange, onReload }: ContentProps) {
  const { toast } = useToast();
  const [search, setSearch] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [identifying, setIdentifying] = useState(false);

  const counts = useMemo(() => {
    let enabled = 0;
    let updatable = 0;
    for (const r of rows) {
      if (r.mod.enabled) enabled += 1;
      if (r.update) updatable += 1;
    }
    return { all: rows.length, enabled, disabled: rows.length - enabled, updatable };
  }, [rows]);

  // 可更新档只在计数大于零时出现，避免点进去是空 tab。
  const filterOptions = useMemo(() => {
    const base: { value: ModFilter; label: string }[] = [
      { value: "all", label: `全部 ${counts.all}` },
      { value: "enabled", label: `已启用 ${counts.enabled}` },
      { value: "disabled", label: `已禁用 ${counts.disabled}` },
    ];
    if (counts.updatable > 0) base.push({ value: "updatable", label: `可更新 ${counts.updatable}` });
    return base;
  }, [counts]);

  // 计数归零后该档消失，选中态得跟着退回「全部」，否则列表会空着且无档可选。
  useEffect(() => {
    if (filter === "updatable" && counts.updatable === 0) onFilterChange("all");
  }, [filter, counts.updatable, onFilterChange]);

  const updateCandidates = useMemo(
    () => rows.map((r) => r.update).filter((u): u is UpdateCandidate => u !== null),
    [rows],
  );

  const shown = useMemo(() => {
    const q = search.trim().toLowerCase();
    return rows.filter((r) => {
      const byFilter =
        filter === "all"
          ? true
          : filter === "enabled"
            ? r.mod.enabled
            : filter === "disabled"
              ? !r.mod.enabled
              : r.update !== null;
      if (!byFilter) return false;
      if (!q) return true;
      const hay = [
        r.mod.file_name,
        r.mod.metadata?.name ?? "",
        r.mod.metadata?.mod_id ?? "",
        r.entry?.project_id ?? "",
      ]
        .join(" ")
        .toLowerCase();
      return hay.includes(q);
    });
  }, [rows, filter, search]);

  const toggle = async (row: ModRow, next: boolean) => {
    setBusy(row.mod.file_name);
    try {
      await setModEnabled(versionId, row.mod.file_name, next);
      await onReload();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(null);
    }
  };

  // identifyInstalledMods 是整个实例一起反查，行内按钮只是最自然的触发点。
  const identify = async () => {
    setIdentifying(true);
    try {
      const n = await identifyInstalledMods(versionId);
      await onReload();
      toast(n > 0 ? `已补上 ${n} 个文件的来源` : "没有能反查到来源的文件", n > 0 ? "success" : "info");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setIdentifying(false);
    }
  };

  return (
    <div className="min-w-0">
      <div className="mb-4 flex items-center gap-3">
        <SearchField
          value={search}
          onChange={setSearch}
          placeholder="搜索文件名、Mod 名或工程 id"
          className="min-w-0 flex-1"
        />
        <Segmented<ModFilter>
          value={filter}
          onChange={onFilterChange}
          options={filterOptions}
          ariaLabel="内容筛选"
        />
      </div>

      {rows.length === 0 ? (
        <EmptyState icon={<PackageIcon />} title="这个实例的 mods 目录还是空的" />
      ) : filter === "updatable" ? (
        // 「可更新」这一档交给 UpdatePanel：它带勾选、风险确认与批量执行，
        // 让这一档从只能看变成能动手，否则更新检查查出来也没有下一步。
        <UpdatePanel
          versionId={versionId}
          candidates={updateCandidates}
          onRefresh={() => void onReload()}
          onUpdated={() => void onReload()}
        />
      ) : shown.length === 0 ? (
        <EmptyState icon={<SearchIcon />} title="没有匹配的内容" />
      ) : (
        <ul className="m-0 list-none p-0">
          {shown.map((r) => {
            const meta = r.mod.metadata;
            const title = meta?.name ?? meta?.mod_id ?? r.key;
            return (
              <li
                key={r.mod.file_name}
                className="flex items-center gap-4 border-b border-ink/8 py-[13px] last:border-b-0"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
                    <span
                      className={`truncate text-[15px] font-bold ${r.mod.enabled ? "text-ink" : "text-ink/60"}`}
                    >
                      {title}
                    </span>
                    {meta?.version && <span className="text-[12px] text-ink/60">{meta.version}</span>}
                    {meta && <Tag>{meta.loader}</Tag>}
                    {r.update && <Tag tone="accent">可更新 {r.update.latest.version_number}</Tag>}
                  </div>

                  <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[11.5px] text-ink/60">
                    <span className="truncate font-mono">{r.mod.file_name}</span>
                    {r.entry ? (
                      <>
                        <Tag>{r.entry.platform}</Tag>
                        <span className="font-mono">{r.entry.project_id}</span>
                        <span className="font-mono">版本 {r.entry.version_id}</span>
                      </>
                    ) : (
                      <>
                        <span className="text-ink/60">来源未知</span>
                        <button
                          type="button"
                          onClick={() => void identify()}
                          disabled={identifying}
                          className="cursor-pointer font-semibold text-accent underline-offset-2 transition-opacity hover:underline disabled:pointer-events-none disabled:opacity-45 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
                        >
                          {identifying ? "识别中" : "识别"}
                        </button>
                      </>
                    )}
                  </div>
                </div>

                <Toggle
                  checked={r.mod.enabled}
                  onChange={(next) => void toggle(r, next)}
                  disabled={busy === r.mod.file_name}
                  ariaLabel={`${r.mod.enabled ? "禁用" : "启用"} ${title}`}
                />
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// ============ 变更史 ============
const EVENT_LABEL: Record<HistoryEvent["kind"], string> = {
  install: "安装",
  update: "更新",
  rollback: "回滚",
  remove: "移除",
};

function eventFiles(e: HistoryEvent): string[] {
  switch (e.kind) {
    case "install":
    case "remove":
      return e.files;
    case "update":
      return [e.file_name];
    case "rollback":
      return [];
  }
}

function eventDetail(e: HistoryEvent): string | null {
  switch (e.kind) {
    case "update":
      return `${e.from_version} → ${e.to_version}`;
    case "rollback":
      return `撤销事件 ${e.reverted_event}`;
    default:
      return null;
  }
}

/** 事件 id 形如 "<unix秒>-<三位序号>"：同秒多条只靠 at 排会乱序，用序号做次级键。 */
function eventOrder(e: HistoryEvent): number {
  const seq = Number(e.id.slice(e.id.indexOf("-") + 1));
  return e.at * 1000 + (Number.isFinite(seq) ? seq : 0);
}

interface HistoryProps {
  versionId: string;
  history: ChangeHistory;
  checks: RollbackCheck[];
  backupBytes: number;
  onReload: () => Promise<void>;
}

function HistoryTab({ versionId, history, checks, backupBytes, onReload }: HistoryProps) {
  const { toast } = useToast();
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmClean, setConfirmClean] = useState(false);

  const checkOf = useMemo(() => {
    const map = new Map<string, RollbackCheck>();
    for (const c of checks) map.set(c.event_id, c);
    return map;
  }, [checks]);

  const events = useMemo(
    () => [...history.events].sort((a, b) => eventOrder(b) - eventOrder(a)),
    [history.events],
  );

  const rollbackable = useMemo(() => checks.filter((c) => c.can_rollback), [checks]);

  const doRollback = async (eventId: string) => {
    setBusy(eventId);
    try {
      await rollback(versionId, eventId);
      await onReload();
      toast("已回滚到更新前的文件", "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex min-h-full min-w-0 flex-col">
      {events.length === 0 ? (
        <EmptyState icon={<LayersIcon />} title="这个实例还没有留下任何变更记录" />
      ) : (
        <ul className="m-0 list-none p-0">
          {events.map((e) => {
            const check = checkOf.get(e.id);
            const files = eventFiles(e);
            const detail = eventDetail(e);
            return (
              <li
                key={e.id}
                className="flex items-start gap-4 border-b border-ink/8 py-[13px] last:border-b-0"
              >
                <div className="min-w-0 flex-1">
                  <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
                    <span className="text-[15px] font-bold">{EVENT_LABEL[e.kind]}</span>
                    <span className="font-mono text-[12px] text-ink/60 tabular-nums">{fmtTime(e.at)}</span>
                    {detail && <span className="font-mono text-[12px] text-ink/60">{detail}</span>}
                  </div>
                  {files.length > 0 && (
                    <p className="mt-1 font-mono text-[11.5px] break-all text-ink/60">
                      {files.join("、")}
                    </p>
                  )}
                  {check && !check.can_rollback && check.reason && (
                    <p className="mt-1 text-[11.5px] text-ink/60">{check.reason}</p>
                  )}
                </div>

                {check && (
                  <Button
                    variant="secondary"
                    className="shrink-0"
                    disabled={!check.can_rollback || busy === e.id}
                    onClick={() => void doRollback(e.id)}
                  >
                    {busy === e.id ? "回滚中" : "回滚"}
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}

      {/* 备份是实打实的磁盘占用，常驻页尾显式告知，不让它在暗处自己长。 */}
      <div className="mt-auto flex items-center gap-4 border-t border-ink/12 pt-4">
        <span className="text-[13px] text-ink/60">
          备份占用 <span className="font-mono font-bold text-ink tabular-nums">{fmtBytes(backupBytes)}</span>
        </span>
        <span className="text-[12px] text-ink/60">
          {rollbackable.length > 0
            ? `${rollbackable.length} 个事件仍可回滚`
            : "当前没有可回滚的事件"}
        </span>
        <Button
          variant="secondary"
          className="ml-auto shrink-0"
          disabled={backupBytes === 0}
          onClick={() => setConfirmClean(true)}
        >
          清理备份
        </Button>
      </div>

      <Modal
        open={confirmClean}
        onClose={() => setConfirmClean(false)}
        title="清理备份"
        footer={
          <>
            <Button variant="secondary" onClick={() => setConfirmClean(false)}>
              取消
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                setConfirmClean(false);
                toast("清理备份尚未接线", "error");
              }}
            >
              确认清理
            </Button>
          </>
        }
      >
        <p>
          清理会删除更新时留下的 <span className="font-mono">.old</span> 备份，释放{" "}
          <span className="font-mono font-bold">{fmtBytes(backupBytes)}</span>。
        </p>
        {rollbackable.length > 0 ? (
          <>
            <p className="mt-3">清理后，以下事件将无法再回滚：</p>
            <ul className="mt-2 list-none p-0">
              {rollbackable.map((c) => (
                <li key={c.event_id} className="font-mono text-[12.5px] text-ink/60">
                  {c.event_id}
                </li>
              ))}
            </ul>
          </>
        ) : (
          <p className="mt-3">当前没有可回滚的事件，清理不会让你失去任何回滚能力。</p>
        )}
      </Modal>
    </div>
  );
}

// ============ 页面 ============
/** 一次读齐的实例卷宗。除更新检查（要走网络）外都是本地读取，一并 Promise.all 拿回来。 */
interface Dossier {
  identity: InstalledVersionDto | null;
  settings: VersionSettingsDto;
  mods: InstalledMod[];
  ledger: Ledger;
  history: ChangeHistory;
  checks: RollbackCheck[];
  backupBytes: number;
  crash: CrashReport | null;
}

export function InstanceDetail() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { toast } = useToast();
  const versionId = id ?? "";

  const [tab, setTab] = useState<TabKey>("overview");
  const [modFilter, setModFilter] = useState<ModFilter>("all");
  const [data, setData] = useState<Dossier | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 更新检查要打平台接口，单独一路：它慢或失败都不该把整页拖住。
  const [updates, setUpdates] = useState<UpdateCandidate[]>([]);
  const [updatesLoading, setUpdatesLoading] = useState(true);
  const [updatesError, setUpdatesError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!versionId) return;
    setError(null);
    try {
      const [scan, settings, mods, ledger, history, checks, backupBytes, crash] = await Promise.all([
        listInstalled(),
        getVersionSettings(versionId),
        listMods(versionId),
        listLedger(versionId),
        listHistory(versionId),
        rollbackChecks(versionId),
        backupSize(versionId),
        lastCrash(versionId),
      ]);
      setData({
        identity: scan.versions.find((v) => v.id === versionId) ?? null,
        settings,
        mods,
        ledger,
        history,
        checks,
        backupBytes,
        crash,
      });
    } catch (e) {
      setError(String(e));
    }
  }, [versionId]);

  const loadUpdates = useCallback(async () => {
    if (!versionId) return;
    setUpdatesLoading(true);
    setUpdatesError(null);
    try {
      setUpdates(await checkUpdates(versionId));
    } catch (e) {
      setUpdatesError(String(e));
    } finally {
      setUpdatesLoading(false);
    }
  }, [versionId]);

  useEffect(() => {
    void load();
    void loadUpdates();
  }, [load, loadUpdates]);

  const reloadMods = useCallback(async () => {
    const [mods, ledger] = await Promise.all([listMods(versionId), listLedger(versionId)]);
    setData((d) => (d ? { ...d, mods, ledger } : d));
  }, [versionId]);

  // 回滚会同时动磁盘文件与历史，两边都得重取。
  const reloadHistory = useCallback(async () => {
    const [history, checks, backupBytes, mods, ledger] = await Promise.all([
      listHistory(versionId),
      rollbackChecks(versionId),
      backupSize(versionId),
      listMods(versionId),
      listLedger(versionId),
    ]);
    setData((d) => (d ? { ...d, history, checks, backupBytes, mods, ledger } : d));
  }, [versionId]);

  const saveSettings = useCallback(
    async (patch: { description?: string | null; favorite?: boolean; isolation?: IsolationOverride }) => {
      if (!data) return;
      const s = data.settings;
      try {
        const next = await setVersionSettings(versionId, {
          description: s.description,
          icon: s.icon,
          favorite: s.favorite,
          category: s.category,
          isolation: s.isolation,
          ...patch,
        });
        setData((d) => (d ? { ...d, settings: next } : d));
        toast("已保存", "success");
        // 隔离覆盖换了工作目录，mods、卷宗、历史全都换了一处落脚点，更新检查的结果也随之作废，必须整页重取。
        if (patch.isolation !== undefined && next.working_dir !== s.working_dir) {
          await Promise.all([load(), loadUpdates()]);
        }
      } catch (e) {
        toast(String(e), "error");
      }
    },
    [data, versionId, toast, load, loadUpdates],
  );

  // 磁盘是权威：以 listMods 的扫盘结果为骨架，卷宗与更新检查只往上贴，绝不反过来凭卷宗造行。
  const rows = useMemo<ModRow[]>(() => {
    if (!data) return [];
    const entryOf = new Map<string, LedgerEntry>();
    for (const e of data.ledger.entries) entryOf.set(e.file_name, e);
    const updateOf = new Map<string, UpdateCandidate>();
    for (const u of updates) updateOf.set(ledgerKey(u.file_name), u);

    return data.mods.map((mod) => {
      const key = ledgerKey(mod.file_name);
      return {
        mod,
        key,
        entry: entryOf.get(key) ?? null,
        update: updateOf.get(key) ?? null,
      };
    });
  }, [data, updates]);

  const updatableCount = useMemo(() => rows.filter((r) => r.update).length, [rows]);

  if (!versionId) {
    return (
      <motion.div variants={pageItem}>
        <EmptyState
          icon={<AlertIcon />}
          title="路由里没有实例 id，无法打开卷宗"
          action={{ label: "返回版本列表", onClick: () => navigate("/versions") }}
        />
      </motion.div>
    );
  }

  return (
    <>
      <motion.div variants={pageItem} className="mb-5 flex items-baseline justify-between gap-6">
        <div className="flex min-w-0 items-baseline gap-4">
          <button
            type="button"
            onClick={() => navigate("/versions")}
            className="shrink-0 cursor-pointer text-[12px] font-semibold text-ink/60 transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
          >
            版本
          </button>
          <h1 className="truncate text-[20px] font-extrabold tracking-[-0.01em] tabular-nums">
            {versionId}
          </h1>
          <span className="shrink-0 text-[12px] text-ink/60">实例卷宗</span>
        </div>
        <button
          type="button"
          onClick={() => {
            void load();
            void loadUpdates();
          }}
          className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 text-[12px] font-semibold text-ink/60 transition-colors hover:text-ink [&_svg]:h-3.5 [&_svg]:w-3.5"
        >
          <RefreshIcon />
          刷新
        </button>
      </motion.div>

      {error && (
        <motion.div
          variants={pageItem}
          className="mb-5 flex items-center gap-3 rounded-[3px] border border-danger/40 px-4 py-3 text-[13px] text-danger"
        >
          <AlertIcon size={18} />
          <span className="flex-1">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={() => void load()}>
            重试
          </Button>
        </motion.div>
      )}

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
                on ? "font-extrabold text-ink" : "font-semibold text-ink/60 hover:text-ink/75",
              ].join(" ")}
            >
              <Icon size={16} />
              {t.label}
              {on && (
                <motion.span
                  layoutId="instance-tab-underline"
                  className="absolute inset-x-0 -bottom-px h-[2px] bg-accent"
                  transition={springs.tap}
                />
              )}
            </button>
          );
        })}
      </motion.div>

      <motion.div variants={pageItem} className="flex min-h-0 flex-1 flex-col">
        {!data ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-[92px] w-full" />
            <Skeleton className="h-[160px] w-full" delay={0.08} />
            <Skeleton className="h-[72px] w-full" delay={0.16} />
          </div>
        ) : (
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={tab}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -4 }}
              transition={springs.tap}
              className="flex min-h-full flex-col"
            >
              {tab === "overview" && (
                <OverviewTab
                  versionId={versionId}
                  identity={data.identity}
                  settings={data.settings}
                  crash={data.crash}
                  updatableCount={updatableCount}
                  updatesLoading={updatesLoading}
                  updatesError={updatesError}
                  onSaveSettings={saveSettings}
                  onRetryUpdates={() => void loadUpdates()}
                  onGoUpdates={() => {
                    setModFilter("updatable");
                    setTab("content");
                  }}
                />
              )}
              {tab === "content" && (
                <ContentTab
                  versionId={versionId}
                  rows={rows}
                  filter={modFilter}
                  onFilterChange={setModFilter}
                  onReload={reloadMods}
                />
              )}
              {tab === "history" && (
                <HistoryTab
                  versionId={versionId}
                  history={data.history}
                  checks={data.checks}
                  backupBytes={data.backupBytes}
                  onReload={reloadHistory}
                />
              )}
            </motion.div>
          </AnimatePresence>
        )}
      </motion.div>
    </>
  );
}
