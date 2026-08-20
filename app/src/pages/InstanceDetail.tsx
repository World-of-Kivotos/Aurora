// 实例卷宗页：那一个已装实例的三面——概览（身份 / 实例设置 / 待处理清单）、内容（Mod 清单）、变更史（事件流与回滚）。
// 单实例契约：Aurora 只管 World of Kivotos 一个实例，路由 /instance 不带参数，实例 id 取自 config.selected_version。
// 实例的唯一产生途径是安装受管整合包，所以「没有实例」等于「游戏还没装」，空态一律把人送回启动屏而不是某个列表页。
// 地基纪律：磁盘是权威、卷宗只是索引。内容 tab 以 listMods 的扫盘结果为准，listLedger 只负责把身份贴上去；
// 卷宗里有、磁盘上没有的条目一概不显示（那是残留索引，不是已装内容）。
// 崩溃诊断刻意不单独占 tab：它是「上次运行留下的线索」而非常驻功能区，只在概览顶部出一条提示条。

import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { openPath } from "@tauri-apps/plugin-opener";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { EmptyState } from "../components/EmptyState";
import { LiquidGlass } from "../components/LiquidGlass";
import { Modal } from "../components/Modal";
import { ManagedModpackPanel, ModpackFileOwnership } from "../components/ManagedModpackPanel";
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
import { pageItem, springs, tabPanel } from "../lib/motion";
import {
  backupSize,
  checkUpdates,
  getConfig,
  getVersionSettings,
  identifyInstalledMods,
  lastCrash,
  listHistory,
  listInstalled,
  listLedger,
  listMods,
  managedModpackFiles,
  managedModpackStatus,
  rollback,
  rollbackChecks,
  setModEnabled,
  setVersionSettings,
  syncManagedModpack,
  type CrashReport,
  type History as ChangeHistory,
  type HistoryEvent,
  type InstalledMod,
  type InstalledVersionDto,
  type IsolationOverride,
  type Ledger,
  type LedgerEntry,
  type ManagedModpackFile,
  type RollbackCheck,
  type UpdateCandidate,
  type VersionSettingsDto,
} from "../lib/ipc";
import {
  parseModpackSyncError,
  type ManagedModpackStatus,
  type ModpackFileOwner,
  type ModpackSyncState,
} from "../lib/modpack-ui";
import { modpackOwnerOf } from "../lib/modpack-ownership";
import { managedModpackInstallRoute } from "../lib/modpack-navigation";
import { useAppearance } from "../lib/appearance-context";

type TabKey = "overview" | "content" | "history";
type ModFilter = "all" | "enabled" | "disabled" | "updatable";

const TABS: { key: TabKey; label: string; icon: typeof CubeIcon }[] = [
  { key: "overview", label: "概览", icon: CubeIcon },
  { key: "content", label: "内容", icon: PackageIcon },
  { key: "history", label: "变更史", icon: LayersIcon },
];

/** 同行控件统一 40px 高，与下载页一致。 */
const CTRL = "h-10";

/*
 * 液态玻璃小件的透镜参数。白名单里的每个小件都该用这一组数，逐字一致：
 * 玻璃的厚度是材料属性而不是尺寸属性，同一种材料在几个小件上给出几种厚度，
 * 读起来就不再是同一种材料——这正是并行改界面时最容易留下的那种不一致。
 *
 * 三条取值依据，都不是拍的：
 *   1. bevel 8 —— bevelStops 会把「边厚 / 边长」夹到 0.5，一旦边厚超过半个边长，
 *      中性区宽度归零，这块玻璃从「有平面的透镜」退化成「整块都是斜面的棱镜」。
 *      小件高度只有 32~44px，库里那个给大面板用的默认值 22 直接触顶，必须调小。
 *   2. strength 10 —— 库里 26/22 的强度边厚比是 1.18，这里按同一比例缩到小件尺度，
 *      边缘最外沿的采样偏移约 5px，落在 8px 的斜面带内。
 *   3. blur/saturation 分两档 —— 组件写的是内联 backdrop-filter，优先级高于 .surface-liquid，
 *      会把类里那条整个盖掉（连 saturate 一起）。所以毛玻璃档必须逐字复刻类里的
 *      blur(14px) saturate(170%)；液态档的 saturate 补到 200%，与 :root[data-glass=liquid]
 *      那条对齐——折射滤镜内部有 feColorMatrix type="saturate" 承接这个数，不会丢。
 *      折射档的 blur 取 4 而不是 10：大模糊会把折射本身糊掉，清晰度是这个效果的一部分。
 */
