// 设置页：接通 aurora-core 配置（getConfig/updateConfig/setGameDirectory）+ Java 检测/下载 + 无障碍减少动效。
// 保存策略：Select/Toggle 即时保存（乐观更新，失败回滚 + toast）；数字输入 blur/Enter 提交；
// 游戏目录与 client_id 走显性按钮。错误一律 toast(String(e),"error")，不吞不掩盖。

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { LiquidGlass } from "../components/LiquidGlass";
import {
  PackageIcon,
  AlertIcon,
  CubeIcon,
  DownloadIcon,
  RefreshIcon,
  SaveIcon,
  SparkleIcon,
} from "../components/icons";
import { BackgroundPicker } from "../components/BackgroundPicker";
import { useToast } from "../components/Toast";
import { useMotionPref } from "../lib/motion-pref";
import { pageItem, springs } from "../lib/motion";
import { checkUpdate, installUpdate, type UpdateStatus } from "../lib/updater";
import {
  addGameDirectory,
  discoverGameDirectories,
  getConfig,
  listGameDirectories,
  removeGameDirectory,
  switchGameDirectory,
  updateConfig,
  setGameDirectory,
  setGlassMode,
  detectJava,
  installJava,
  onCoreEvent,
  type ConfigDto,
  type ConfigPatch,
  type DownloadSourcePolicy,
  type GameDirectoryEntry,
  type IsolationPolicy,
  type JavaInstallationDto,
  type GlassMode,
  type NamedDirectory,
} from "../lib/ipc";
import { useAppearance } from "../lib/appearance-context";

// 输入框取下沉档。它是寄生层，永远套在分区卡片里，不会直接压在照片上，符合寄生层的落位约束。
// 描边交给 .surface-sunken 的 inset 阴影，不再自带 border：加减材质类时盒子尺寸不变，省掉一次整页重排。
const inputCls =
  "surface-sunken w-full rounded-control px-3.5 py-2.5 text-[14px] text-ink transition-colors placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

/**
 * 带内嵌提交按钮的输入框：按钮压在输入框右侧内部，不再与输入框并排。
 *
 * 并排的写法在窄栏里会把按钮挤到只剩一个字宽，中文于是竖排成两行。内嵌之后按钮宽度由自身内容
 * 决定、绝不被压缩，输入框用右内边距给它让位。
 */
function InputWithAction({
  label,
  value,
  placeholder,
  onChange,
  onSubmit,
  actionLabel,
  pendingLabel,
  pending,
}: {
  label: string;
  value: string;
  placeholder?: string;
  onChange: (v: string) => void;
  onSubmit: () => void;
  actionLabel: string;
  pendingLabel: string;
  pending: boolean;
}) {
  return (
    <div className="relative">
      <input
        type="text"
        aria-label={label}
        placeholder={placeholder}
        className={`${inputCls} pr-[104px]`}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onSubmit();
        }}
      />
      <button
        type="button"
        onClick={onSubmit}
        disabled={pending}
        className={[
          "absolute top-1 right-1 bottom-1 inline-flex shrink-0 cursor-pointer items-center gap-1.5",
          "rounded-chip px-3 text-[13px] font-bold whitespace-nowrap",
          // 实心墨底而不是再铺一层墨洗：它压在输入框的下沉层上，两层墨洗相加会越过单元素 8% 的浓度上限。
          "bg-ink text-paper-on transition-colors hover:bg-accent",
          "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent",
          "disabled:pointer-events-none disabled:opacity-45",
        ].join(" ")}
      >
        <SaveIcon size={14} />
        {pending ? pendingLabel : actionLabel}
      </button>
    </div>
  );
}

const DOWNLOAD_SOURCE_OPTIONS: { value: DownloadSourcePolicy; label: string }[] = [
  { value: "auto", label: "自动（按网络择优）" },
  { value: "official_first", label: "官方源优先" },
  { value: "mirror_first", label: "镜像源优先" },
];

// 两档写成人话而不是 frost / liquid：设置项要让人在不知道内部术语的前提下选得对。
const GLASS_OPTIONS: { value: GlassMode; label: string }[] = [
  { value: "frost", label: "磨砂玻璃" },
  { value: "liquid", label: "液态玻璃" },
];

const ISOLATION_OPTIONS: { value: IsolationPolicy; label: string }[] = [
  { value: "disabled", label: "关闭隔离" },
  { value: "mod_loaders_only", label: "仅 Mod 加载器版本" },
  { value: "non_release_only", label: "仅非正式版本" },
  { value: "mod_loaders_and_non_release", label: "Mod 加载器与非正式版本" },
  { value: "all", label: "全部版本隔离" },
];

