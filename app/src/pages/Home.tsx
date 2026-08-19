// 主页：极简启动屏 —— 右下角一块信息面板，竖排 状态 / 当前版本 / 当前账户 / 启动键，上方整版留白。
// Aurora 收敛成 World of Kivotos 专用启动器之后，这一屏还兼了「把游戏装上」：
// 实例的唯一产生途径是安装 WOK 受管整合包，那套流程从下载页搬到了这里（下载页只剩玩家自己装 Mod）。
// 真调 IPC：入场并行 current_account + list_installed + get_config；启动走 launch_game + 日志窗；错误显式冒泡不吞。

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { CrashBanner } from "../components/CrashBanner";
import { LaunchControl, type LaunchPhase } from "../components/LaunchControl";
import { ModpackInstallFlow, type ModpackInstallState } from "../components/ModpackInstallFlow";
import { SkinHead } from "../components/SkinHead";
import { useToast } from "../components/Toast";
import { AlertIcon, RefreshIcon } from "../components/icons";
import { useAppearance } from "../lib/appearance-context";
import { plateMode } from "../lib/appearance";
import { pageItem, springs } from "../lib/motion";
import {
  createOfflineAccount,
  currentAccount,
  getConfig,
  installManagedModpack,
  launchGame,
  listInstalled,
  listMods,
  managedModpackFiles,
  managedModpackStatus,
  onCoreEvent,
  onGameCrash,
  onGameLog,
  stopGame,
  updateConfig,
  type AccountDto,
  type AccountType,
  type CrashReport,
  type GameLog,
  type InstalledVersionDto,
  type LaunchArgs,
  type ManagedModpackFile,
  type VersionScanDto,
} from "../lib/ipc";
import {
  parseModpackSyncError,
  validateModpackPointerUrl,
  type ManagedModpackStatus,
} from "../lib/modpack-ui";
import { modpackOwnerOf } from "../lib/modpack-ownership";
import { managedModpackInstallIntent } from "../lib/modpack-navigation";
import { notifyInstanceChanged } from "../lib/instance-signal";

// WOK 受管整合包的内置入口。原先写死在下载页的「整合包」tab 里，专用化之后那个 tab 撤了，
// 但它是这台启动器唯一的装游戏途径（install_managed_modpack 会连 MC 本体与加载器一并装好），
// 所以是搬过来而不是跟着 tab 一起删。
const BUILT_IN_MODPACK = {
  label: "WOK 地址",
  pointer_url: "https://api.mcwok.cn/api/v1/pack/latest",
};

// 首帧进度：真进度事件到达前先把步骤条钉在第一步，免得点下去到第一个事件之间是一段没有反馈的空白。
const INITIAL_MODPACK_INSTALL_PROGRESS = {
  stage: "resolving_manifest",
  completed_files: 0,
  total_files: 0,
  downloaded_bytes: 0,
  total_bytes: null,
  current_file: null,
} as const;

const ACCOUNT_TYPE_LABEL: Record<AccountType, string> = {
  microsoft: "微软正版",
  offline: "离线账户",
  authlib_injector: "外置登录",
};

function loaderText(v: InstalledVersionDto): string {
  if (v.loaders.length === 0) return "—";
  const l = v.loaders[0];
  return l.version ? `${l.kind} ${l.version}` : l.kind;
}

// 右下角那撮内容的三种形态。恒定铺一块材质的做法已被否掉: 一块纸压在照片上,
// 无论做得多透, 都还是在图里挖了一块出来; 真正融进去的做法是让字自己适应它压着的那片图。
//
// 全站铺图之后判定范围确实变了, 但启动屏仍是唯一把字裸压在图上的地方 ——
// 别的页面文字都落在容器材质上, 走 app.css 那张实算表; 这里走按图取样的 plateMode。
// 两套账各管各的场景, 不冲突。
const PLATE_BARE = "mt-auto flex flex-col items-end gap-6 pt-10";
// 裸字: 不铺底, 只靠字色。右对齐与间距沿用纸片那套, 换形态时版位不跳。
const PLATE_NAKED = "mt-auto ml-auto flex flex-col items-end gap-6 pt-10";
// 兜底纸片: 图明暗跨度大到两种字色都撑不住时才用。走信息密集档, 它最实。
const PLATE_FROSTED =
  "surface-panel-strong mt-auto ml-auto flex flex-col items-end gap-6 rounded-panel px-7 py-6";

