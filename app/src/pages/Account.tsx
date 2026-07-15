// 账户页：微软正版 / 离线 / 外置（authlib-injector）三类账户的增删与切换。
// 账户命令多为 Windows 专属：非 Windows 下 listAccounts 返回空、登录命令 reject——如实展示，不崩。
// IPC reject 一个字符串；本页作为最外层展示层用 try/catch → toast 暴露，绝不吞。

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { motion } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PageHeader } from "../components/PageHeader";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { Modal } from "../components/Modal";
import { useToast } from "../components/Toast";
import { UserIcon, RefreshIcon, AlertIcon } from "../components/icons";
import { pageItem } from "../lib/motion";
import {
  listAccounts,
  currentAccount,
  setCurrentAccount,
  removeAccount,
  createOfflineAccount,
  microsoftLogin,
  authlibLogin,
  onDeviceCode,
  type AccountDto,
  type AccountType,
  type DeviceCode,
} from "../lib/ipc";

const TYPE_LABEL: Record<AccountType, string> = {
  microsoft: "微软正版",
  offline: "离线账户",
  authlib_injector: "外置登录",
};

const INPUT_CLS =
  "w-full rounded-[3px] border border-ink/16 bg-paper px-3.5 py-2.5 text-[14px] text-ink outline-none transition-colors placeholder:text-ink/40 hover:border-ink/40 focus:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

// 头像取名字首个码位（Array.from 正确切割 CJK/代理对）；空名兜底问号仅为字形占位，非业务掩盖。
function initialOf(name: string): string {
  return Array.from(name.trim())[0]?.toUpperCase() ?? "?";
}

interface TextFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: "text" | "password";
  placeholder?: string;
}

function TextField({ label, value, onChange, type = "text", placeholder }: TextFieldProps) {
  const id = useId();
  return (
    <label htmlFor={id} className="block">
      <span className="mb-1.5 block text-[12.5px] font-bold text-ink/70">{label}</span>
      <input
        id={id}
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        className={INPUT_CLS}
      />
    </label>
  );
}

