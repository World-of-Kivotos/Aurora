// 主页：极简启动屏 —— 报头 + 巨号当前版本号（左上）+ 皮肤头像账户 chip 与开始游戏（右下）。
// 已安装版本列表移交「版本」页；主页只聚焦"以谁的身份、启动哪个版本"这一个动作。
// 真调 IPC：入场并行 current_account + list_installed；启动走 launch_game + 日志窗；错误显式冒泡不吞。

import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { LaunchControl, type LaunchPhase } from "../components/LaunchControl";
import { SkinHead } from "../components/SkinHead";
import { useToast } from "../components/Toast";
import { AlertIcon, RefreshIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  createOfflineAccount,
  currentAccount,
  getConfig,
  launchGame,
  listInstalled,
  listMods,
  onCoreEvent,
  onGameLog,
  stopGame,
  type AccountDto,
  type AccountType,
  type GameLog,
  type InstalledVersionDto,
  type LaunchArgs,
  type VersionScanDto,
} from "../lib/ipc";

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

export function Home() {
  const { toast } = useToast();
  const [account, setAccount] = useState<AccountDto | null>(null);
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  // config 里选中的启动版本 id；入场随 load 拉取，决定「开始游戏」启动哪个（版本页设定）。
  const [selectedVersion, setSelectedVersion] = useState<string | null>(null);
  // 当前版本的 Mod 数量（仅装了加载器时有意义）；随当前版本变化重取。
  const [modCount, setModCount] = useState<number | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // 启动链路状态：launching=命令在途，running=进程已起。
  const [launching, setLaunching] = useState(false);
  const [running, setRunning] = useState(false);
  // 游戏日志后台累积（不在 UI 显示，留作诊断 / 未来日志页）。
  const logRef = useRef<GameLog[]>([]);
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
      setSelectedVersion(cfg.selected_version);
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

  const versions = scan?.versions ?? [];
  // 当前启动版本：优先 config 选中项（若仍已安装），否则回落扫描首项；版本页用同一套解析。
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

    // 先订阅日志与进度事件，再 invoke，避免漏掉启动早期的输出。日志只后台累积，告警仍冒泡到 toast。
    const unGame = await onGameLog((line) => logRef.current.push(line));
    const unCore = await onCoreEvent((ev) => {
      if (ev.kind === "warning") toast(`告警：${ev.message}`, "error");
    });
    runUnlisten.current = [unGame, unCore];

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

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader
          title="主页"
          subtitle="以选中的账户与版本启动游戏"
          right={
            <>
              <div className="text-[10px] font-bold tracking-[0.22em] text-ink/40">状态</div>
              <div className="mt-1.5 font-mono text-[12px] tracking-[0.08em] text-ink/60 tabular-nums">
                {loading ? "读取中" : running ? "运行中" : canLaunch ? "准备就绪" : "未就绪"}
              </div>
            </>
          }
        />
      </motion.div>

      {error && (
        <Card variants={pageItem} className="mb-6 flex items-center gap-4 border-danger/40">
          <span className="text-danger [&_svg]:h-5 [&_svg]:w-5">
            <AlertIcon />
          </span>
          <span className="flex-1 text-[13px] text-danger">{error}</span>
          <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void load()}>
            重试
          </Button>
        </Card>
      )}

      {/* 启动屏：右下角竖排 版本信息 → 账户 → 放大 Start，上方大留白 */}
      <motion.section variants={pageItem} aria-label="启动" className="flex min-h-0 flex-1 flex-col">
        <div className="mt-auto flex flex-col items-end gap-6 pt-10">
          {/* 版本信息：实例名(主，粗大) + MC版本 · 加载器 · Mod数(次，细小) */}
          {current ? (
            <div className="max-w-[460px] text-right">
              <div className="truncate text-[19px] leading-tight font-extrabold tracking-[-0.01em]">
                {current.id}
              </div>
              {versionMeta && (
                <div className="mt-1 truncate font-mono text-[12px] tracking-[0.02em] text-ink/50">
                  {versionMeta}
                </div>
              )}
            </div>
          ) : (
            <div className="text-right text-[15px] font-bold text-ink/30">
              {loading ? "读取中…" : "尚未安装版本"}
            </div>
          )}

          {/* 账户：头像 + 名字 / 类型 */}
          {account ? (
            <div className="flex items-center gap-3">
              <SkinHead uuid={account.uuid} name={account.name} size={44} />
              <div className="min-w-0 text-right">
                <div className="truncate text-[16px] leading-tight font-extrabold">{account.name}</div>
                <div className="mt-0.5 text-[11px] tracking-[0.1em] text-ink/45">
                  {ACCOUNT_TYPE_LABEL[account.account_type]}
                </div>
              </div>
            </div>
          ) : (
            <div className="flex flex-col items-end gap-2">
              <span className="text-[13px] text-ink/45">
                {loading ? "正在读取账户…" : "还没有账户"}
              </span>
              {!loading && (
                <Button variant="secondary" onClick={() => void handleCreateOffline()} disabled={busy}>
                  创建离线账户
                </Button>
              )}
            </div>
          )}

          {/* 主操作：手写体 Aurora 启动动效（竖线扫左 → 按真实进度写字 → 进程起+2s → Stop）。日志后台存。 */}
          <LaunchControl
            phase={launchPhase}
            disabled={!canLaunch}
            onStart={() => void handlePlay()}
            onStop={() => void handleStop()}
          />
        </div>
      </motion.section>
    </>
  );
}