const LIQUID_LENS = {
  liquid: { mode: "auto", strength: 10, bevel: 8, blur: 4, saturation: 200, sheen: false },
  frost: { mode: "frost", strength: 10, bevel: 8, blur: 14, saturation: 170, sheen: false },
} as const;

// 输入框走下沉档：它是寄生层，只能套在自足材质里（本页所有输入框都在 Card 或工具条面板内）。
// 描边焊在材质里，所以这里不再写 border，也不靠 border 表达聚焦——聚焦只由 outline 承担，玻璃上仍可见。
const inputCls =
  "w-full surface-sunken rounded-control px-3.5 py-2.5 text-[14px] text-ink outline-none placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

// 危险提示条：底仍是面板档（图上要能读），危险度只由左缘竖条与文字色承担。
// 材质类把描边与投影焊在 box-shadow 里且属于无层 CSS，工具类的 ring/border 压不过它，故用实体竖条。
const dangerBar = "surface-panel relative overflow-hidden rounded-panel px-4 py-3";

const ISOLATION_OPTIONS: { value: IsolationOverride; label: string }[] = [
  { value: "follow_global", label: "跟随全局" },
  { value: "enabled", label: "强制隔离" },
  { value: "disabled", label: "强制不隔离" },
];

const DISABLED_SUFFIX = ".disabled";

