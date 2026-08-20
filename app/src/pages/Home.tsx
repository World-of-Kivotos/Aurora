// 主页：极简启动屏 —— 右下角一块信息面板，竖排 状态 / 当前版本 / 当前账户 / 启动键，上方整版留白。
// Aurora 收敛成 World of Kivotos 专用启动器之后，这一屏还兼了「把游戏装上」：
// 实例的唯一产生途径是安装 WOK 受管整合包，那套流程从下载页搬到了这里（下载页只剩玩家自己装 Mod）。
// 真调 IPC：入场并行 current_account + list_installed + get_config；启动走 launch_game + 日志窗；错误显式冒泡不吞。
//
// 「把游戏装上」在这一屏没有自己的面板：整版就是一张图，装机全程只借用主操作键那一颗控件
// （Download -> 进度条 -> Retry / Start），四步的说明并进进度条左下角那行。地址输入不在这里，
// 它是配置（见 pointerUrl 那段）。唯一的例外是失败：出了事得能读到原因，故失败时在主操作位上方
// 补一张卡片，那也是这一屏上唯一铺材质的东西。

import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { CrashBanner } from "../components/CrashBanner";
import {
  LaunchControl,
  type LaunchInstallProgress,
  type LaunchInstallState,
  type LaunchPhase,
} from "../components/LaunchControl";
import { ModpackSyncFailureView } from "../components/ManagedModpackPanel";
import {
  ModpackSetupFailureView,
  type ModpackInstallState,
} from "../components/ModpackInstallFlow";
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
  formatModpackBytes,
  parseModpackSyncError,
  syncProgressRatio,
  SYNC_STAGE_LABEL,
  type CheckedManagedModpackStatus,
  type ManagedModpackStatus,
  type ModpackSyncProgress,
  type ModpackSyncStage,
} from "../lib/modpack-ui";
import { modpackOwnerOf } from "../lib/modpack-ownership";
import { managedModpackInstallIntent } from "../lib/modpack-navigation";
import { notifyInstanceChanged } from "../lib/instance-signal";

// WOK 受管整合包的内置地址：这台启动器唯一的装游戏途径
// （install_managed_modpack 会连 MC 本体与加载器一并装好）。
// 只有测试服才需要换成别的地址，那属于配置，不在这一屏上给入口。
const BUILT_IN_POINTER_URL = "https://api.mcwok.cn/api/v1/pack/latest";

// 首帧进度：真进度事件到达前先把步骤条钉在第一步，免得点下去到第一个事件之间是一段没有反馈的空白。
const INITIAL_MODPACK_INSTALL_PROGRESS = {
  stage: "resolving_manifest",
  completed_files: 0,
  total_files: 0,
  downloaded_bytes: 0,
  total_bytes: null,
  current_file: null,
} as const;

/**
 * 装游戏那四步在总进度里各占多长（起点, 终点）。
 *
 * 不均分，也不是随手写的：跨度按「这一步通常要跑多久」定，否则百分比会在快步骤上狂跳、
 * 在慢步骤上装死。读取清单只有两次 HTTP 往返，装 Minecraft 要拉三千多个资源文件是全程最久的一段，
 * 装加载器只有几十个库加一次安装器执行，整合包同步是字节量最大、也是唯一全程有真实计数的一段。
 * 后三个整合包阶段（下载/清理/写快照）共用第四步那一段，删文件与写快照本身几乎不耗时，
 * 只各留一点，让「快装完了」这件事在条上看得见。
 *
 * 顺序与后端 install_managed_modpack 的推进顺序一致，所以总进度按步骤单调不回退。
 */
const STAGE_SPAN: Record<ModpackSyncStage, readonly [number, number]> = {
  resolving_manifest: [0, 0.04],
  installing_minecraft: [0.04, 0.46],
  installing_loader: [0.46, 0.62],
  downloading_files: [0.62, 0.96],
  deleting_files: [0.96, 0.98],
  writing_snapshot: [0.98, 1],
};

