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
import { AlertIcon, PlayIcon, RefreshIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  createOfflineAccount,
  currentAccount,
  launchGame,
  listInstalled,
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

// 版本 id 拆成主段 + 加载器后缀（"1.20.1-fabric" -> {base:"1.20.1", sfx:"-fabric"}）。
function splitId(id: string): { base: string; sfx: string } {
  const i = id.indexOf("-");
  return i < 0 ? { base: id, sfx: "" } : { base: id.slice(0, i), sfx: id.slice(i) };
}

function loaderText(v: InstalledVersionDto): string {
  if (v.loaders.length === 0) return "—";
  const l = v.loaders[0];
  return l.version ? `${l.kind} ${l.version}` : l.kind;
}

export function Home() {
  const { toast } = useToast();
  const [account, setAccount] = useState<AccountDto | null>(null);
  const [scan, setScan] = useState<VersionScanDto | null>(null);
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
      const [acc, sc] = await Promise.all([currentAccount(), listInstalled()]);
      setAccount(acc);
      setScan(sc);
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
  const current = versions[0] ?? null;
  const canLaunch = !loading && !!account && versions.length > 0;

  const handlePlay = useCallback(async () => {
    if (!account || versions.length === 0) return;
    const versionId = versions[0].id;
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
  }, [account, versions, toast, dropRunListeners]);

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

  const cur = current ? splitId(current.id) : null;
  // 加载器标签：优先用扫描解析出的加载器（"Forge 47.4.20"），回落到版本 id 后缀（去前导 - 与下划线）。
  const loaderLabel =
    current && current.loaders.length > 0
      ? loaderText(current)
      : cur && cur.sfx
        ? cur.sfx.replace(/^-/, "").replace(/_/g, " ")
        : null;

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader
          title="主页"
          subtitle="启动你的 Minecraft 世界"
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

      {/* 极简启动屏：版本号左上，启动集群右下，中间大留白 */}
      <motion.section variants={pageItem} aria-label="启动" className="flex min-h-0 flex-1 flex-col">
        <div className="min-w-0">
          <div className="mb-[14px] flex items-center gap-3 text-[11px] font-bold tracking-[0.2em] text-ink/60">
            <span className="inline-block h-[2px] w-[26px] bg-accent" />
            {cur ? "准备就绪 · 当前版本" : "当前版本"}
          </div>
          {cur ? (
            <>
              <p className="m-0 text-[clamp(46px,6.4vw,84px)] leading-[0.9] font-extrabold tracking-[-0.035em] tabular-nums break-words">
                {cur.base}
              </p>
              {loaderLabel && (
                <p className="mt-1.5 max-w-full truncate text-[clamp(19px,2.5vw,32px)] font-bold tracking-[-0.01em] text-accent">
                  {loaderLabel}
                </p>
              )}
            </>
          ) : (
            <p className="m-0 text-[32px] font-extrabold text-ink/26">
              {loading ? "读取中…" : "尚未安装版本"}
            </p>
          )}
        </div>

        {/* 启动集群：右下角 —— 皮肤头像账户 chip + 大启动按钮 */}
        <div className="mt-auto flex flex-col items-end gap-5 pt-10">
          {account ? (
            <div className="flex items-center gap-3">
              <SkinHead uuid={account.uuid} name={account.name} size={42} />
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

          <div className="flex flex-col items-end gap-3">
            <Button
              variant="primary"
              icon={<PlayIcon />}
              onClick={() => void handlePlay()}
              disabled={!canLaunch || launching || running}
              className="px-9 py-[18px] text-[18px]"
            >
              {launching ? "启动中…" : running ? "运行中" : "开始游戏"}
            </Button>
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
            <p className="max-w-[420px] truncate text-right font-mono text-[12px] text-ink/50">
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