const INITIAL_MODPACK_PROGRESS = {
  stage: "resolving_manifest",
  completed_files: 0,
  total_files: 0,
  downloaded_bytes: 0,
  total_bytes: null,
  current_file: null,
  download_speed: null,
} as const;

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
      <span className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink/60">
        <SearchIcon size={16} />
      </span>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className={`${CTRL} surface-sunken w-full rounded-control pr-3 pl-9 text-[14px] text-ink outline-none placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2`}
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
  // 透镜档跟着全局玻璃模式走。frost 模式下不许出现折射：那一档的契约就是「纯毛玻璃」，
  // 由调用方明确请求 frost，而不是指望组件的能力探测替我们守住产品档位。
  const { appearance } = useAppearance();

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
            {on && (
              // 分段控件的选中页是液态玻璃的四个白名单之一：小面积，滤镜成本可控。
              // 套在下沉轨里，按「纸对纸无影」摘掉投影；剩下的受光边与高光足以读出「浮起的那一页」。
              //
              // 分两层是被 backdrop-filter 的语义逼出来的，不是包一层图省事：
              // 外层只拿 layoutId 做位移动画，不带任何材质；纸与透镜一起落在内层。
              // 若把纸留在外层，透镜采到的背景里已经含了这张纸，折射的就不再是照片。
              <motion.div
                layoutId={`seg-${ariaLabel}`}
                aria-hidden="true"
                className="absolute inset-0"
                transition={springs.tap}
              >
                <LiquidGlass
                  {...LIQUID_LENS[appearance.glass]}
                  className="surface-liquid surface-nested h-full w-full rounded-chip"
                />
              </motion.div>
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
  // 素色档走下沉材质而不是自拌一层半透明底：小记号也在照片上，底色必须由材质层统一给。
  // 强调档走实心 accent + 纸色字（4.77）：淡底配朱红字是全站最不该读不清的地方里最不清的一档
  // （面板上 2.93，密集档 3.63），而这枚标签承载的是「不在已安装列表中」「可更新」这类结论。
  const cls = tone === "accent" ? "bg-accent text-paper-on" : "surface-sunken text-ink/75";
  return (
    <span className={`shrink-0 rounded-chip px-1.5 py-0.5 text-[11px] font-semibold ${cls}`}>
      {children}
    </span>
  );
}

function SectionTitle({ title, note }: { title: string; note?: string }) {
  return (
    <h2 className="mb-3 flex items-baseline gap-3">
      <span className="text-[11px] font-bold tracking-[0.22em] text-ink/75">{title}</span>
      {note && <span className="text-[12px] text-ink/75">{note}</span>}
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
        <div className="mt-1 text-[12.5px] text-ink/75">{desc}</div>
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
    <div className={dangerBar}>
      {/* 有线索才亮红：竖条是这条提示条唯一的危险语义载体，无线索时整条退回中性面板。 */}
      {alarming && <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-danger" />}
      <div className="flex items-center gap-3">
        <span className={alarming ? "text-danger" : "text-ink/60"}>
          <AlertIcon size={18} />
        </span>
        <span className={`flex-1 text-[13px] ${alarming ? "text-danger" : "text-ink/75"}`}>
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
        <p className="mt-2 truncate font-mono text-[11px] text-ink/75" title={report.log_path}>
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
                      <p className="mt-1 text-[12.5px] text-ink/75">{d.advice}</p>
                      {d.detail && (
                        <p className="mt-1 font-mono text-[11.5px] text-ink/75">{d.detail}</p>
                      )}
                      <p className="mt-1 truncate font-mono text-[11px] text-ink/75" title={d.matched}>
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
                        {s.file_name && <span className="ml-2 text-[11.5px] text-ink/75">{s.mod_id}</span>}
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
  /** 页面已在渲染前用扫盘结果确认过实例存在，这里必然拿得到身份，不再有「不在已安装列表中」这一态。 */
  identity: InstalledVersionDto;
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
    // 这一面是一列按自然高度顺排的正文，不参与高度分配：出口由外层那块滚动区给（见 tab 分支处的注释）。
    // 别在这里加 flex-1 或 overflow-y-auto —— 前者会让内容被压扁而不是滚起来，后者是第二根滚动轴。
    <div className="min-w-0">
      {crash && (
        <div className="mb-5">
          <CrashBanner report={crash} />
        </div>
      )}

      {/* 身份条：这个实例「是谁、文件落在哪」，常驻概览顶部。 */}
      <div className="surface-panel rounded-panel px-[18px] py-4">
        <div className="flex flex-wrap items-baseline gap-x-3 gap-y-2">
          <span className="text-[21px] leading-tight font-extrabold tracking-[-0.01em] tabular-nums">
            {versionId}
          </span>
          <Tag>MC {identity.mc_version}</Tag>
          <Tag>{loaderText(identity)}</Tag>
          <Tag tone={settings.isolated ? "accent" : "plain"}>
            {settings.isolated ? "隔离" : "共享"}
          </Tag>
        </div>

        <div className="mt-3 flex items-baseline gap-3">
          <span className="shrink-0 text-[11px] font-bold tracking-[0.18em] text-ink/75">工作目录</span>
          <span className="min-w-0 flex-1 font-mono text-[12px] break-all text-ink/75">
            {settings.working_dir}
          </span>
        </div>

        {/* 这句原来是朱红字，在面板玻璃上实算 3.40，正文门槛过不了。整句都是要读的内容而不是记号，
            所以强调改由字重承担：周围是 ink/75，这里满墨加粗，层级差得出来且对比度回到 11.76。 */}
        {settings.forced_by_local_data && (
          <p className="mt-2 text-[12px] font-semibold text-ink">
            因目录内已有存档或 Mod 被强制隔离——此时把覆盖设为「不隔离」也不会生效。
          </p>
        )}
      </div>

      {/* 实例设置：整体覆盖语义，读出完整对象改完写回。 */}
      <div className="mt-7">
        <SectionTitle title="实例设置" />
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
          {/* 「收藏」这一档随版本列表一起下线：它的唯一效果是把实例排到列表前面，
              单实例下既无列表也无排序，留一个改了什么都不会发生的开关比不给更糟。
              后端字段不动，saveSettings 每次都把原值原样写回，将来若要恢复不丢数据。 */}
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
            <p className="text-[13px] text-ink/75">暂无待处理项。</p>
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
  /** null 表示归属尚未读取完成，此时管理开关必须保持禁用。 */
  owner: ModpackFileOwner | null;
}

interface ContentProps {
  versionId: string;
  rows: ModRow[];
  ownershipError: string | null;
  filter: ModFilter;
  onFilterChange: (f: ModFilter) => void;
  onReload: () => Promise<void>;
}

function ContentTab({ versionId, rows, ownershipError, filter, onFilterChange, onReload }: ContentProps) {
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
    // 这一面自己把高度分完：工具条与提示条定高不动，剩下的全给清单，清单在自己身上滚。
    // 因此外层（页签内容区）刻意不给它再套一层滚动区——那会变成同一根轴上的双层滚动。
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {ownershipError && (
        <div
          className={`${dangerBar} mb-4 flex shrink-0 items-center gap-3 text-[13px] text-danger`}
          role="alert"
        >
          <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-danger" />
          <AlertIcon size={18} />
          <span>无法确认整合包文件归属：{ownershipError}。为避免误改受管文件，管理开关已停用。</span>
        </div>
      )}
      {/* 工具条自成一块面板：搜索框与分段轨都是寄生层，没有这层自足材质它们就直接压在照片上。
          它同时是全站最宽的一横排，minWidth 960 就卡在这里（换算见 app.css 第六之二节）。 */}
      <div className="surface-panel mb-4 flex shrink-0 items-center gap-3 rounded-panel px-3 py-3">
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
        <div className="surface-panel rounded-panel px-[18px] py-2">
          <EmptyState icon={<PackageIcon />} title="mods 目录还是空的" />
        </div>
      ) : filter === "updatable" ? (
        // 「可更新」这一档交给 UpdatePanel：它带勾选、风险确认与批量执行，
        // 让这一档从只能看变成能动手，否则更新检查查出来也没有下一步。
        // 它同样是一条与 Mod 数量等长的清单，故与下面那张清单一样在内部滚；
        // 滚动区放在外面是因为面板归 UpdatePanel 自己管，本页只负责给它一只定高的盒子。
        <div className="min-h-0 flex-1 overflow-y-auto pr-1">
          <UpdatePanel
            versionId={versionId}
            candidates={updateCandidates}
            onRefresh={() => void onReload()}
            onUpdated={() => void onReload()}
          />
        </div>
      ) : shown.length === 0 ? (
        <div className="surface-panel rounded-panel px-[18px] py-2">
          <EmptyState icon={<SearchIcon />} title="没有匹配的内容" />
        </div>
      ) : (
        // Mod 清单是要长时间连续扫读的长列表，托最实的一档：ink/75 在它上面拿到 7.20，越过 AAA。
        // 滚动做在这张面板自己身上而不是外面包一层：这样面板的上下缘钉在内容盒里不动，
        // 滚的只是行；包在外面则整块面板连同圆角一起滑走，读起来是「页面在滚」而不是「清单在滚」。
        <ul className="surface-panel-strong m-0 min-h-0 flex-1 list-none overflow-y-auto rounded-panel px-[18px] py-1">
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
                    {/* 已禁用只靠一档灰度表达。15px 粗体属「大字」，ink/60 门槛是 3.0，
                        在最实那档托底上实得 4.43，是全表里少数还能合法留在 ink/60 的位置。 */}
                    <span
                      className={`truncate text-[15px] font-bold ${r.mod.enabled ? "text-ink" : "text-ink/60"}`}
                    >
                      {title}
                    </span>
                    {meta?.version && <span className="text-[12px] text-ink/75">{meta.version}</span>}
                    {meta && <Tag>{meta.loader}</Tag>}
                    {r.owner && <ModpackFileOwnership owner={r.owner} />}
                    {r.update && <Tag tone="accent">可更新 {r.update.latest.version_number}</Tag>}
                  </div>

                  <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[11.5px] text-ink/75">
                    <span className="truncate font-mono">{r.mod.file_name}</span>
                    {r.entry ? (
                      <>
                        <Tag>{r.entry.platform}</Tag>
                        <span className="font-mono">{r.entry.project_id}</span>
                        <span className="font-mono">版本 {r.entry.version_id}</span>
                      </>
                    ) : (
                      <>
                        <span>来源未知</span>
                        <button
                          type="button"
                          onClick={() => void identify()}
                          disabled={identifying}
                          // 文字按钮的可点感原来靠朱红字，而 accent 在这档托底上只有 4.29，不到正文的 4.5。
                          // 改成满墨常驻下划线：affordance 由下划线给，颜色回到全档通吃的满墨。
                          className="cursor-pointer font-semibold text-ink underline underline-offset-2 transition-opacity hover:decoration-2 disabled:pointer-events-none disabled:opacity-45 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
                        >
                          {identifying ? "识别中" : "识别"}
                        </button>
                      </>
                    )}
                  </div>
                </div>

                <span
                  title={
                    r.owner === "modpack"
                      ? "受管 Mod 由整合包统一维护，不能单独启用或禁用"
                      : r.owner === null
                        ? "整合包文件归属尚未安全确认"
                        : undefined
                  }
                >
                  <Toggle
                    checked={r.mod.enabled}
                    onChange={(next) => void toggle(r, next)}
                    disabled={r.owner !== "player" || busy === r.mod.file_name}
                    ariaLabel={`${r.mod.enabled ? "禁用" : "启用"} ${title}`}
                  />
                </span>
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
    // 高度由外层分配（原来是 min-h-full，那是给会滚的外壳写的），备份条继续靠 mt-auto 钉在页尾。
    // 与「内容」同理：事件流在面板内部滚，这一面因此不需要、也不许再被外层套一层滚动区。
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {events.length === 0 ? (
        <div className="surface-panel rounded-panel px-[18px] py-2">
          <EmptyState icon={<LayersIcon />} title="还没有留下任何变更记录" />
        </div>
      ) : (
        // 事件流同样是连续扫读的长列表，与 Mod 清单取同一档托底，两个 tab 之间不出现材质跳档；
        // 滚动区也做在面板自己身上：这条流随更新与装 Mod 只增不减，几次更新就能超过一屏，
        // 外壳是 overflow-clip，不给它自己的出口就会把下面那条备份/清理操作条无声裁掉。
        <ul className="surface-panel-strong m-0 min-h-0 flex-1 list-none overflow-y-auto rounded-panel px-[18px] py-1">
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
                    <span className="font-mono text-[12px] text-ink/75 tabular-nums">{fmtTime(e.at)}</span>
                    {detail && <span className="font-mono text-[12px] text-ink/75">{detail}</span>}
                  </div>
                  {files.length > 0 && (
                    <p className="mt-1 font-mono text-[11.5px] break-all text-ink/75">
                      {files.join("、")}
                    </p>
                  )}
                  {check && !check.can_rollback && check.reason && (
                    <p className="mt-1 text-[11.5px] text-ink/75">{check.reason}</p>
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

      {/* 备份是实打实的磁盘占用，常驻页尾显式告知，不让它在暗处自己长。
          它与上面的列表之间隔着一段留白，压的是照片而不是列表，所以要自带一层材质而非只留一条分隔线。 */}
      <div className="surface-panel mt-auto flex items-center gap-4 rounded-panel px-[18px] py-3.5">
        <span className="text-[13px] text-ink/75">
          备份占用 <span className="font-mono font-bold text-ink tabular-nums">{fmtBytes(backupBytes)}</span>
        </span>
        <span className="text-[12px] text-ink/75">
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
                <li key={c.event_id} className="font-mono text-[12.5px] text-ink/75">
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
  identity: InstalledVersionDto;
  settings: VersionSettingsDto;
  mods: InstalledMod[];
  ledger: Ledger;
  history: ChangeHistory;
  checks: RollbackCheck[];
  backupBytes: number;
  crash: CrashReport | null;
}

/**
 * 实例的解析结果。四态分开而不是用一个可空 id 混着表达：
 * resolving 与 absent 长得像却是两件事——前者是还没读到 config，此时画「游戏没装」就是在骗人。
 */
type Resolution =
  | { kind: "resolving" }
  | { kind: "failed"; message: string }
  | { kind: "absent" }
  | { kind: "ready"; versionId: string };

export function InstanceDetail() {
  const navigate = useNavigate();
  const { toast } = useToast();

  const [resolution, setResolution] = useState<Resolution>({ kind: "resolving" });
  // 未就绪时给空串：下面每个按实例取数的回调都以此短路，省掉一层「id 可能为空」的分支。
  const versionId = resolution.kind === "ready" ? resolution.versionId : "";

  const [tab, setTab] = useState<TabKey>("overview");
  const [modFilter, setModFilter] = useState<ModFilter>("all");
  const [data, setData] = useState<Dossier | null>(null);
  const [error, setError] = useState<string | null>(null);

  // 更新检查要打平台接口，单独一路：它慢或失败都不该把整页拖住。
  const [updates, setUpdates] = useState<UpdateCandidate[]>([]);
  const [updatesLoading, setUpdatesLoading] = useState(true);
  const [updatesError, setUpdatesError] = useState<string | null>(null);
  const [managedStatus, setManagedStatus] = useState<ManagedModpackStatus | null>(null);
  const [managedStatusLoading, setManagedStatusLoading] = useState(true);
  const [managedStatusError, setManagedStatusError] = useState<string | null>(null);
  const [managedFiles, setManagedFiles] = useState<ManagedModpackFile[] | null | undefined>(undefined);
  const [managedFilesError, setManagedFilesError] = useState<string | null>(null);
  const [modpackSync, setModpackSync] = useState<ModpackSyncState>({ kind: "idle" });

  // 实例 id 来自 config，与磁盘状态是两回事：整合包装到一半失败、玩家手删版本目录，config 都还留着旧 id。
  // 所以解析只负责取 id，是否真的存在一律由 load() 的扫盘结果说了算。
  const resolve = useCallback(async () => {
    setResolution({ kind: "resolving" });
    try {
      const cfg = await getConfig();
      // 空串与 null 同义，都表示还没选中任何实例；漏掉空串会让下面全部取数被空 id 短路，卡在半张页面上。
      const selected = cfg.selected_version;
      setResolution(
        selected === null || selected === ""
          ? { kind: "absent" }
          : { kind: "ready", versionId: selected },
      );
    } catch (e) {
      setResolution({ kind: "failed", message: String(e) });
    }
  }, []);

  const load = useCallback(async () => {
    if (!versionId) return;
    setError(null);
    try {
      // 先单独扫盘复核：id 指向的实例可能已经不在磁盘上，那时后面按版本读的七个调用全无意义，
      // 且其中任何一个抛错都会被画成「读取失败」，把「游戏没装」这件事说成了故障。
      const scan = await listInstalled();
      const identity = scan.versions.find((v) => v.id === versionId) ?? null;
      if (identity === null) {
        setData(null);
        setResolution({ kind: "absent" });
        return;
      }

      const [settings, mods, ledger, history, checks, backupBytes, crash] = await Promise.all([
        getVersionSettings(versionId),
        listMods(versionId),
        listLedger(versionId),
        listHistory(versionId),
        rollbackChecks(versionId),
        backupSize(versionId),
        lastCrash(versionId),
      ]);
      setData({
        identity,
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

  const loadManagedStatus = useCallback(async (preservePrevious: boolean) => {
    if (!versionId) return;
    setManagedStatusLoading(true);
    setManagedStatusError(null);
    setManagedStatus((previous) => {
      if (!preservePrevious || previous === null) return null;
      return {
        kind: "checking",
        subscription: previous.subscription,
        last_known: previous.kind === "ready" ? previous.versions : previous.last_known,
      };
    });
    try {
      setManagedStatus(await managedModpackStatus(versionId));
    } catch (e) {
      setManagedStatusError(String(e));
    } finally {
      setManagedStatusLoading(false);
    }
  }, [versionId]);

  const loadManagedFiles = useCallback(async () => {
    if (!versionId) return;
    setManagedFiles(undefined);
    setManagedFilesError(null);
    try {
      setManagedFiles(await managedModpackFiles(versionId));
    } catch (e) {
      setManagedFilesError(String(e));
    }
  }, [versionId]);

  // 先解析实例 id。下面那个 effect 的四个回调都以空 id 短路，id 落定后依赖变化会自动跑第二轮真正取数。
  useEffect(() => {
    void resolve();
  }, [resolve]);

  useEffect(() => {
    setModpackSync({ kind: "idle" });
    void load();
    void loadUpdates();
    void loadManagedStatus(false);
    void loadManagedFiles();
  }, [load, loadUpdates, loadManagedStatus, loadManagedFiles]);

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

  const runModpackSync = useCallback(async (targetVersion: string) => {
    setModpackSync({
      kind: "running",
      target_version: targetVersion,
      progress: INITIAL_MODPACK_PROGRESS,
    });

    try {
      const outcome = await syncManagedModpack(versionId, targetVersion, (progress) => {
        setModpackSync({
          kind: "running",
          target_version: targetVersion,
          progress,
        });
      });
      setModpackSync({ kind: "complete", installed_version: outcome.installed_version });
      await Promise.all([
        load(),
        loadUpdates(),
        loadManagedStatus(true),
        loadManagedFiles(),
      ]);
    } catch (e) {
      const structured = parseModpackSyncError(e);
      if (structured) {
        setModpackSync({
          kind: "failed",
          target_version: structured.target_version,
          stage: structured.stage,
          failure: structured.failure,
        });
      } else {
        setModpackSync({ kind: "idle" });
        toast(`整合包同步失败：${String(e)}`, "error");
      }
    }
  }, [versionId, load, loadUpdates, loadManagedStatus, loadManagedFiles, toast]);

  const ownershipStatus =
    managedStatusLoading || managedStatusError !== null ? undefined : managedStatus;

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
        owner: modpackOwnerOf(ownershipStatus, managedFiles, mod.file_name),
      };
    });
  }, [data, updates, ownershipStatus, managedFiles]);

  const updatableCount = useMemo(() => rows.filter((r) => r.update).length, [rows]);

  if (resolution.kind === "resolving") {
    // 读 config 是本地调用，通常一闪而过；仍要占位，否则会先闪一下空态再跳出卷宗。
    return (
      <motion.div variants={pageItem} className="flex flex-col gap-3">
        <Skeleton className="h-[92px] w-full" />
        <Skeleton className="h-[160px] w-full" delay={0.08} />
      </motion.div>
    );
  }

  if (resolution.kind === "failed") {
    return (
      <motion.div
        variants={pageItem}
        className={`${dangerBar} flex items-center gap-3 text-[13px] text-danger`}
        role="alert"
      >
        <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-danger" />
        <AlertIcon size={18} />
        <span className="flex-1">读取启动器配置失败，无法确定要打开哪个实例：{resolution.message}</span>
        <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={() => void resolve()}>
          重试
        </Button>
      </motion.div>
    );
  }

  if (resolution.kind === "absent") {
    // 单实例下没有实例就等于没装游戏，而装游戏的唯一入口在启动屏（受管整合包），所以这里只给这一个出口。
    // EmptyState 是内容片段不是容器（见组件头注），自己不带材质，main 也刻意不挂，故必须包一层。
    return (
      <motion.div variants={pageItem} className="surface-panel rounded-panel px-[18px] py-2">
        <EmptyState
          icon={<PackageIcon />}
          title="World of Kivotos 还没有安装，暂时没有卷宗可看"
          action={{ label: "去启动屏安装", onClick: () => navigate("/") }}
        />
      </motion.div>
    );
  }

  return (
    <>
      {/* 卷宗抬头：面包屑、标题、刷新与三面 tab 合成同一块面板。
          它们本来就是同一组导航件，图铺满之后各自浮一片纸只会在照片上多出一道无意义的缝。 */}
      {/* 抬头连同三面 tab 是本页的固定件（shrink-0）：外壳已不滚（app.css 第六节），
          高度由这一页自己分配。三面各有各的出口，且每面只有一根滚动轴：
          概览滚整篇正文（滚动区在下面那个 tab 分支里），内容滚 Mod 清单 / UpdatePanel，
          变更史滚事件流；页签切换时抬头一格不动。 */}
      <motion.div
        variants={pageItem}
        className="surface-panel mb-5 shrink-0 rounded-panel px-5 pt-4 pb-3"
      >
        <div className="flex items-baseline justify-between gap-6">
          <div className="flex min-w-0 items-baseline gap-4">
            {/* 版本列表页已下线，这条返回只能指回启动屏——它是这个专用启动器唯一的上一级。 */}
            <button
              type="button"
              onClick={() => navigate("/")}
              className="shrink-0 cursor-pointer text-[12px] font-semibold text-ink/75 transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2"
            >
              启动屏
            </button>
            <h1 className="truncate text-[20px] font-extrabold tracking-[-0.01em] tabular-nums">
              {versionId}
            </h1>
            <span className="shrink-0 text-[12px] text-ink/75">实例卷宗</span>
          </div>
          <button
            type="button"
            onClick={() => {
              void load();
              void loadUpdates();
              void loadManagedStatus(true);
              void loadManagedFiles();
            }}
            className="inline-flex shrink-0 cursor-pointer items-center gap-1.5 text-[12px] font-semibold text-ink/75 transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2 [&_svg]:h-3.5 [&_svg]:w-3.5"
          >
            <RefreshIcon />
            刷新
          </button>
        </div>

        {/* 分段 tab：选中下划线用共享 layoutId，切换时在标签之间滑过去而不是闪现。
            这条基线收在面板内侧，左右都离圆角起弯处还有一段，不会与面板的圆角打架。 */}
        <div className="mt-4 flex gap-1 border-b border-ink/12">
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
                  on ? "font-extrabold text-ink" : "font-semibold text-ink/75 hover:text-ink",
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
        </div>
      </motion.div>

      {error && (
        <motion.div
          variants={pageItem}
          className={`${dangerBar} mb-5 flex shrink-0 items-center gap-3 text-[13px] text-danger`}
        >
          <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-danger" />
          <AlertIcon size={18} />
          <span className="flex-1">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={() => void load()}>
            重试
          </Button>
        </motion.div>
      )}

      {(managedStatusError || managedFilesError) && (
        <motion.div
          variants={pageItem}
          className={`${dangerBar} mb-5 flex shrink-0 items-center gap-3 text-[13px] text-danger`}
          role="alert"
        >
          <span aria-hidden className="absolute inset-y-0 left-0 w-[3px] bg-danger" />
          <AlertIcon size={18} />
          <span className="flex-1">{managedStatusError ?? managedFilesError}</span>
          <Button
            variant="secondary"
            icon={<RefreshIcon size={15} />}
            onClick={() => {
              void loadManagedStatus(true);
              void loadManagedFiles();
            }}
          >
            重试
          </Button>
        </motion.div>
      )}

      <motion.div variants={pageItem} className="flex min-h-0 flex-1 flex-col">
        {!data ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-[92px] w-full" />
            <Skeleton className="h-[160px] w-full" delay={0.08} />
            <Skeleton className="h-[72px] w-full" delay={0.16} />
          </div>
        ) : (
          <AnimatePresence mode="wait" initial={false}>
            {/* 变体标签而非对象字面量：此刻这里面还没有带 variants 的子元素，所以对象写法也看不出毛病，
                但加进第一个就会永久透明（设置页已经这么中过一次），原委见 motion.ts 的 tabPanel。 */}
            <motion.div
              key={tab}
              variants={tabPanel}
              initial="hidden"
              animate="show"
              exit="exit"
              transition={springs.tap}
              className="flex min-h-0 flex-1 flex-col"
            >
              {tab === "overview" && (
                /*
                 * 概览是本页唯一「自然长度不设上限」的一面：整合包面板、崩溃诊断（正文来自日志分析，
                 * 条数没有上界）、身份条、实例设置、待处理清单顺排下来，在 960x720 的内容盒（618）里必超。
                 * 它又不像另外两面那样内部只有一条长清单可以单独关进滚动区——五块都要按顺序读，
                 * 所以滚的是这一面的整篇正文，滚动区就开在页签内容区这一层。
                 *
                 * 为什么必须开出口而不是把内容压紧：外壳是 overflow-clip（开发期换 hidden，见 app.css
                 * 第六之三节），溢出的那截不是滚出去而是被无声裁掉；更糟的是 hidden 仍吃编程滚动，
                 * 「实例描述」输入框一旦落在被裁的那截里，敲键时浏览器会把它滚进视野、随即被复位，
                 * 表现成「打一个字就丢焦点」。给这一面一个真出口，两件事一起消失。
                 *
                 * 只有这一面在外层滚：另外两面（内容 / 变更史）的长清单各自在面板内部滚，
                 * 它们的根是 min-h-0 flex-1 的分配式布局，本身不会超出，外层再套一层滚动
                 * 就成了同一根轴上的双层滚动。所以外层滚动区只加在这个分支里，不加在共用的页签容器上。
                 * 概览内部也没有任何自带滚动的子件（ManagedModpackPanel / CrashBanner 都是自然高度），
                 * 这一层是它唯一的滚动轴。
                 */
                <div className="min-h-0 flex-1 overflow-y-auto pr-1">
                  {managedStatusLoading && managedStatus === null && (
                    <Skeleton className="mb-5 h-[196px] w-full" />
                  )}
                  {managedStatus && (
                    <div className="mb-5">
                      <ManagedModpackPanel
                        status={managedStatus}
                        instanceId={versionId}
                        sync={modpackSync}
                        onCheck={() => {
                          setModpackSync({ kind: "idle" });
                          void loadManagedStatus(true);
                        }}
                        onSync={(targetVersion) => void runModpackSync(targetVersion)}
                        onInstallNewVersion={() =>
                          navigate(managedModpackInstallRoute(managedStatus.subscription.pointer_url))
                        }
                      />
                    </div>
                  )}
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
                </div>
              )}
              {tab === "content" && (
                <ContentTab
                  versionId={versionId}
                  rows={rows}
                  ownershipError={managedFilesError}
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
