// 主页：极简启动屏 —— 报头 + 巨号当前版本号（左上）+ 皮肤头像账户 chip 与开始游戏（右下）。
// 已安装版本列表移交「版本」页；主页只聚焦"以谁的身份、启动哪个版本"这一个动作。
// 真调 IPC：入场并行 current_account + list_installed；启动走 launch_game + 日志窗；错误显式冒泡不吞。

import { useCallback, useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { Modal } from "../components/Modal";
import { LogConsole } from "../components/LogConsole";
import { SkinHead } from "../components/SkinHead";
import { useToast } from "../components/Toast";
import { AlertIcon, RefreshIcon } from "../components/icons";
import { pageItem, springs } from "../lib/motion";
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
  const [status, setStatus] = useState<string | null>(null);
  // 启动链路状态：launching=命令在途，running=进程已起。日志行随 onGameLog 累积。
  const [launching, setLaunching] = useState(false);
  const [running, setRunning] = useState(false);
  const [pid, setPid] = useState<number | null>(null);
  const [logLines, setLogLines] = useState<GameLog[]>([]);
  const [logOpen, setLogOpen] = useState(false);
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

  // 创建离线账户：订阅进度事件流后再 invoke，结束务必 unlisten。
  const handleCreateOffline = useCallback(async () => {
    setBusy(true);
    setError(null);
    setStatus(null);
    const unlisten = await onCoreEvent((ev) => {
      if (ev.kind === "warning") setStatus(`告警：${ev.message}`);
      else if (ev.kind === "stage") setStatus(ev.message);
    });
    try {
      const created = await createOfflineAccount("Steve");
      setAccount(created);
      setStatus((prev) => prev ?? `已创建离线账户 ${created.name}`);
    } catch (e) {
      setError(String(e));
    } finally {
      unlisten();
      setBusy(false);
    }
  }, []);

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
    setStatus(null);
    setLogLines([]);
    setLogOpen(true);

    // 先订阅日志与进度事件，再 invoke，避免漏掉启动早期的输出。
    const unGame = await onGameLog((line) => setLogLines((prev) => [...prev, line]));
    const unCore = await onCoreEvent((ev) => {
      if (ev.kind === "warning") {
        setStatus(`告警：${ev.message}`);
        toast(`告警：${ev.message}`, "error");
      } else if (ev.kind === "stage") {
        setStatus(ev.message);
      }
    });
    runUnlisten.current = [unGame, unCore];

    // 微软/外置登录用 accountUuid 走服务器校验；离线账户只有本地名，用 offlineName。
    const args: LaunchArgs =
      account.account_type === "offline"
        ? { versionId, offlineName: account.name }
        : { versionId, accountUuid: account.uuid };

    try {
      const launched = await launchGame(args);
      setPid(launched.pid);
      setRunning(true);
      setStatus(`已启动 ${versionId}`);
      toast(
        launched.pid != null ? `已启动 ${versionId}，PID ${launched.pid}` : `已启动 ${versionId}`,
        "success",
      );
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
      setStatus("已结束游戏");
      toast("已结束游戏", "success");
    } catch (e) {
      setError(String(e));
      toast(String(e), "error");
    } finally {
      // 无论 stop 成败都收束运行态与订阅：进程若已退出，命令报错也不该留下悬挂监听。
      dropRunListeners();
      setRunning(false);
      setPid(null);
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

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader
          title="主页"
          subtitle="Start your Minecraft journey~"
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

          {/* 主操作：放大的「竖线 + 文字」——全屏最大字，绝对主角。无填充方块，hover 点亮朱红、竖条抽高。 */}
          <div className="flex flex-col items-end gap-3">
            <motion.button
              type="button"
              onClick={() => void handlePlay()}
              disabled={!canLaunch || launching || running}
              whileTap={{ scale: 0.99 }}
              transition={springs.tap}
              className="group inline-flex items-center gap-5 py-1 focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-40"
            >
              <span className="text-[clamp(34px,4.4vw,52px)] leading-none font-extrabold tracking-[-0.02em] text-ink transition-colors duration-200 group-hover:text-accent">
                {launching ? "启动中" : running ? "运行中" : "Start"}
              </span>
              <span
                aria-hidden="true"
                className="h-[40px] w-[4px] shrink-0 bg-accent transition-all duration-200 group-hover:h-[58px]"
              />
            </motion.button>
            {(running || logLines.length > 0) && (
              <div className="flex gap-3">
                <Button variant="secondary" onClick={() => setLogOpen(true)}>
                  查看日志
                </Button>
                {running && (
                  <Button variant="secondary" onClick={() => void handleStop()}>
                    结束游戏
                  </Button>
                )}
              </div>
            )}
          </div>

          {status && (
            <p className="max-w-[440px] truncate text-right font-mono text-[12px] text-ink/50">
              {status}
            </p>
          )}
        </div>
      </motion.section>

      <Modal
        open={logOpen}
        onClose={() => setLogOpen(false)}
        title={running && pid != null ? `运行中 · PID ${pid}` : running ? "运行中" : "游戏日志"}
        footer={
          running ? (
            <Button variant="primary" onClick={() => void handleStop()}>
              结束游戏
            </Button>
          ) : (
            <Button variant="secondary" onClick={() => setLogOpen(false)}>
              关闭
            </Button>
          )
        }
      >
        <div className="h-72">
          <LogConsole lines={logLines} />
        </div>
      </Modal>
    </>
  );
}
