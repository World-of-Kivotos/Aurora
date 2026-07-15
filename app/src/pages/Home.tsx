// 主页（设计标杆）：纯白，PageHeader -> 启动区 -> 账户/版本两栏。
// 真调 IPC：入场并行 invoke current_account + list_installed，渲染真实后端数据；加载/错误态显式处理。
// 离线账户创建顺带示范“进度事件流”范式：订阅 onCoreEvent，把门面发来的告警/阶段写进状态行。

import { useCallback, useEffect, useState } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import {
  AlertIcon,
  LayersIcon,
  PlayIcon,
  RefreshIcon,
  UserIcon,
} from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  createOfflineAccount,
  currentAccount,
  listInstalled,
  onCoreEvent,
  type AccountDto,
  type AccountType,
  type VersionScanDto,
} from "../lib/ipc";
import styles from "./Home.module.css";

const ACCOUNT_TYPE_LABEL: Record<AccountType, string> = {
  microsoft: "微软正版",
  offline: "离线账户",
  authlib_injector: "外置登录",
};

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
  // 传非 ASCII 名会触发门面的 Warning 事件（用户名含非标准字符），可借此看到告警流真跑通。
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
  const canLaunch = !loading && !!account && versions.length > 0;

  const handlePlay = useCallback(() => {
    // 启动链路（launch 命令）由后续 agent 接入；此处如实反馈就绪状态，不伪造成功。
    setStatus(
      canLaunch
        ? `就绪：可启动 ${versions[0].id}（launch 命令将由后续 agent 接入）`
        : "请先创建账户并安装至少一个版本",
    );
  }, [canLaunch, versions]);

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title="主页" subtitle="启动你的 Minecraft 世界" />
      </motion.div>

      {error && (
        <Card variants={pageItem} className={styles.errorCard}>
          <span className={styles.errorIcon}>
            <AlertIcon />
          </span>
          <span className={styles.errorText}>{error}</span>
          <Button variant="secondary" icon={<RefreshIcon />} onClick={() => void load()}>
            重试
          </Button>
        </Card>
      )}

      {/* 启动区：全页唯一主 CTA */}
      <Card variants={pageItem} className={styles.launch}>
        <div className={styles.launchInfo}>
          <span className={styles.launchLabel}>准备就绪</span>
          <span className={styles.launchHint}>
            {loading
              ? "正在读取本地数据…"
              : account
                ? `当前账户 ${account.name}`
                : "尚未选择账户"}
          </span>
        </div>
        <Button variant="primary" icon={<PlayIcon />} onClick={handlePlay} disabled={loading}>
          开始游戏
        </Button>
      </Card>

      {status && <p className={styles.status}>{status}</p>}

      <div className={styles.grid}>
        {/* 当前账户 */}
        <Card variants={pageItem}>
          <h2 className={styles.cardHeading}>当前账户</h2>
          {account ? (
            <div className={styles.accountRow}>
              <span className={styles.avatar}>
                <UserIcon />
              </span>
              <div className={styles.accountMeta}>
                <span className={styles.accountName}>{account.name}</span>
                <span className={styles.accountType}>
                  {ACCOUNT_TYPE_LABEL[account.account_type]}
                </span>
              </div>
            </div>
          ) : (
            <EmptyState
              icon={<UserIcon />}
              title={loading ? "正在读取账户…" : "还没有账户"}
              action={
                loading
                  ? undefined
                  : { label: "创建离线账户", onClick: () => void handleCreateOffline(), disabled: busy }
              }
            />
          )}
        </Card>

        {/* 已安装版本 */}
        <Card variants={pageItem}>
          <h2 className={styles.cardHeading}>已安装版本</h2>
          {versions.length > 0 ? (
            <ul className={styles.versionList}>
              {versions.map((v) => (
                <li key={v.id} className={styles.versionRow}>
                  <span className={styles.versionId}>{v.id}</span>
                  <span className={styles.tags}>
                    <span className={styles.tag}>{v.is_release ? "正式版" : "非正式版"}</span>
                    {v.loaders.map((l) => (
                      <span key={l.kind} className={styles.tag}>
                        {l.kind}
                        {l.version ? ` ${l.version}` : ""}
                      </span>
                    ))}
                  </span>
                </li>
              ))}
              {broken.map((b) => (
                <li key={b.id} className={styles.versionRow}>
                  <span className={styles.versionId}>{b.id}</span>
                  <span className={`${styles.tag} ${styles.tagBroken}`}>{b.reason}</span>
                </li>
              ))}
            </ul>
          ) : (
            <EmptyState
              icon={<LayersIcon />}
              title={loading ? "正在扫描版本…" : "还没有安装任何版本"}
            />
          )}
        </Card>
      </div>
    </>
  );
}
