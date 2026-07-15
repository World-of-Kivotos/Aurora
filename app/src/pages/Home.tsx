// 主页（设计标杆）：编辑部版式 —— 报头 + 非对称启动特写（巨号当前版本号为主角）+ 大字版本清单。
// 真调 IPC：入场并行 invoke current_account + list_installed，渲染真实后端数据；加载/错误态显式处理。
// 离线账户创建顺带示范“进度事件流”范式：订阅 onCoreEvent，把门面发来的告警/阶段写进状态行。

import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { AlertIcon, LayersIcon, PlayIcon, RefreshIcon, UserIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  createOfflineAccount,
  currentAccount,
  listInstalled,
  onCoreEvent,
  type AccountDto,
  type AccountType,
  type InstalledVersionDto,
  type VersionScanDto,
} from "../lib/ipc";

const ACCOUNT_TYPE_LABEL: Record<AccountType, string> = {
  microsoft: "微软正版",
  offline: "离线账户",
  authlib_injector: "外置登录",
};

// 版本 id 拆成主段 + 加载器后缀（"1.20.1-fabric" -> {base:"1.20.1", sfx:"-fabric"}），后缀在版面里以朱红呈现。
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
  const [account, setAccount] = useState<AccountDto | null>(null);
  const [scan, setScan] = useState<VersionScanDto | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<string | null>(null);

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
  const broken = scan?.broken ?? [];
  const total = versions.length + broken.length;
  const current = versions[0] ?? null;
  const canLaunch = !loading && !!account && versions.length > 0;

  const handlePlay = useCallback(() => {
    // 启动链路（launch 命令）由后续 agent 接入；此处如实反馈就绪状态，不伪造成功。
    setStatus(
      canLaunch
        ? `就绪：可启动 ${versions[0].id}（launch 命令将由后续 agent 接入）`
        : "请先创建账户并安装至少一个版本",
    );
  }, [canLaunch, versions]);

  const cur = current ? splitId(current.id) : null;

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
                {loading ? "读取中" : account ? "准备就绪" : "未就绪"}
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

      {/* 启动特写：非对称双栏，巨号版本号为主角 */}
      <motion.section
        variants={pageItem}
        aria-label="启动"
        className="mb-[34px] grid grid-cols-[1.45fr_0.95fr] items-end gap-[52px] max-[1080px]:grid-cols-1 max-[1080px]:gap-8"
      >
        <div className="min-w-0">
          <div className="mb-[14px] flex items-center gap-3 text-[11px] font-bold tracking-[0.2em] text-ink/60">
            <span className="inline-block h-[2px] w-[26px] bg-accent" />
            准备就绪 · 当前版本
          </div>
          {cur ? (
            <>
              <p className="m-0 text-[clamp(46px,6.7vw,86px)] leading-[0.9] font-extrabold tracking-[-0.035em] whitespace-nowrap tabular-nums">
                {cur.base}
                {cur.sfx && <span className="font-bold tracking-[-0.02em] text-accent">{cur.sfx}</span>}
              </p>
              <div className="mt-[18px] flex flex-wrap items-baseline gap-x-4 gap-y-1 text-[12.5px] text-ink/40">
                <span>
                  <span className="font-bold tracking-[0.12em] text-ink/60">加载器</span>{" "}
                  <span className="font-mono tabular-nums">{current ? loaderText(current) : "—"}</span>
                </span>
                {account && (
                  <span>
                    <span className="font-bold tracking-[0.12em] text-ink/60">账户</span> {account.name}
                  </span>
                )}
              </div>
            </>
          ) : (
            <p className="m-0 text-[28px] font-extrabold text-ink/26">
              {loading ? "读取中…" : "尚未安装版本"}
            </p>
          )}
        </div>

        <div className="flex flex-col gap-4">
          {account ? (
            <div className="flex items-center gap-[15px] rounded-[3px] border border-ink/9 bg-paper-sink px-4 py-[14px]">
              <span className="grid h-[52px] w-[52px] shrink-0 place-items-center rounded-[3px] bg-ink text-[22px] font-extrabold text-paper-on">
                {account.name.slice(0, 1).toUpperCase()}
              </span>
              <div className="flex min-w-0 flex-col gap-[5px]">
                <div className="truncate text-[19px] leading-none font-extrabold">{account.name}</div>
                <span className="self-start rounded-[2px] border border-ink/16 px-2 py-[3px] text-[10.5px] font-bold tracking-[0.14em] text-ink/60">
                  {ACCOUNT_TYPE_LABEL[account.account_type]}
                </span>
              </div>
            </div>
          ) : (
            <Card>
              <EmptyState
                icon={<UserIcon />}
                title={loading ? "正在读取账户…" : "还没有账户"}
                action={
                  loading
                    ? undefined
                    : { label: "创建离线账户", onClick: () => void handleCreateOffline(), disabled: busy }
                }
              />
            </Card>
          )}
          <Button
            variant="primary"
            icon={<PlayIcon />}
            onClick={handlePlay}
            disabled={loading}
            className="w-full py-[17px] text-[17px]"
          >
            开始游戏
          </Button>
        </div>
      </motion.section>

      {status && <p className="mb-6 font-mono text-[12px] text-ink/60">{status}</p>}

      {/* 已安装版本：大字非对称清单 */}
      <motion.section variants={pageItem} aria-label="已安装版本" className="mt-auto">
        <div className="mb-0.5 flex items-baseline justify-between border-b border-ink/16 pb-[11px]">
          <h2 className="text-[19px] font-extrabold">已安装版本</h2>
          <span className="font-mono text-[11px] tracking-[0.14em] text-ink/40 tabular-nums">
            共 {String(total).padStart(2, "0")} 项
          </span>
        </div>
        {total > 0 ? (
          <ul className="m-0 list-none p-0">
            {versions.map((v, i) => {
              const s = splitId(v.id);
              return (
                <li
                  key={v.id}
                  className="flex items-baseline justify-between gap-6 border-b border-ink/9 py-[13px] last:border-b-0"
                >
                  <span className="flex items-baseline gap-0.5 text-[24px] font-bold tracking-[-0.01em] tabular-nums">
                    {s.base}
                    {s.sfx && <span className="font-semibold text-ink/40">{s.sfx}</span>}
                    {i === 0 && (
                      <span className="ml-3 self-center text-[9.5px] font-extrabold tracking-[0.18em] text-accent">
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
          <EmptyState
            icon={<LayersIcon />}
            title={loading ? "正在扫描版本…" : "还没有安装任何版本"}
          />
        )}
      </motion.section>
    </>
  );
}