type SettingsTab = "launcher" | "game";

/**
 * 一级分区。划分依据是「这条设置改的是谁的行为」：
 * 启动器 tab 管启动器自己（下哪里、放哪里、怎么登录、界面动效），
 * 游戏 tab 管被启动的那个游戏（吃多少内存、文件隔不隔离、用哪个 Java）。
 */
const TABS: { key: SettingsTab; label: string; icon: typeof SparkleIcon; subtitle: string }[] = [
  { key: "launcher", label: "启动器", icon: SparkleIcon, subtitle: "下载源、目录与登录凭据" },
  { key: "game", label: "游戏", icon: CubeIcon, subtitle: "内存、版本隔离与 Java 运行时" },
];

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

const JAVA_SOURCE_LABEL: Record<JavaInstallationDto["source"], string> = {
  registry: "注册表",
  common_dir: "常用目录",
  path: "PATH",
  managed: "托管",
};

/**
 * 分区外壳：标题收进卡片内部，整块参与页面 stagger 入场。
 *
 * 标题原先摆在卡片外面。背景图铺满全站之后那里是裸露的照片，11px 的小标题没有任何底可依，
 * 遇上亮图直接消失；收进卡片就自动落在分区卡片那一档材质的对比度预算里。
 */
function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <motion.div variants={pageItem} className="mt-5 first:mt-0">
      <Card>
        <h2 className="mb-3.5 text-[11px] font-bold tracking-[0.22em] text-ink/75">{title}</h2>
        <div>{children}</div>
      </Card>
    </motion.div>
  );
}

/**
 * 设置行：左侧标题+说明，右侧控件槽（定宽）。
 *
 * 分隔线从 ink/9 加到 ink/12。设置页是全站最密的「标题+说明+控件」堆叠，而分区卡片本身半透明，
 * 底下照片的纹理会把太淡的发丝线吃掉，行与行于是糊成一片；ink/12 在纯黑与纯白两端都还立得住。
 */