/**
 * 安装期两条辅助事件流的最新一帧。
 *
 * 装游戏的现场分散在三种事件里：modpack_sync 只有第四步带文件/字节计数，
 * 前三步的细粒度进度归下载器的 download 事件，而每一步在干什么由 stage 事件用一句中文说出来。
 * 主操作键原先只订了 modpack_sync，于是前三步既没有百分比也没有说明，只剩一条来回跑的加载条。
 *
 * stage 字段是这一帧的归属步骤：换步即整帧作废，上一步的文案与文件计数绝不许串到下一步上。
 *
 * download 事件按「批次」计数：装原版是 版本 JSON / 本体与库 / 资源对象 三批依次下，
 * 换批即从 0/total 重新起算，故步内百分比会在批次边界回落一次。那是真实的边界（前两批合计只有
 * 几十个文件、几秒就过，长的是第三批），不在这里做平滑掩饰——步与步之间仍是单调不回退的。
 */
export interface InstallFeed {
  stage: ModpackSyncStage;
  message: string | null;
  download: { total: number; finished: number; bytes: number; speed: number } | null;
}

/** 当前步骤自己的完成比例；这一步拿不到任何计数时为 null（此时总进度停在本步起点）。 */
function stageRatio(progress: ModpackSyncProgress, feed: InstallFeed): number | null {
  if ((progress.total_bytes !== null && progress.total_bytes > 0) || progress.total_files > 0) {
    return syncProgressRatio(progress);
  }
  const download = feed.stage === progress.stage ? feed.download : null;
  if (download !== null && download.total > 0) {
    return Math.min(1, download.finished / download.total);
  }
  return null;
}

/** 左下角那行的现场补充：谁的数最实就用谁，都没有才退回后端那句中文阶段说明。 */
function stageDetail(progress: ModpackSyncProgress, feed: InstallFeed): string | null {
  if (progress.total_bytes !== null && progress.total_bytes > 0) {
    return `${formatModpackBytes(progress.downloaded_bytes)} / ${formatModpackBytes(progress.total_bytes)}`;
  }
  const fresh = feed.stage === progress.stage ? feed : null;
  const download = fresh?.download ?? null;
  if (download !== null && download.total > 0) {
    const files = `${download.finished}/${download.total} 个文件`;
    return download.speed > 0 ? `${files} · ${formatModpackBytes(download.speed)}/s` : files;
  }
  if (progress.total_files > 0) {
    return `${progress.completed_files}/${progress.total_files} 个文件`;
  }
  return progress.current_file ?? fresh?.message ?? null;
}

/**
 * 把三路事件折算成主操作键要画的那一份进度。步骤名一律复用 SYNC_STAGE_LABEL，不另写一套中文。
 *
 * 导出是为了能单独测：这段折算是「四步都有真百分比」这条要求的全部实现，
 * 而它在组件里只经由一个 IPC 长流程才跑得到，不导出就只能靠肉眼看。
 */
export function installProgressView(
  progress: ModpackSyncProgress,
  feed: InstallFeed,
): LaunchInstallProgress {
  const [base, end] = STAGE_SPAN[progress.stage];
  const ratio = stageRatio(progress, feed);
  const label = SYNC_STAGE_LABEL[progress.stage];
  const head = ratio === null ? label : `${label} ${Math.round(ratio * 100)}%`;
  const detail = stageDetail(progress, feed);
  return {
    overall: base + (end - base) * (ratio ?? 0),
    activity: detail === null ? head : `${head} · ${detail}`,
    counted: ratio !== null,
    stage: progress.stage,
  };
}

// 受管整合包在界面上的名字。拆成「包名 + 通道」两半是留给将来的双通道切换：
// 用户要的「悬停飘出 Beta」这一半现在做不了——服务端 pack_version 表没有 channel 列，
// 且唯一索引钉死「全局同时只能有一个 published 版本」，Beta 通道要先做库迁移才存在，
// 所以这次只落 Master，不造一个点了没反应的假入口。
//
// 将来接 Beta 要动的地方只有两处：
//   1) 通道名改成从 managedStatus.subscription 里读（届时后端会把 channel 一并带出来），
//      本文件不需要再新造 IPC；
//   2) 悬浮切换器挂到下面那个 relative 的实例名容器上（它已经是定位上下文），
//      从右向左飘出，不改右下角这一列的版位。
const MODPACK_NAME = "World of Kivotos";
const MODPACK_CHANNEL = "Master";

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
// 贴底不靠这一撮自己: 上面那只告警滚动盒子是 flex-1, 它吃掉全部留白, 本 section 只按自然高度
// 落在页尾。三种形态都不自带 mt-auto —— 同一根 flex 列里出现两个 auto 上外边距时剩余空间是
// **均分**的, 那会把上面的失败卡片顶到半空、与这撮字断成两截。
const PLATE_BARE = "flex flex-col items-end gap-6 pt-10";
// 裸字: 不铺底, 只靠字色。右对齐与间距沿用纸片那套, 换形态时版位不跳。
const PLATE_NAKED = "ml-auto flex flex-col items-end gap-6 pt-10";
// 兜底纸片: 图明暗跨度大到两种字色都撑不住时才用。走信息密集档, 它最实。
const PLATE_FROSTED =
  "surface-panel-strong ml-auto flex flex-col items-end gap-6 rounded-panel px-7 py-6";