/**
 * 游戏还没装上时占住 Start 的版位。
 *
 * 语义必须换掉：那颗 Start 在没有实例时只会是一颗按下去必然失败的按钮，
 * 而这一屏右下角是玩家眼里唯一的主操作位，它说什么，玩家就以为该做什么。
 *
 * 视觉沿用 LaunchControl 的裸字语言（不套任何材质、右缘对齐、朱红竖规压在右侧、
 * 字色随 onDark 反相），版位与量级不动。之所以不是给 LaunchControl 加一个模式：
 * 那颗按钮整套 rAF 手写动效是为「启动进度」写的，安装进度另有面板承载，
 * 塞进去只会让两件互不相干的进度互相牵制。
 */
function InstallControl({
  onDark,
  busy,
  onInstall,
}: {
  onDark: boolean;
  busy: boolean;
  onInstall: () => void;
}) {
  return (
    <motion.button
      type="button"
      onClick={onInstall}
      disabled={busy}
      aria-label="安装游戏"
      // 22px 是 LaunchControl 里字样右缘到竖线的同一个气口，右下角这两种形态换来换去时竖规不会跳。
      style={{ height: 62, paddingRight: 26 }}
      whileTap={{ scale: 0.98 }}
      transition={springs.tap}
      className="group relative inline-flex items-center focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-4 disabled:pointer-events-none disabled:opacity-40"
    >
      <span
        className={`text-[34px] leading-none font-extrabold tracking-[-0.01em] ${
          onDark ? "text-paper-on" : "text-ink"
        } transition-colors duration-200 group-hover:text-accent`}
      >
        {busy ? "安装中" : "安装游戏"}
      </span>
      <span
        aria-hidden
        className="absolute top-1/2 right-0 h-[42px] w-[4px] -translate-y-1/2 bg-accent"
      />
    </motion.button>
  );
}