function Row({ title, desc, control }: { title: string; desc: string; control: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-ink/12 py-[18px] first:pt-0 last:border-b-0 last:pb-0">
      <div className="min-w-0">
        <div className="text-[15px] font-bold">{title}</div>
        <div className="mt-1 text-[12.5px] text-ink/75">{desc}</div>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

export function Settings() {
  const { toast } = useToast();
  const { reduceMotion, setReduceMotion } = useMotionPref();
  // 外观走全局 context 而不是本页各拉一次：改完当场生效，否则外壳还按旧材质渲染。
  const { appearance, applyAppearance } = useAppearance();

  const [tab, setTab] = useState<SettingsTab>("launcher");

  // ---- 启动器更新 ----
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState("");

  // ---- 游戏目录列表 ----
  const [dirs, setDirs] = useState<GameDirectoryEntry[]>([]);
  const [discovered, setDiscovered] = useState<NamedDirectory[]>([]);
  const [dirsBusy, setDirsBusy] = useState(false);

  // ---- 配置区 ----
  const [config, setConfig] = useState<ConfigDto | null>(null);
  const [configLoading, setConfigLoading] = useState(true);
  const [configError, setConfigError] = useState<string | null>(null);

  // 数字/文本输入的本地镜像（仅在载入时同步，避免打字被服务端值覆盖）。
  const [concurrencyInput, setConcurrencyInput] = useState("");
  const [maxMemInput, setMaxMemInput] = useState("");
  const [minMemInput, setMinMemInput] = useState("");
  const [gameDirInput, setGameDirInput] = useState("");
  const [clientIdInput, setClientIdInput] = useState("");
  const [savingGameDir, setSavingGameDir] = useState(false);
  const [savingClientId, setSavingClientId] = useState(false);

  // ---- Java 区 ----
  const [javas, setJavas] = useState<JavaInstallationDto[] | null>(null);
  const [javaLoading, setJavaLoading] = useState(true);
  const [javaError, setJavaError] = useState<string | null>(null);
  const [javaMajorInput, setJavaMajorInput] = useState("21");
  const [installing, setInstalling] = useState(false);
  const [coreStatus, setCoreStatus] = useState<string | null>(null);

  const loadConfig = async (silent = false) => {
    if (!silent) {
      setConfigLoading(true);
      setConfigError(null);
    }
    try {
      const c = await getConfig();
      setConfig(c);
      setConcurrencyInput(String(c.download_concurrency));
      setMaxMemInput(String(c.memory.max_mb));
      setMinMemInput(c.memory.min_mb === null ? "" : String(c.memory.min_mb));
      setGameDirInput(c.game_dir);
    } catch (e) {
      if (silent) toast(String(e), "error");
      else setConfigError(String(e));
    } finally {
      if (!silent) setConfigLoading(false);
    }
  };

  const loadJava = async () => {
    setJavaLoading(true);
    setJavaError(null);
    try {
      setJavas(await detectJava());
    } catch (e) {
      setJavaError(String(e));
    } finally {
      setJavaLoading(false);
    }
  };

  useEffect(() => {
    void loadConfig();
    void loadJava();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 安装 Java 期间的阶段/下载进度反馈（onCoreEvent 是全局流，仅安装态展示）。
  useEffect(() => {
    let un: (() => void) | undefined;
    void onCoreEvent((ev) => {
      if (ev.kind === "stage") setCoreStatus(ev.message);
      else if (ev.kind === "warning") setCoreStatus(`警告：${ev.message}`);
      else if (ev.kind === "download") {
        const pct = ev.total > 0 ? Math.round((ev.finished / ev.total) * 100) : 0;
        setCoreStatus(`下载中 ${ev.finished}/${ev.total}（${pct}%）`);
      }
    }).then((u) => {
      un = u;
    });
    return () => un?.();
  }, []);

  // 乐观保存：先落本地，失败回滚到 prev 并 toast。
  const save = async (patch: ConfigPatch, next: (c: ConfigDto) => ConfigDto) => {
    if (!config) return;
    const prev = config;
    setConfig(next(prev));
    try {
      await updateConfig(patch);
      toast("已保存", "success");
    } catch (e) {
      setConfig(prev);
      toast(String(e), "error");
    }
  };

  // 玻璃模式没有乐观更新：它只有两档、后端一个赋值就返回，抢那一次往返不值得多一条回滚路径。
  // 失败照常 toast 并保持旧值——DTO 没换，界面上那个 Select 也就还停在原来那一档。
  const applyGlass = async (glass: GlassMode) => {
    try {
      applyAppearance(await setGlassMode(glass));
      toast("已保存", "success");
    } catch (e) {
      toast(String(e), "error");
    }
  };

  const commitConcurrency = () => {
    if (!config) return;
    const n = Number.parseInt(concurrencyInput, 10);
    if (!Number.isInteger(n) || n < 1) {
      toast("并发数需为不小于 1 的整数", "error");
      setConcurrencyInput(String(config.download_concurrency));
      return;
    }
    if (n === config.download_concurrency) return;
    void save({ downloadConcurrency: n }, (c) => ({ ...c, download_concurrency: n }));
  };

  const commitMemory = () => {
    if (!config) return;
    const max = Number.parseInt(maxMemInput, 10);
    const revert = () => {
      setMaxMemInput(String(config.memory.max_mb));
      setMinMemInput(config.memory.min_mb === null ? "" : String(config.memory.min_mb));
    };
    if (!Number.isInteger(max) || max < 1) {
      toast("最大内存需为不小于 1 的整数（MB）", "error");
      revert();
      return;
    }
    const minTrim = minMemInput.trim();
    let min: number | null = null;
    if (minTrim !== "") {
      const m = Number.parseInt(minTrim, 10);
      if (!Number.isInteger(m) || m < 0) {
        toast("最小内存需为非负整数（MB）或留空", "error");
        revert();
        return;
      }
      min = m;
    }
    if (min !== null && min > max) {
      toast("最小内存不能大于最大内存", "error");
      revert();
      return;
    }
    if (max === config.memory.max_mb && min === config.memory.min_mb) return;
    const memory = { max_mb: max, min_mb: min };
    void save({ memory }, (c) => ({ ...c, memory }));
  };

  const applyGameDir = async () => {
    const path = gameDirInput.trim();
    if (path === "") {
      toast("游戏目录不能为空", "error");
      return;
    }
    setSavingGameDir(true);
    try {
      await setGameDirectory(path);
      toast("游戏目录已更新", "success");
      await loadConfig(true); // 目录变更可能连带影响 data_dir，拉回真实值
      await loadDirs();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSavingGameDir(false);
    }
  };

  // 列表与探测一起取：两者都不慢，分开取会让「添加」之后的两块出现短暂不一致。
  const loadDirs = useCallback(async () => {
    try {
      const [listed, found] = await Promise.all([
        listGameDirectories(),
        discoverGameDirectories(),
      ]);
      setDirs(listed);
      setDiscovered(found);
    } catch (e) {
      // 目录列表取不到不该顶掉整个设置页，用 toast 报出来即可。
      toast(String(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void loadDirs();
  }, [loadDirs]);

  const adoptDir = async (name: string, path: string) => {
    setDirsBusy(true);
    try {
      await addGameDirectory(name, path);
      await loadDirs();
      toast(`已添加 ${name}`, "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setDirsBusy(false);
    }
  };

  const removeDir = async (path: string) => {
    setDirsBusy(true);
    try {
      await removeGameDirectory(path);
      await loadDirs();
      toast("已移除记录，磁盘文件未改动", "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setDirsBusy(false);
    }
  };

  const doCheckUpdate = async () => {
    setChecking(true);
    try {
      setUpdate(await checkUpdate());
    } finally {
      setChecking(false);
    }
  };

  const doInstallUpdate = async () => {
    setUpdating(true);
    setUpdateProgress("正在下载…");
    try {
      await installUpdate((downloaded, total) => {
        const mb = (n: number) => (n / 1024 / 1024).toFixed(1);
        // 服务端没给 Content-Length 时只报已下载量，不编一个假的百分比。
        setUpdateProgress(
          total === null
            ? `已下载 ${mb(downloaded)} MB`
            : `已下载 ${mb(downloaded)} / ${mb(total)} MB`,
        );
      });
      // 走到这里通常已经在重启了，留一句兜底文案应对重启被系统拦下的情形。
      setUpdateProgress("安装完成，正在重启…");
    } catch (e) {
      toast(String(e), "error");
      setUpdating(false);
      setUpdateProgress("");
    }
  };

  /** 更新区那一行说明文字：不同状态各说各的，不含糊成一句「点按钮试试」。 */
  const updateHint = (() => {
    if (checking) return "正在向更新服务器询问…";
    switch (update?.kind) {
      case "unsupported":
        return "浏览器预览模式下不支持更新，请在安装后的启动器里检查";
      case "up-to-date":
        return "已是最新版本";
      case "available":
        return `有新版本 ${update.version}${update.date ? `，发布于 ${update.date.slice(0, 10)}` : ""}`;
      case "error":
        return `检查失败：${update.message}`;
      default:
        return "检查是否有新版本可用";
    }
  })();

  const switchDir = async (path: string, name: string) => {
    setDirsBusy(true);
    try {
      await switchGameDirectory(path, name);
      // 当前目录换了，配置与列表都要重取；输入框也得跟上，否则还显示旧路径。
      await loadConfig(true);
      await loadDirs();
      toast(`已切换到 ${name}`, "success");
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setDirsBusy(false);
    }
  };

  const applyClientId = async () => {
    const id = clientIdInput.trim();
    if (id === "") {
      toast("请填写微软 client_id", "error");
      return;
    }
    setSavingClientId(true);
    try {
      await updateConfig({ clientId: id });
      toast("微软 client_id 已设置", "success");
      setClientIdInput("");
      await loadConfig(true); // 刷新 has_client_id 状态（不回显原值）
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSavingClientId(false);
    }
  };

  const doInstallJava = async () => {
    const major = Number.parseInt(javaMajorInput, 10);
    if (!Number.isInteger(major) || major < 1) {
      toast("请输入有效的 Java 主版本号（如 21）", "error");
      return;
    }
    setInstalling(true);
    setCoreStatus(null);
    try {
      const rt = await installJava(major);
      toast(`已安装 Java ${rt.version.raw}`, "success");
      await loadJava();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setInstalling(false);
      setCoreStatus(null);
    }
  };

  const activeTab = TABS.find((t) => t.key === tab)!;

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title="设置" subtitle={activeTab.subtitle} />
      </motion.div>

      {/*
        一级分区：启动器自身的行为 vs 影响游戏运行的设置。

        从下划线页签改成分段控件。理由是背景图铺满全站之后，原先那排标签直接裸露在照片上，
        既没有底也没法靠色阶救——一条 2px 下划线撑不起一整排文字的可读性。
        容器给一档面板玻璃把文字接住，选中页用液态玻璃：它是设计契约点名允许液态的四处之一，
        面积也确实只有一颗控件那么大。layoutId 让那块玻璃在两页之间滑过去，
        比一根下划线更能交代「当前在哪一页」。
      */}
      <motion.div variants={pageItem} className="mb-6">
        <div className="surface-panel inline-flex gap-1 rounded-panel p-1.5">
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
                  "relative flex cursor-pointer items-center gap-2 rounded-control px-4 py-2 text-[14px] transition-colors",
                  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                  on ? "font-extrabold text-ink" : "font-semibold text-ink/75 hover:text-ink",
                ].join(" ")}
              >
                {on && (
                  // 分两层是被 backdrop-filter 的语义逼出来的，不是包一层图省事：
                  // 外层只拿 layoutId 做位移动画，不带任何材质；纸与透镜一起落在内层。
                  // 若把纸留在外层，透镜采到的背景里已经含了这张纸，折射的就不再是照片。
                  <motion.div
                    layoutId="settings-tab-active"
                    aria-hidden="true"
                    className="absolute inset-0"
                    transition={springs.tap}
                  >
                    <LiquidGlass
                      {...LIQUID_LENS[appearance.glass]}
                      className="surface-liquid surface-nested h-full w-full rounded-control"
                    />
                  </motion.div>
                )}
                {/* 选中片是绝对定位的兄弟节点，会盖住普通流里的文字；内容自己定位一次才回到它上面。 */}
                <span className="relative flex items-center gap-2">
                  <Icon size={16} />
                  {t.label}
                </span>
              </button>
            );
          })}
        </div>
      </motion.div>

      {configLoading && (
        <motion.div variants={pageItem}>
          <Card>
            <p className="py-2 text-[13.5px] text-ink/75">载入配置中…</p>
          </Card>
        </motion.div>
      )}

      {!configLoading && configError && (
        <motion.div variants={pageItem}>
          {/* 危险描边走 outline：Card 的底改成材质类之后描边是 inset 阴影，外部再传 border-* 已经没有着力点，
              而 outline 既不参与 box-shadow 的叠加、也不撑大盒子。 */}
          <Card className="outline-2 outline-danger/45">
            <div className="flex items-start gap-3">
              <span className="text-danger">
                <AlertIcon size={20} />
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-[14px] font-bold text-danger">配置载入失败</div>
                <p className="mt-1 text-[13px] break-words text-ink/75">{configError}</p>
                <div className="mt-3">
                  <Button variant="secondary" icon={<RefreshIcon size={16} />} onClick={() => void loadConfig()}>
                    重试
                  </Button>
                </div>
              </div>
            </div>
          </Card>
        </motion.div>
      )}

      {/* 页签正文改成淡入淡出：切页签只是换内容不是换页面，与 Download / InstanceDetail 用同一套过渡语言，整片瞬切会让人以为页面被重载。 */}
      {/* 外层 pageItem 让整片正文作为一个单元参与页面 stagger：AnimatePresence 的 initial={false} 会连带压掉内部 Section 的首屏入场。 */}
      <motion.div variants={pageItem} className="mt-7">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={tab}
            initial={{ opacity: 0, y: 6 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -4 }}
            transition={springs.tap}
          >
            {tab === "launcher" && (
              <>
                {config && (
                  <>
                    <Section title="下载与更新">
                      <Row
                        title="下载源策略"
                        desc="下载游戏文件时官方源与镜像源的取舍"
                        control={
                          <div className="w-[240px]">
                            <Select
                              ariaLabel="下载源策略"
                              value={config.download_source}
                              options={DOWNLOAD_SOURCE_OPTIONS}
                              onChange={(v) => void save({ downloadSource: v }, (c) => ({ ...c, download_source: v }))}
                            />
                          </div>
                        }
                      />
                      <Row
                        title="版本列表源"
                        desc="拉取版本清单（manifest）时的来源策略"
                        control={
                          <div className="w-[240px]">
                            <Select
                              ariaLabel="版本列表源"
                              value={config.version_list_source}
                              options={DOWNLOAD_SOURCE_OPTIONS}
                              onChange={(v) => void save({ versionListSource: v }, (c) => ({ ...c, version_list_source: v }))}
                            />
                          </div>
                        }
                      />
                      <Row
                        title="下载并发数"
                        desc="同时进行的下载任务上限"
                        control={
                          <input
                            type="number"
                            min={1}
                            inputMode="numeric"
                            aria-label="下载并发数"
                            className={`${inputCls} w-24 text-right tabular-nums`}
                            value={concurrencyInput}
                            onChange={(e) => setConcurrencyInput(e.target.value)}
                            onBlur={commitConcurrency}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") e.currentTarget.blur();
                            }}
                          />
                        }
                      />
                    </Section>

                    <Section title="目录">
                      <div className="border-b border-ink/12 py-[18px] first:pt-0">
                        <div className="text-[15px] font-bold">游戏目录</div>
                        <div className="mt-1 text-[12.5px] text-ink/75">.minecraft 所在位置，变更后需重新扫描版本</div>
                        <div className="mt-3">
                          <InputWithAction
                            label="游戏目录"
                            value={gameDirInput}
                            onChange={setGameDirInput}
                            onSubmit={() => void applyGameDir()}
                            actionLabel="应用"
                            pendingLabel="应用中"
                            pending={savingGameDir}
                          />
                        </div>
                      </div>
                      <div className="border-b border-ink/12 py-[18px]">
                        <div className="flex items-center justify-between gap-4">
                          <div>
                            <div className="text-[15px] font-bold">文件夹列表</div>
                            <div className="mt-1 text-[12.5px] text-ink/75">
                              可以并存多个 .minecraft，点「切换」把某个设为当前；移除只删记录，不动磁盘文件
                            </div>
                          </div>
                          <Button
                            variant="secondary"
                            className="shrink-0"
                            icon={<RefreshIcon size={15} />}
                            onClick={() => void loadDirs()}
                            disabled={dirsBusy}
                          >
                            重新探测
                          </Button>
                        </div>

                        <ul className="m-0 mt-3 flex list-none flex-col gap-1.5 p-0">
                          {dirs.map((d) => (
                            <li
                              key={d.path}
                              className="surface-sunken flex items-center gap-3 rounded-control px-3 py-2.5"
                            >
                              <span className="min-w-0 flex-1">
                                <span className="flex items-center gap-2">
                                  <span className="truncate text-[13.5px] font-bold">{d.name}</span>
                                  {/* 「当前」用实心朱红配纸色字，不用朱红洗底配朱红字：10px 的朱红压在半透明朱红上，
                                      在玻璃那一档底色上算不到 4.5，而它恰恰是这一列里最该一眼认出来的记号。 */}
                                  {d.is_current && (
                                    <span className="shrink-0 rounded-chip bg-accent px-1.5 py-0.5 text-[10px] font-bold tracking-[0.08em] text-paper-on">
                                      当前
                                    </span>
                                  )}
                                  {!d.available && (
                                    <span
                                      title="这个位置现在访问不到（盘没挂或已被删除），记录仍然保留"
                                      className="shrink-0 rounded-chip border border-ink/20 px-1.5 py-0.5 text-[10px] font-bold text-ink/75"
                                    >
                                      不可达
                                    </span>
                                  )}
                                </span>
                                <span className="mt-0.5 block truncate font-mono text-[11px] text-ink/75">
                                  {d.path}
                                </span>
                              </span>
                              {!d.is_current && (
                                <>
                                  <button
                                    type="button"
                                    onClick={() => void switchDir(d.path, d.name)}
                                    disabled={dirsBusy || !d.available}
                                    title={d.available ? undefined : "位置访问不到，无法切过去"}
                                    className="shrink-0 cursor-pointer rounded-chip px-2 py-1 text-[11.5px] font-bold text-ink/75 transition-colors hover:bg-ink hover:text-paper-on focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
                                  >
                                    切换
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => void removeDir(d.path)}
                                    disabled={dirsBusy}
                                    className="shrink-0 cursor-pointer rounded-chip px-2 py-1 text-[11.5px] font-bold text-ink/75 transition-colors hover:text-danger focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
                                  >
                                    移除
                                  </button>
                                </>
                              )}
                            </li>
                          ))}
                        </ul>

                        {discovered.length > 0 && (
                          <div className="surface-sunken mt-3 rounded-panel px-3 py-2.5">
                            <div className="text-[12px] font-bold text-ink/75">发现未记录的文件夹</div>
                            <ul className="m-0 mt-2 flex list-none flex-col gap-1.5 p-0">
                              {discovered.map((d) => (
                                <li key={d.path} className="flex items-center gap-3">
                                  <span className="min-w-0 flex-1">
                                    <span className="block truncate text-[12.5px] font-bold">{d.name}</span>
                                    <span className="block truncate font-mono text-[11px] text-ink/75">
                                      {d.path}
                                    </span>
                                  </span>
                                  <button
                                    type="button"
                                    onClick={() => void adoptDir(d.name, d.path)}
                                    disabled={dirsBusy}
                                    className="shrink-0 cursor-pointer rounded-chip px-2 py-1 text-[11.5px] font-bold text-ink/75 transition-colors hover:bg-ink hover:text-paper-on focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
                                  >
                                    添加
                                  </button>
                                </li>
                              ))}
                            </ul>
                          </div>
                        )}
                      </div>

                      <div className="py-[18px] last:pb-0">
                        <div className="text-[12.5px] text-ink/75">数据目录</div>
                        <div className="mt-1 font-mono text-[12px] break-all text-ink/75">{config.data_dir}</div>
                      </div>
                    </Section>

                    <Section title="账户凭据">
                      <div className="py-[18px] first:pt-0 last:pb-0">
                        <div className="flex items-center justify-between gap-4">
                          <div className="text-[15px] font-bold">微软 client_id</div>
                          <span
                            className={[
                              "rounded-chip px-2 py-0.5 text-[11px] font-bold tracking-[0.08em]",
                              config.has_client_id ? "bg-ink text-paper-on" : "border border-ink/20 text-ink/75",
                            ].join(" ")}
                          >
                            {config.has_client_id ? "已配置" : "未配置"}
                          </span>
                        </div>
                        <div className="mt-1 text-[12.5px] text-ink/75">
                          自定义 Azure 应用的 client_id，用于微软正版登录；出于安全不回显已保存的值
                        </div>
                        <div className="mt-3">
                          <InputWithAction
                            label="微软 client_id"
                            placeholder={config.has_client_id ? "输入以覆盖现有 client_id" : "输入 client_id"}
                            value={clientIdInput}
                            onChange={setClientIdInput}
                            onSubmit={() => void applyClientId()}
                            actionLabel="保存"
                            pendingLabel="保存中"
                            pending={savingClientId}
                          />
                        </div>
                      </div>
                    </Section>
                  </>
                )}

                {/*
                  外观一节按「先定底图，再定压在图上的材质，最后定动效」重排，原先是动效在前、背景在后。
                  玻璃模式（frost / liquid）改的是所有材质处理这张图的方式，人得先看见图才谈得上挑材质，
                  所以它的位置在背景与动效之间。
                  选中值与背景同属一份外观配置（后端 AppearanceSettings），不另立存储；
                  往 documentElement 写 data-glass 的活由 AppearanceProvider 统一做，这里只负责改配置。
                */}
                <Section title="外观">
                  {/* BackgroundPicker 根节点自带 first:pt-0 / last:pb-0，作为独子会把自己的上下内边距全部归零，
                      所以下边距由这层包裹给，否则它会直接贴上分隔线。 */}
                  <div className="border-b border-ink/12 pb-[18px]">
                    <BackgroundPicker />
                  </div>
                  <Row
                    title="玻璃模式"
                    desc="磨砂只有模糊；液态再给主按钮、Toast 与分段控件加一道受光亮边与斜向高光"
                    control={
                      <div className="w-[240px]">
                        <Select
                          ariaLabel="玻璃模式"
                          value={appearance.glass}
                          options={GLASS_OPTIONS}
                          onChange={(v) => void applyGlass(v)}
                        />
                      </div>
                    }
                  />
                  <Row
                    title="减少动态效果"
                    desc="降低或关闭界面动画（防晕动 / 低性能设备）"
                    control={
                      <Toggle ariaLabel="减少动态效果" checked={reduceMotion} onChange={setReduceMotion} />
                    }
                  />
                </Section>

                <Section title="关于">
                  <div className="py-[18px] first:pt-0 last:pb-0">
                    <div className="flex items-center justify-between gap-4">
                      <div className="min-w-0">
                        <div className="text-[15px] font-bold">启动器更新</div>
                        <div className="mt-1 text-[12.5px] text-ink/75">{updateHint}</div>
                      </div>
                      <div className="flex shrink-0 items-center gap-2.5">
                        {update?.kind === "available" ? (
                          <Button
                            variant="primary"
                            icon={<DownloadIcon size={16} />}
                            onClick={() => void doInstallUpdate()}
                            disabled={updating}
                          >
                            {updating ? "更新中" : `更新到 ${update.version}`}
                          </Button>
                        ) : (
                          <Button
                            variant="secondary"
                            icon={<RefreshIcon size={16} />}
                            onClick={() => void doCheckUpdate()}
                            disabled={checking || update?.kind === "unsupported"}
                          >
                            {checking ? "检查中" : "检查更新"}
                          </Button>
                        )}
                      </div>
                    </div>

                    {update?.kind === "available" && update.notes && (
                      <p className="surface-sunken mt-3 mb-0 rounded-panel px-3 py-2.5 text-[12.5px] leading-relaxed whitespace-pre-wrap text-ink/75">
                        {update.notes}
                      </p>
                    )}

                    {updating && (
                      <p className="mt-2.5 font-mono text-[12px] text-ink/75 tabular-nums">
                        {updateProgress}
                      </p>
                    )}
                  </div>
                </Section>
              </>
            )}

            {tab === "game" && (
              <>
                {config && (
                  <Section title="运行时">
                    <Row
                      title="内存分配（MB）"
                      desc="最大 / 最小堆内存，最小留空表示不限制"
                      control={
                        <div className="flex items-center gap-2">
                          <input
                            type="number"
                            min={1}
                            inputMode="numeric"
                            aria-label="最大内存 MB"
                            className={`${inputCls} w-24 text-right tabular-nums`}
                            value={maxMemInput}
                            onChange={(e) => setMaxMemInput(e.target.value)}
                            onBlur={commitMemory}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") e.currentTarget.blur();
                            }}
                          />
                          <span className="text-ink/75">/</span>
                          <input
                            type="number"
                            min={0}
                            inputMode="numeric"
                            aria-label="最小内存 MB"
                            placeholder="不限"
                            className={`${inputCls} w-24 text-right tabular-nums`}
                            value={minMemInput}
                            onChange={(e) => setMinMemInput(e.target.value)}
                            onBlur={commitMemory}
                            onKeyDown={(e) => {
                              if (e.key === "Enter") e.currentTarget.blur();
                            }}
                          />
                        </div>
                      }
                    />
                    <Row
                      title="版本隔离档位"
                      desc="决定哪些版本使用独立的存档与配置目录"
                      control={
                        <div className="w-[240px]">
                          <Select
                            ariaLabel="版本隔离档位"
                            value={config.isolation_policy}
                            options={ISOLATION_OPTIONS}
                            onChange={(v) => void save({ isolationPolicy: v }, (c) => ({ ...c, isolation_policy: v }))}
                          />
                        </div>
                      }
                    />
                  </Section>
                )}

                <Section title="Java 运行时">
                  {config && (
                    <Row
                      title="自动下载 Java"
                      desc="启动缺少匹配运行时时自动获取对应 JRE"
                      control={
                        <Toggle
                          ariaLabel="自动下载 Java"
                          checked={config.auto_download_java}
                          onChange={(v) => void save({ autoDownloadJava: v }, (c) => ({ ...c, auto_download_java: v }))}
                        />
                      }
                    />
                  )}
                  <div className="border-b border-ink/12 py-[18px]">
                    <div className="mb-3 flex items-center justify-between gap-4">
                      <div className="text-[15px] font-bold">本机检测</div>
                      <Button
                        variant="secondary"
                        icon={<RefreshIcon size={16} />}
                        onClick={() => void loadJava()}
                        disabled={javaLoading}
                      >
                        {javaLoading ? "扫描中…" : "重新扫描"}
                      </Button>
                    </div>

                    {javaLoading && <p className="py-2 text-[13.5px] text-ink/75">扫描本机 Java…</p>}

                    {!javaLoading && javaError && (
                      <div className="flex items-start gap-3 py-1">
                        <span className="text-danger">
                          <AlertIcon size={18} />
                        </span>
                        <p className="text-[13px] break-words text-ink/75">{javaError}</p>
                      </div>
                    )}

                    {!javaLoading && !javaError && javas && javas.length === 0 && (
                      <EmptyState icon={<PackageIcon />} title="未检测到本机 Java" />
                    )}

                    {!javaLoading && !javaError && javas && javas.length > 0 && (
                      <ul className="flex flex-col gap-2">
                        {javas.map((j) => (
                          <li
                            key={j.path}
                            className="surface-sunken flex items-center justify-between gap-4 rounded-control px-3.5 py-2.5"
                          >
                            <div className="min-w-0">
                              <div className="flex items-center gap-2">
                                <span className="text-[14px] font-bold tabular-nums">Java {j.version.major}</span>
                                <span className="font-mono text-[11px] text-ink/75">{j.version.raw}</span>
                              </div>
                              <div className="mt-0.5 truncate font-mono text-[11.5px] text-ink/75">{j.path}</div>
                            </div>
                            <div className="flex shrink-0 items-center gap-1.5">
                              <span className="rounded-chip border border-ink/16 px-2 py-0.5 text-[11px] text-ink/75">
                                {j.is_64bit ? "64 位" : "32 位"}
                              </span>
                              <span className="rounded-chip border border-ink/16 px-2 py-0.5 text-[11px] text-ink/75">
                                {JAVA_SOURCE_LABEL[j.source]}
                              </span>
                            </div>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>

                  <div className="pt-[18px]">
                    <div className="text-[15px] font-bold">下载运行时</div>
                    <div className="mt-1 text-[12.5px] text-ink/75">按主版本号获取由启动器托管的 JRE（如 8 / 17 / 21）</div>
                    <div className="mt-3 flex items-center gap-2.5">
                      <input
                        type="number"
                        min={1}
                        inputMode="numeric"
                        aria-label="Java 主版本号"
                        className={`${inputCls} w-28 text-right tabular-nums`}
                        value={javaMajorInput}
                        onChange={(e) => setJavaMajorInput(e.target.value)}
                        disabled={installing}
                      />
                      <Button variant="primary" onClick={() => void doInstallJava()} disabled={installing}>
                        {installing ? "安装中…" : "下载运行时"}
                      </Button>
                    </div>
                    {installing && coreStatus && (
                      <p className="mt-2.5 font-mono text-[12px] break-words text-ink/75">{coreStatus}</p>
                    )}
                  </div>
                </Section>
              </>
            )}
          </motion.div>
        </AnimatePresence>
      </motion.div>
    </>
  );
}