export function Home() {
  const { toast } = useToast();
  const navigate = useNavigate();
  // 卷宗页遇到同步冲突时会把当前订阅地址带过来（managedModpackInstallRoute）。
  // 那条链路的落点就是这一屏：地址直接喂给主操作键，同时把那颗键钉在 Download 上，
  // 否则游戏已经装着的人过来只会看到一颗 Start，深链等于没点。
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
  // 当前实例的受管整合包订阅。null = 没订阅（玩家自己拼的纯净版实例），右下角就照旧显示实例信息。
  const [managedStatus, setManagedStatus] = useState<CheckedManagedModpackStatus | null>(null);
  // 装游戏这条链路的状态机。与启动链路完全分开：一个是把游戏搞下来，一个是把它跑起来。
  const [installState, setInstallState] = useState<ModpackInstallState>({ kind: "idle" });
  // 安装期 stage / download 两条事件流的最新一帧，与 installState 里的 modpack_sync 合起来才凑得齐进度。
  const [installFeed, setInstallFeed] = useState<InstallFeed>({
    stage: INITIAL_MODPACK_INSTALL_PROGRESS.stage,
    message: null,
    download: null,
  });
  // 要装的整合包地址。启动屏不再有地址输入框（那是配置，不是启动屏该管的事），
  // 于是只剩两个来源：卷宗页深链带过来的那条，或内置的官方地址。
  // 两条都已经过 validateModpackPointerUrl —— 深链在 managedModpackPointerFromSearch 里筛过，
  // 内置那条是常量，所以这里不再重复校验。地址输入将来落到设置页时，校验跟着输入框一起走。
  const pointerUrl = installIntent.pointerUrl ?? BUILT_IN_POINTER_URL;
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
      setInstallFeed({
        stage: INITIAL_MODPACK_INSTALL_PROGRESS.stage,
        message: null,
        download: null,
      });

      // installManagedModpack 自带的回调只送 modpack_sync（且按 operation_id 对号入座），
      // 而前三步的现场只走 stage / download 两种事件，得另开一条订阅去接。
      // 两条订阅落在同一个 Tauri 事件通道上，投递顺序即后端发事件的顺序，
      // 所以「换步的 modpack_sync」总是先于那一步的 stage 文案到达，归属判断才成立。
      const unlistenCore = await onCoreEvent((ev) => {
        if (ev.kind === "stage") {
          setInstallFeed((prev) => ({ ...prev, message: ev.message }));
        } else if (ev.kind === "download") {
          setInstallFeed((prev) => ({
            ...prev,
            download: {
              total: ev.total,
              finished: ev.finished,
              bytes: ev.bytes,
              speed: ev.speed,
            },
          }));
        }
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
          setInstallFeed((prev) =>
            prev.stage === progress.stage
              ? prev
              : { stage: progress.stage, message: null, download: null },
          );
        });
      } catch (e) {
        // 后端给的结构化同步失败带着阶段与现场，能落到失败卡片里逐条说清；解不出来才退回通用文案。
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
      } finally {
        // 成败都撤订阅：安装结束后这条通道上还会跑启动、更新等别人的事件，
        // 留着监听只会把别处的阶段文案糊到这一屏上。
        unlistenCore();
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

  // 主操作位那颗 Download / Retry：装的是上面解出来的那条地址。
  const handleInstallFromControl = useCallback(() => {
    void installGame(pointerUrl.trim());
  }, [pointerUrl, installGame]);

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

  // 当前实例订没订受管整合包。装完游戏那一轮 load() 会换掉 currentId，于是这里跟着重取，
  // 名字从实例 id 翻成整合包名不需要额外的信号。
  // 读不到就当没订阅：这一格是「这个实例是什么」的陈述，宁可退回实例自己的 id，
  // 也不能在拿不准的时候把整合包名字挂上去——那是拿一句没核实过的话糊在界面上。
  useEffect(() => {
    if (!currentId) {
      setManagedStatus(null);
      return;
    }
    let cancelled = false;
    void managedModpackStatus(currentId)
      .then((status) => {
        if (!cancelled) setManagedStatus(status);
      })
      .catch(() => {
        if (!cancelled) setManagedStatus(null);
      });
    return () => {
      cancelled = true;
    };
  }, [currentId]);

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

  // 实例名这一格的两个来源：订了受管整合包就报整合包名，没订（玩家自己拼的纯净版实例）仍报实例 id。
  // 不是无条件写死：服务端至今一个整合包版本都没发布过（/pack/latest 实测 404），
  // 无条件写死等于拿一句没核实过的话糊在界面上，玩家点开管理页会看到对不上的东西。
  const subscribed = managedStatus !== null;
  const instanceTitle = subscribed ? `${MODPACK_NAME} - ${MODPACK_CHANNEL}` : (current?.id ?? "");

  // 版本副行：MC 版本 · 加载器/原版 · Mod 数量，按需拼接；与主行(实例名)字号字重分层。
  // 报整合包名时 MC 版本一律带上：实例 id 已经不在屏上了，「与实例名相同就省略」那条前提不成立。
  const versionMeta = current
    ? [
        subscribed || current.mc_version !== current.id ? current.mc_version : null,
        current.loaders.length > 0 ? loaderText(current) : "原版",
        current.loaders.length > 0 && modCount !== null ? `${modCount} 个 Mod` : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";

  // 启动控件视觉阶段：命令在途=launching(写字爬升)，进程已起=spawned(补满并切 Stop)。
  const launchPhase: LaunchPhase = launching ? "launching" : running ? "spawned" : "idle";

  const installing = installState.kind === "running";

  // 主操作位的装机形态。四档优先级，从「此刻最要紧的事」往下排：
  //   1. 安装在途 —— 整颗按钮就是进度条。已经装着游戏时也走这一支：装到一半让人点 Start，
  //      启动的是一个正在被改写的实例。
  //   2. 上一次没装完 —— 换成 Retry。安装卡片撤掉之后这是唯一的重试入口，
  //      而且它不看游戏装没装：安装若在创建实例之后才失败，current 已非空，
  //      此时若交回 Start，玩家能启动的是一个只装了一半的实例。
  //   3. 确认没装、或深链带着地址过来 —— 换成 Download。深链这一条不能省：
  //      卷宗页遇同步冲突时跳过来的人通常已经装着游戏（current 非空），
  //      不认这条信号，那颗「安装新版本」按下去就没有任何后续。
  //   4. 其余 —— 交回启动语义。首帧扫描结果没回来时也走这里（按禁用的 Start 渲染），
  //      免得闪一下 Download 又跳回 Start。
  const installForm: LaunchInstallState | null =
    installState.kind === "running"
      ? { kind: "running", progress: installProgressView(installState.progress, installFeed) }
      : installState.kind === "failed"
        ? { kind: "failed", onRetry: handleInstallFromControl }
        : (!loading && !current) || (installIntent.requested && installState.kind === "idle")
          ? { kind: "absent", onInstall: handleInstallFromControl }
          : null;

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

      {/* 告警堆叠区：错误块 / 崩溃横条 / 安装失败卡片三块共用这一只盒子，它吃掉主操作位以上的
          全部高度，自己在里面滚。
          为什么必须是滚动区而不是任其自然排：这三块的高度全由后端文案定（失败卡片的正文直接来自
          failure.detail，长度没有上限），三者又互不排斥（读取失败 + 上一局崩溃 + 这次装机失败能同时在场）。
          而这一屏是白名单里明令不许整页滚的那一屏——外壳 overflow-clip，溢出不会长滚动条，
          只会把最下面的东西无声裁掉，而最下面的正是这台启动器唯一的开始游戏入口。
          把它们关进一只定高的盒子，主操作位就永远钉在原地，代价只是告警本身要滚一下。 */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto">
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

          {/* 安装失败：卡片没了，失败现场不能跟着一起消失。
              落点选在主操作位正上方而不是页顶：重试入口就是下面那颗 Retry，
              「出了什么事」与「按哪里」隔着一整屏留白，读起来是两件事。
              mt-auto 把它压在告警区的底缘，mb-5 补出与下面那撮状态字之间原来那道 gap-5。
              这是启动屏上唯一还铺材质的东西——它是一段要逐行读的说明，
              与那撮扫一眼就走的状态字不同，裸压在图上没法读。 */}
          {installState.kind === "failed" && (
            <Card variants={pageItem} role="alert" className="mt-auto mb-5 w-full max-w-[520px] self-end">
              <div className="flex items-center gap-2.5 text-[13px] font-extrabold text-danger">
                <AlertIcon size={18} />
                安装没有完成
              </div>
              {installState.problem.kind === "setup" ? (
                <ModpackSetupFailureView failure={installState.problem.failure} />
              ) : (
                <ModpackSyncFailureView failure={installState.problem.failure} />
              )}
              <p className="mt-3.5 text-[12px] leading-relaxed text-ink/75">
                点右下角的 Retry 重新安装。已经下好并校验过的文件不会重复下载。
              </p>
            </Card>
          )}
      </div>

      {/* 启动屏: 右下角竖排 状态 -> 版本信息 -> 账户, 再往下是放大的 Start, 上方大留白。
       *
       * 这撮信息按三态渲染: 无图坐纸底 / 有图且撑得住就裸字压图 / 图撑不住才退回磨砂纸片。
       * 判定见 appearance.ts 的 plateMode, 数据来自 Rust 侧对右下角这块区域的 p10/p90 取样。
       * 恒定铺一块材质试过, 被否掉: 纸压在照片上永远是在图里挖了一块出来, 启动屏整版留给图
       * 才是这一屏的版面语言, 可读性该由字色去适应图, 而不是拿一块底把图盖住。
       *
       * shrink-0: 这一撮加主操作键是本屏唯一不许被压缩、也不许被裁掉的东西，
       * 上方留白与告警都归那只滚动盒子管，它只按自然高度占位。
       */}
      <motion.section
        variants={pageItem}
        aria-label="启动"
        className="flex shrink-0 flex-col items-end gap-5"
      >
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

          {/* 版本信息：实例名(主，粗大) + MC版本 · 加载器 · Mod数(次，细小)。
              主行的文字来自 instanceTitle（整合包名 / 实例 id 两种来源），不是就地取 current.id——
              将来的 Master/Beta 切换器要挂在这个 relative 容器上（悬停从右向左飘出），
              名字与来源分开写，那时只需把 instanceTitle 换成按通道取，这段 JSX 不用重排。 */}
          {current ? (
            <div className="relative max-w-[460px] text-right">
              <div className="truncate text-[19px] leading-tight font-extrabold tracking-[-0.01em]">
                {instanceTitle}
              </div>
              {versionMeta && (
                <div className={`mt-1 truncate font-mono text-[12px] tracking-[0.02em] ${fg("text-ink/75", "mid")}`}>
                  {versionMeta}
                </div>
              )}
            </div>
          ) : (
            // 没有实例时这一格照样给游戏名：这台启动器只服务一个游戏，空着反而像是坏了。
            // 这里只给包名不给通道：一个都没装的时候谈不上装的是哪个通道。
            <div className="relative max-w-[460px] text-right">
              <div className="truncate text-[19px] leading-tight font-extrabold tracking-[-0.01em]">
                {MODPACK_NAME}
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

        {/* 主操作位。一颗控件三种语义，切换由 installForm 决定（见上面那三档优先级）：
            还没装 = Download，正在装 = 进度条，装好了 = Start。
            启动态：手写体 Aurora 启动动效（竖线扫左 → 按真实进度写字 → 进程起+2s → Stop）。日志后台存。 */}
        <LaunchControl
          onDark={mode === "paperOn"}
          phase={launchPhase}
          disabled={!canLaunch}
          install={installForm}
          onStart={() => void handlePlay()}
          onStop={() => void handleStop()}
        />
      </motion.section>
    </>
  );
}