export function Home() {
  const { toast } = useToast();
  const navigate = useNavigate();
  // 卷宗页遇到同步冲突时会把当前订阅地址带过来（managedModpackInstallRoute）。
  // 那条链路的落点就是这一屏，地址栏是它唯一的传话筒。
  const [searchParams] = useSearchParams();
  const installIntent = managedModpackInstallIntent(searchParams);
  const { appearance } = useAppearance();
  // 只剩一个用处：崩溃横条据此决定要不要投影（压在照片上是两个平面要有影，
  // 落在纯纸底上是纸对纸、由它自己挂 .surface-nested 摘掉）。字色与底色都归材质管，页面不再参与。
  const onPhoto = appearance.background !== null;
  // 有图时先问一句这块图撑不撑得住裸字。撑得住就不要纸片, 字直接压上去。
  const mode = onPhoto ? plateMode(appearance.plate, appearance.veil) : "plate";
  const naked = mode !== "plate";
  /**
   * 裸字模式下的字色。shade 是纸片模式沿用的原色阶, tier 是它在层级里的档位。
   *
   * 两种裸字档的余量天差地别, 不能套同一套规则:
   * 满墨字压在浅色图上, 准入线只保证满强度达到 4.5:1, 所以这一档只能全满墨,
   * 层级交给字号与字重; 纸色字压在压暗后的深色图上则宽裕得多, 三级都过得了 4.5,
   * 层级该还回来就还回来, 全篇一个白只会读成一张扁平清单。
   */
  const fg = (shade: string, tier: "mid" | "weak") =>
    mode === "ink"
      ? "text-ink"
      : mode === "paperOn"
        ? tier === "mid"
          ? "text-paper-on/85"
          : "text-paper-on/70"
        : shade;
  const [account, setAccount] = useState<AccountDto | null>(null);
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  // config 里那一个实例的 id；入场随 load 拉取，装完游戏时由本页写回（见 load 与 installGame）。
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  // 当前版本的 Mod 数量（仅装了加载器时有意义）；随当前版本变化重取。
  const [modCount, setModCount] = useState<number | null>(null);
  // 装游戏这条链路的状态机。与启动链路完全分开：一个是把游戏搞下来，一个是把它跑起来。
  const [installState, setInstallState] = useState<ModpackInstallState>({ kind: "idle" });
  // 面板里此刻填着的整合包地址。右下角那颗「安装游戏」要装的是它，而不是内置那条——
  // 玩家改了地址却从角上点安装、结果装回官方包，是最难查的那类不一致。
  const [pointerUrl, setPointerUrl] = useState(
    installIntent.pointerUrl ?? BUILT_IN_MODPACK.pointer_url,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 启动链路状态：launching=命令在途，running=进程已起。
  const [launching, setLaunching] = useState(false);
  const [running, setRunning] = useState(false);
  // 游戏日志后台累积（不在 UI 显示，留作诊断 / 未来日志页）。
  const logRef = useRef<GameLog[]>([]);
  // 崩溃报告：由后端在进程异常退出后推送。玩家主动点结束不会触发（detect_crash 对主动终止短路）。
  const [crash, setCrash] = useState<{
    report: CrashReport;
    versionId: string;
    managedStatus: ManagedModpackStatus | null | undefined;
    managedFiles: ManagedModpackFile[] | null | undefined;
  } | null>(null);
  // 进程运行期间持续存活的事件订阅，仅在结束游戏 / 组件卸载时统一 unlisten。
  const runUnlisten = useRef<Array<() => void>>([]);

  const dropRunListeners = useCallback(() => {
    runUnlisten.current.forEach((fn) => fn());
    runUnlisten.current = [];
  }, []);

  useEffect(() => () => dropRunListeners(), [dropRunListeners]);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [acc, sc, cfg] = await Promise.all([currentAccount(), listInstalled(), getConfig()]);
      setAccount(acc);
      setScan(sc);

      // 单实例模型：config.selected_version 是那一个实例的 id，卷宗页与下载页都认它。
      // 版本页撤掉之后没有任何界面再写它了，于是这里补上自愈——扫到了实例、而 config 里指着空
      // （或指着一个已经不存在的 id），就把它对齐回扫描结果，否则玩家明明装好了游戏，
      // 进「管理」却看到空态。写失败只是没能记住选择，扫描结果照用，所以错误上报但不阻断这次加载。
      const resolved = sc.versions.find((v) => v.id === cfg.selected_version) ?? sc.versions[0] ?? null;
      setSelectedVersion(resolved?.id ?? null);
      if (resolved && resolved.id !== cfg.selected_version) {
        await updateConfig({ selectedVersion: resolved.id });
        // 自愈刚把配置从「没选中 / 指着不存在的 id」改成一个真实实例，侧栏据此才能长出「管理」入口。
        notifyInstanceChanged();
      }
    } catch (e) {
      // 错误自然冒泡到这里统一展示，不吞。
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 创建离线账户。
  const handleCreateOffline = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const created = await createOfflineAccount("Steve");
      setAccount(created);
      toast(`已创建离线账户 ${created.name}`, "success");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }, [toast]);

  // 装游戏：读整合包清单 -> 装 Minecraft -> 装加载器 -> 同步文件，四步都在 install_managed_modpack 里。
  const installGame = useCallback(
    async (url: string) => {
      setInstallState({
        kind: "running",
        pointer_url: url,
        target_version: "latest",
        progress: INITIAL_MODPACK_INSTALL_PROGRESS,
      });

      let outcome;
      try {
        outcome = await installManagedModpack(url, (progress) => {
          setInstallState({
            kind: "running",
            pointer_url: url,
            target_version: "latest",
            progress,
          });
        });
      } catch (e) {
        // 后端给的结构化同步失败带着阶段与现场，能落到面板里逐条说清；解不出来才退回通用文案。
        const structured = parseModpackSyncError(e);
        setInstallState(
          structured
            ? {
                kind: "failed",
                pointer_url: url,
                target_version: structured.target_version,
                problem: { kind: "sync", stage: structured.stage, failure: structured.failure },
              }
            : {
                kind: "failed",
                pointer_url: url,
                target_version: null,
                problem: {
                  kind: "setup",
                  failure: {
                    stage: "resolving_manifest",
                    title: "无法开始安装",
                    detail: String(e),
                    action: "确认能连上服务器后重试；若仍失败，请把这段错误详情发给服务器管理员。",
                  },
                },
              },
        );
        return;
      }

      setInstallState({
        kind: "complete",
        pointer_url: url,
        instance_id: outcome.instance_id,
        installed_version: outcome.installed_version,
      });

      // 刚装出来的实例就是这台启动器认的那一个，id 记回 config。这是全前端唯一的写入点，
      // 卷宗页（/instance）与下载页的安装目标都从这里取。
      try {
        await updateConfig({ selectedVersion: outcome.instance_id });
      } catch (e) {
        // 游戏本身已经装好了，配置没写上是另一回事，不能把它渲染成安装失败。
        setError(String(e));
      }
      await load();
      // 装完不跳转（玩家原地就能点 Start），所以侧栏收不到任何路由信号，只能由这里告诉它实例已就位。
      notifyInstanceChanged();
      toast(`World of Kivotos 已安装完成（整合包 ${outcome.installed_version}）`, "success");
    },
    [load, toast],
  );

  // 右下角那颗「安装游戏」：面板里填的是什么就装什么，地址不合法时原样把理由说出来。
  const handleInstallFromControl = useCallback(() => {
    const invalid = validateModpackPointerUrl(pointerUrl);
    if (invalid !== null) {
      toast(invalid, "error");
      return;
    }
    void installGame(pointerUrl.trim());
  }, [pointerUrl, installGame, toast]);

  const versions = scan?.versions ?? [];
  // 当前实例：优先 config 选中项（若仍已安装），否则回落扫描首项——单实例模型下两者本该是同一个，
  // 回落只是为了在 config 尚未写回的那一瞬间不至于把已装好的游戏判成没装。
  const current = versions.find((v) => v.id === selectedVersion) ?? versions[0] ?? null;
  const canLaunch = !loading && !!account && !!current;
  const currentId = current?.id ?? null;
  const currentHasLoader = !!current && current.loaders.length > 0;

  // 当前版本 Mod 数量：仅装了加载器时取，版本切换即重取；失败静默降级（辅助展示，不阻断主流程）。
  useEffect(() => {
    if (!currentId || !currentHasLoader) {
      setModCount(null);
      return;
    }
    let cancelled = false;
    void listMods(currentId)
      .then((mods) => {
        if (!cancelled) setModCount(mods.length);
      })
      .catch(() => {
        if (!cancelled) setModCount(null);
      });
    return () => {
      cancelled = true;
    };
  }, [currentId, currentHasLoader]);

  const handlePlay = useCallback(async () => {
    if (!account || !current) return;
    const versionId = current.id;
    setLaunching(true);
    setError(null);
    logRef.current = [];
    // 上一次的崩溃横条属于上一次会话，重新启动就该收起来。
    setCrash(null);

    // 先订阅日志与进度事件，再 invoke，避免漏掉启动早期的输出。日志只后台累积，告警仍冒泡到 toast。
    const unGame = await onGameLog((line) => logRef.current.push(line));
    const unCore = await onCoreEvent((ev) => {
      if (ev.kind === "warning") toast(`告警：${ev.message}`, "error");
    });
    // 崩溃推送到达即意味着进程已退出：顺手收束运行态，让 Start 从「运行中」复位。
    const unCrash = await onGameCrash((report) => {
      setCrash({ report, versionId, managedStatus: undefined, managedFiles: undefined });
      setRunning(false);
      void Promise.all([
        managedModpackStatus(versionId),
        managedModpackFiles(versionId),
      ])
        .then(([managedStatus, managedFiles]) => {
          setCrash((currentCrash) =>
            currentCrash?.report === report && currentCrash.versionId === versionId
              ? { ...currentCrash, managedStatus, managedFiles }
              : currentCrash,
          );
        })
        .catch((e) => {
          toast(`无法确认崩溃 Mod 的整合包归属：${String(e)}`, "error");
        });
    });
    runUnlisten.current = [unGame, unCore, unCrash];

    // 微软/外置登录用 accountUuid 走服务器校验；离线账户只有本地名，用 offlineName。
    const args: LaunchArgs =
      account.account_type === "offline"
        ? { versionId, offlineName: account.name }
        : { versionId, accountUuid: account.uuid };

    try {
      await launchGame(args);
      setRunning(true);
    } catch (e) {
      // 进程未起：撤销订阅，错误冒泡到错误块与 toast，不吞。
      dropRunListeners();
      setError(String(e));
      toast(String(e), "error");
    } finally {
      setLaunching(false);
    }
  }, [account, current, toast, dropRunListeners]);

  const handleStop = useCallback(async () => {
    try {
      await stopGame();
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    } finally {
      // 无论 stop 成败都收束运行态与订阅：进程若已退出，命令报错也不该留下悬挂监听。
      dropRunListeners();
      setRunning(false);
    }
  }, [toast, dropRunListeners]);

  // 版本副行：MC 版本(若与实例名不同) · 加载器/原版 · Mod 数量，按需拼接；与主行(实例名)字号字重分层。
  const versionMeta = current
    ? [
        current.mc_version !== current.id ? current.mc_version : null,
        current.loaders.length > 0 ? loaderText(current) : "原版",
        current.loaders.length > 0 && modCount !== null ? `${modCount} 个 Mod` : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";

  // 启动控件视觉阶段：命令在途=launching(写字爬升)，进程已起=spawned(补满并切 Stop)。
  const launchPhase: LaunchPhase = launching ? "launching" : running ? "spawned" : "idle";

  const installing = installState.kind === "running";
  // 引导安装的两段可见期：游戏还没装上，以及流程已经开跑（含装完之后——面板要把结果与
  // 「进入管理」交代完，而不是在成功那一刻凭空消失，让玩家以为界面又出了故障）。
  // 只在流程仍是 idle 时才受 loading 压制：首帧扫描结果没回来就先别喊「你没装游戏」，
  // 但装完之后那次重新加载不能把已经跑完的面板闪掉。
  // 第三段可见期是深链：从卷宗页的「安装新版本」过来时游戏通常已经装着（current 非空），
  // 不认这条信号的话面板根本不会出现，那颗按钮就是死的。
  const showInstallFlow =
    installState.kind !== "idle" || installIntent.requested || (!loading && !current);

  const status = loading
    ? "读取中"
    : running
      ? "运行中"
      : installing
        ? "安装中"
        : !current
          ? "未安装"
          : canLaunch
            ? "准备就绪"
            : "未就绪";

  return (
    <>
      {/* 这一页刻意没有报头。原先那个「主页」标题在侧栏改成游戏行之后已经没有对应的导航项，
          副标题「以选中的账户与版本启动游戏」重复了下面那块面板正在展示的三件事，
          留着只是占掉启动屏最值钱的整版留白。唯一有信息量的状态字并进面板首行，
          于是「有图画一套、无图画另一套」的两条渲染路径也一并收成一条。 */}

      {/* 错误块：告警语态由图标与朱红字承担。容器材质走 Card（.surface-panel），页面不再自铺磨砂。
          原来挂在这里的 border-danger/40 已删——它本来就被 Card 基类的描边压掉、从未生效，
          材质化之后描边改走 inset box-shadow，更没有它的位置。危险语态的容器变体要治，
          得给 Card 加 tone，那是 Card 自己的事。 */}
      {error && (
        <Card variants={pageItem} className="mb-6 flex items-center gap-4">
          <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
            <AlertIcon />
          </span>
          <span className="flex-1 text-[13px] text-danger">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void load()}>
            重试
          </Button>
        </Card>
      )}

      {/* 崩溃横条：被动触发的止损入口，不常驻也不打断启动流程。完整诊断在实例卷宗页。 */}
      {/* 玩家点「关闭」是止损动作的收尾，一条报警横条瞬间蒸发更像界面又出了故障，而不是「这次点击生效了」；
          AnimatePresence 留出退场时间，让它淡着上滑走掉。
          投影已焊进横条自己的材质，这层壳只管外边距与退场，不再补 paper-on-photo——
          两处都画影子会叠成一圈重影。 */}
      <AnimatePresence>
        {crash && (
          <motion.div
            variants={pageItem}
            exit={{ opacity: 0, y: -8 }}
            transition={springs.settle}
            className="mb-6"
          >
            <CrashBanner
              onPhoto={onPhoto}
              report={crash.report}
              versionId={crash.versionId}
              ownerOf={(fileName) =>
                modpackOwnerOf(crash.managedStatus, crash.managedFiles, fileName)
              }
              onDismiss={() => setCrash(null)}
              onOpenDetail={() => navigate("/instance")}
            />
          </motion.div>
        )}
      </AnimatePresence>

      {/* 启动屏: 右下角竖排 状态 -> 版本信息 -> 账户, 再往下是放大的 Start, 上方大留白。
       *
       * 这撮信息按三态渲染: 无图坐纸底 / 有图且撑得住就裸字压图 / 图撑不住才退回磨砂纸片。
       * 判定见 appearance.ts 的 plateMode, 数据来自 Rust 侧对右下角这块区域的 p10/p90 取样。
       * 恒定铺一块材质试过, 被否掉: 纸压在照片上永远是在图里挖了一块出来, 启动屏整版留给图
       * 才是这一屏的版面语言, 可读性该由字色去适应图, 而不是拿一块底把图盖住。
       */}
      <motion.section
        variants={pageItem}
        aria-label="启动"
        className="flex min-h-0 flex-1 flex-col items-end gap-5"
      >
        {/* 引导安装：整版留白靠左上让给它，右下角那撮字与主操作位照旧，两者不抢位置。
            这块面板自带 .surface-panel，是启动屏上唯一允许铺材质的东西——
            它承载的是一段要读要填的流程，不是那撮扫一眼就走的状态字。 */}
        {showInstallFlow && (
          <div className="w-full max-w-[560px] self-start">
            <ModpackInstallFlow
              builtIn={BUILT_IN_MODPACK}
              initialPointerUrl={installIntent.pointerUrl ?? undefined}
              state={installState}
              onInstall={(url) => void installGame(url)}
              onPointerUrlChange={setPointerUrl}
              onOpenInstance={() => navigate("/instance")}
            />
          </div>
        )}

        <div
          className={
            !onPhoto
              ? PLATE_BARE
              : naked
                ? // 基色定在容器上而不是逐个节点写。版本名与账户名本来就没写颜色类、
                  // 靠继承 body 的墨色, 压在深色图上直接看不见 —— 逐处去补是治标,
                  // 往后谁再加一行不写颜色的文字就会重犯。定在容器上, 新增节点自动就是对的。
                  `${PLATE_NAKED} ${mode === "paperOn" ? "text-paper-on" : "text-ink"}`
                : PLATE_FROSTED
          }
        >
          <div className="flex items-baseline gap-2.5 self-end">
            <span className={`text-[10px] font-bold tracking-[0.22em] ${fg("text-ink/75", "weak")}`}>状态</span>
            <span className={`font-mono text-[12px] tracking-[0.08em] ${fg("text-ink/75", "mid")} tabular-nums`}>
              {status}
            </span>
          </div>

          {/* 版本信息：实例名(主，粗大) + MC版本 · 加载器 · Mod数(次，细小) */}
          {current ? (
            <div className="max-w-[460px] text-right">
              <div className="truncate text-[19px] leading-tight font-extrabold tracking-[-0.01em]">
                {current.id}
              </div>
              {versionMeta && (
                <div className={`mt-1 truncate font-mono text-[12px] tracking-[0.02em] ${fg("text-ink/75", "mid")}`}>
                  {versionMeta}
                </div>
              )}
            </div>
          ) : (
            // 没有实例时这一格照样给游戏名：这台启动器只服务一个游戏，空着反而像是坏了。
            <div className="max-w-[460px] text-right">
              <div className="truncate text-[19px] leading-tight font-extrabold tracking-[-0.01em]">
                World of Kivotos
              </div>
              <div
                className={`mt-1 truncate font-mono text-[12px] tracking-[0.02em] ${fg("text-ink/75", "mid")}`}
              >
                {loading ? "读取中…" : installing ? "正在安装…" : "尚未安装"}
              </div>
            </div>
          )}

          {/* 账户：头像 + 名字 / 类型 */}
          {account ? (
            <div className="flex items-center gap-3">
              <SkinHead uuid={account.uuid} name={account.name} size={44} />
              <div className="min-w-0 text-right">
                <div className="truncate text-[16px] leading-tight font-extrabold">
                  {account.name}
                </div>
                <div className={`mt-0.5 text-[11px] tracking-[0.1em] ${fg("text-ink/75", "weak")}`}>
                  {ACCOUNT_TYPE_LABEL[account.account_type]}
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-col items-end gap-2">
              <span className={`text-[13px] ${fg("text-ink/75", "weak")}`}>
                {loading ? "正在读取账户…" : "还没有账户"}
              </span>
              {!loading && (
                <Button
                  variant="secondary"
                  onClick={() => void handleCreateOffline()}
                  disabled={busy}
                >
                  创建离线账户
                </Button>
              )}
            </div>
          )}
        </div>

        {/* 主操作位按有没有游戏换语义：装好了是启动，没装是安装。
            读取期间先按启动态渲染（禁用），免得首帧闪一下「安装游戏」又跳回 Start。
            启动态：手写体 Aurora 启动动效（竖线扫左 → 按真实进度写字 → 进程起+2s → Stop）。日志后台存。 */}
        {!loading && !current ? (
          <InstallControl
            onDark={mode === "paperOn"}
            busy={installing}
            onInstall={handleInstallFromControl}
          />
        ) : (
          <LaunchControl
            onDark={mode === "paperOn"}
            phase={launchPhase}
            disabled={!canLaunch}
            onStart={() => void handlePlay()}
            onStop={() => void handleStop()}
          />
        )}
      </motion.section>
    </>
  );
}