export function Account() {
  const { toast } = useToast();

  // null = 尚未加载；[] = 已加载但为空。loadError 与 accounts 互斥呈现。
  const [accounts, setAccounts] = useState<AccountDto[] | null>(null);
  const [currentUuid, setCurrentUuid] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [removeTarget, setRemoveTarget] = useState<AccountDto | null>(null);
  const [removeBusy, setRemoveBusy] = useState(false);

  const [offlineOpen, setOfflineOpen] = useState(false);
  const [offlineName, setOfflineName] = useState("");
  const [offlineBusy, setOfflineBusy] = useState(false);

  const [authOpen, setAuthOpen] = useState(false);
  const [authServer, setAuthServer] = useState("");
  const [authUser, setAuthUser] = useState("");
  const [authPass, setAuthPass] = useState("");
  const [authBusy, setAuthBusy] = useState(false);

  const [msOpen, setMsOpen] = useState(false);
  const [msBusy, setMsBusy] = useState(false);
  const [deviceCode, setDeviceCode] = useState<DeviceCode | null>(null);
  // 微软登录后端不可中断：用自增会话号忽略被取消会话的迟到结果，避免旧会话清掉新会话的监听器。
  const msRunId = useRef(0);
  const msUnlisten = useRef<(() => void) | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const [list, cur] = await Promise.all([listAccounts(), currentAccount()]);
      setAccounts(list);
      setCurrentUuid(cur ? cur.uuid : null);
    } catch (e) {
      setLoadError(String(e));
      toast(String(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => () => msUnlisten.current?.(), []);

  const setActive = async (uuid: string) => {
    try {
      await setCurrentAccount(uuid);
      setCurrentUuid(uuid);
      toast("已设为当前账户", "success");
    } catch (e) {
      toast(String(e), "error");
    }
  };

  const confirmRemove = async () => {
    if (!removeTarget) return;
    setRemoveBusy(true);
    try {
      await removeAccount(removeTarget.uuid);
      toast("已删除账户", "success");
      setRemoveTarget(null);
      await load();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setRemoveBusy(false);
    }
  };

  const openOffline = () => {
    setOfflineName("");
    setOfflineOpen(true);
  };

  const submitOffline = async () => {
    const name = offlineName.trim();
    if (!name) {
      toast("请输入玩家名", "error");
      return;
    }
    setOfflineBusy(true);
    try {
      await createOfflineAccount(name);
      toast("已创建离线账户", "success");
      setOfflineOpen(false);
      setOfflineName("");
      await load();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setOfflineBusy(false);
    }
  };

  const openAuthlib = () => {
    setAuthServer("");
    setAuthUser("");
    setAuthPass("");
    setAuthOpen(true);
  };

  const submitAuthlib = async () => {
    const server = authServer.trim();
    const user = authUser.trim();
    if (!server || !user || !authPass) {
      toast("请填写服务器地址、用户名与密码", "error");
      return;
    }
    setAuthBusy(true);
    try {
      await authlibLogin(server, user, authPass);
      toast("外置登录成功", "success");
      setAuthOpen(false);
      setAuthServer("");
      setAuthUser("");
      setAuthPass("");
      await load();
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setAuthBusy(false);
    }
  };

  const startMicrosoft = async () => {
    const runId = ++msRunId.current;
    msUnlisten.current?.();
    msUnlisten.current = null;
    setDeviceCode(null);
    setMsBusy(true);
    setMsOpen(true);
    try {
      msUnlisten.current = await onDeviceCode((code) => {
        if (msRunId.current === runId) setDeviceCode(code);
      });
      await microsoftLogin();
      if (msRunId.current === runId) {
        toast("微软登录成功", "success");
        setMsOpen(false);
      }
      await load();
    } catch (e) {
      if (msRunId.current === runId) {
        toast(String(e), "error");
        setMsOpen(false);
      }
    } finally {
      // 仅当本会话仍是最新会话时才收尾，避免清掉后开会话的监听器。
      if (msRunId.current === runId) {
        msUnlisten.current?.();
        msUnlisten.current = null;
        setMsBusy(false);
      }
    }
  };

  // 手动关闭：作废当前会话号。后端登录无法取消，其迟到结果将被 runId 守卫忽略。
  const closeMicrosoft = () => {
    msRunId.current += 1;
    msUnlisten.current?.();
    msUnlisten.current = null;
    setMsOpen(false);
    setMsBusy(false);
  };

  const openVerify = async (uri: string) => {
    try {
      await openUrl(uri);
    } catch (e) {
      toast(String(e), "error");
    }
  };

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader
          title="账户"
          subtitle="管理微软正版与离线账户"
          right={
            <Button
              variant="secondary"
              icon={<RefreshIcon size={16} />}
              onClick={load}
              disabled={accounts === null && !loadError}
            >
              刷新
            </Button>
          }
        />
      </motion.div>

      <motion.div variants={pageItem}>
        {loadError ? (
          <div className="rounded-[3px] border border-danger/40 bg-paper p-[18px]">
            <div className="flex items-start gap-3 text-danger">
              <AlertIcon size={20} className="mt-0.5 shrink-0" />
              <div className="min-w-0">
                <div className="text-[14px] font-bold">读取账户失败</div>
                <div className="mt-1 font-mono text-[12px] break-words text-danger/80">
                  {loadError}
                </div>
              </div>
            </div>
            <div className="mt-4">
              <Button variant="secondary" icon={<RefreshIcon size={16} />} onClick={load}>
                重试
              </Button>
            </div>
          </div>
        ) : accounts === null ? (
          <p className="font-mono text-[12px] tracking-[0.06em] text-ink/50">正在读取账户…</p>
        ) : accounts.length === 0 ? (
          <EmptyState icon={<UserIcon />} title="还没有账户，用下方入口添加一个开始游戏。" />
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {accounts.map((a) => {
              const isCurrent = a.uuid === currentUuid;
              return (
                <div
                  key={a.uuid}
                  className={[
                    "flex flex-col rounded-[3px] border bg-paper-sink p-4",
                    isCurrent ? "border-ink" : "border-ink/10",
                  ].join(" ")}
                >
                  <div className="flex items-center gap-3.5">
                    <div
                      className={[
                        "flex h-11 w-11 shrink-0 items-center justify-center rounded-[3px] text-[17px] font-extrabold text-paper-on",
                        isCurrent ? "bg-accent" : "bg-ink",
                      ].join(" ")}
                      aria-hidden="true"
                    >
                      {initialOf(a.name)}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-[15px] font-bold">{a.name}</span>
                        {isCurrent && (
                          <span className="shrink-0 rounded-[2px] bg-accent/12 px-1.5 py-0.5 text-[10px] font-bold tracking-[0.08em] text-accent">
                            当前
                          </span>
                        )}
                      </div>
                      <div className="mt-0.5 text-[12px] text-ink/55">
                        {TYPE_LABEL[a.account_type]}
                      </div>
                    </div>
                  </div>
                  <div className="mt-4 flex items-center gap-2">
                    {!isCurrent && (
                      <Button variant="secondary" onClick={() => setActive(a.uuid)}>
                        设为当前
                      </Button>
                    )}
                    <Button variant="secondary" onClick={() => setRemoveTarget(a)}>
                      删除
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </motion.div>

      <motion.div variants={pageItem} className="mt-9">
        <h2 className="mb-4 text-[12px] font-bold tracking-[0.16em] text-ink/40">添加账户</h2>
        <div className="flex flex-wrap gap-3">
          <Button
            variant="primary"
            icon={<UserIcon size={18} />}
            onClick={startMicrosoft}
            disabled={msBusy}
          >
            微软登录
          </Button>
          <Button variant="secondary" onClick={openOffline}>
            离线账户
          </Button>
          <Button variant="secondary" onClick={openAuthlib}>
            外置登录
          </Button>
        </div>
      </motion.div>

      <Modal
        open={msOpen}
        onClose={closeMicrosoft}
        title="微软登录"
        footer={
          <Button variant="secondary" onClick={closeMicrosoft}>
            取消
          </Button>
        }
      >
        {deviceCode ? (
          <div>
            <p className="text-[13.5px] text-ink/70">
              在浏览器打开下方网址，并输入配对码完成登录：
            </p>
            <div className="mt-3 rounded-[3px] border border-ink/12 bg-paper-sink px-4 py-3 text-center">
              <div className="font-mono text-[26px] font-bold tracking-[0.3em] text-ink tabular-nums">
                {deviceCode.user_code}
              </div>
            </div>
            <div className="mt-4 flex flex-col gap-2.5">
              <div className="font-mono text-[13px] break-all text-ink/70">
                {deviceCode.verification_uri}
              </div>
              <div>
                <Button variant="primary" onClick={() => openVerify(deviceCode.verification_uri)}>
                  打开验证网址
                </Button>
              </div>
            </div>
            <p className="mt-4 text-[12.5px] text-ink/55">{deviceCode.message}</p>
          </div>
        ) : (
          <p className="text-[13.5px] text-ink/60">正在向微软申请配对码，请稍候…</p>
        )}
      </Modal>

      <Modal
        open={offlineOpen}
        onClose={() => setOfflineOpen(false)}
        title="离线账户"
        footer={
          <>
            <Button variant="secondary" onClick={() => setOfflineOpen(false)}>
              取消
            </Button>
            <Button variant="primary" onClick={submitOffline} disabled={offlineBusy}>
              创建
            </Button>
          </>
        }
      >
        <TextField
          label="玩家名"
          value={offlineName}
          onChange={setOfflineName}
          placeholder="例如 Steve"
        />
      </Modal>

      <Modal
        open={authOpen}
        onClose={() => setAuthOpen(false)}
        title="外置登录"
        footer={
          <>
            <Button variant="secondary" onClick={() => setAuthOpen(false)}>
              取消
            </Button>
            <Button variant="primary" onClick={submitAuthlib} disabled={authBusy}>
              登录
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          <TextField
            label="服务器地址"
            value={authServer}
            onChange={setAuthServer}
            placeholder="https://例如 littleskin.cn/api/yggdrasil"
          />
          <TextField label="用户名" value={authUser} onChange={setAuthUser} placeholder="邮箱或用户名" />
          <TextField
            label="密码"
            type="password"
            value={authPass}
            onChange={setAuthPass}
            placeholder="账户密码"
          />
        </div>
      </Modal>

      <Modal
        open={removeTarget !== null}
        onClose={() => setRemoveTarget(null)}
        title="删除账户"
        footer={
          <>
            <Button variant="secondary" onClick={() => setRemoveTarget(null)}>
              取消
            </Button>
            <Button variant="primary" onClick={confirmRemove} disabled={removeBusy}>
              删除
            </Button>
          </>
        }
      >
        <p className="text-[14px] text-ink/80">
          确定删除账户
          <span className="font-bold text-ink">「{removeTarget?.name}」</span>
          吗？此操作不可撤销。
        </p>
      </Modal>
    </>
  );
}
