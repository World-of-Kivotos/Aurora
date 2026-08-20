// 账户页：微软正版 / 离线 / 外置（authlib-injector）三类账户的增删与切换。
// 账户命令多为 Windows 专属：非 Windows 下 listAccounts 返回空、登录命令 reject——如实展示，不崩。
// IPC reject 一个字符串；本页作为最外层展示层用 try/catch → toast 暴露，绝不吞。

import { useCallback, useEffect, useId, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PageHeader } from "../components/PageHeader";
import { Button } from "../components/Button";
import { EmptyState } from "../components/EmptyState";
import { SkinHead } from "../components/SkinHead";
import { Modal } from "../components/Modal";
import { useToast } from "../components/Toast";
import { UserIcon, RefreshIcon, AlertIcon } from "../components/icons";
import { pageItem, springs } from "../lib/motion";
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

// 输入框走下沉块：它是寄生层（只铺墨洗、不带 backdrop-filter），本页所有输入都在弹窗里，
// 宿主是弹窗那层自足材质，符合「寄生层不得直接铺在照片上」。
// 悬停/聚焦不再改描边色——材质的描边是 inset 阴影，再挂一条 border 会撑大盒子；
// 聚焦的可见性由 focus-visible 的朱红焦点环承担（文本框在 Chromium 里点选也会命中 focus-visible）。
const INPUT_CLS =
  "surface-sunken w-full rounded-control px-3.5 py-2.5 text-[14px] text-ink outline-none placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

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
      <span className="mb-1.5 block text-[12.5px] font-bold text-ink/75">{label}</span>
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
      {/* 外壳不滚（见 app.css 第六节），本页自己分配高度：报头与底部的「添加账户」定高不动，
          中间账户列表那一块吃掉剩余高度并在内部滚。账户数量是唯一会无限长的东西，只有它该滚。 */}
      <motion.div variants={pageItem} className="shrink-0">
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

      {/* 滚动区。min-h-0 不是保险：flex 子项的 min-height 默认 auto，不置 0 的话这块会被账户卡
          顶到内容真实高度，溢出原样传回外壳（那层是 overflow-clip，只会把底下的卡片无声裁掉）。
          pr-1 是给滚动条留的道，与 InstallPlanPreview 同一手法。 */}
      <motion.div variants={pageItem} className="min-h-0 flex-1 overflow-y-auto pr-1">
        {loadError ? (
          // 告警块用默认档材质。危险语态由图标与朱红字承担，不再靠一圈 border-danger：
          // 材质的描边走 inset 阴影，容器要分语态得由材质层出变体，页面自己加边框只会打架。
          // 报错正文用满档 danger 而不是 danger/80——后者压在这档玻璃上实算 4.26，不到 4.5。
          <div className="surface-panel rounded-panel p-[18px]">
            <div className="flex items-start gap-3 text-danger">
              <AlertIcon size={20} className="mt-0.5 shrink-0" />
              <div className="min-w-0">
                <div className="text-[14px] font-bold">读取账户失败</div>
                <div className="mt-1 font-mono text-[12px] break-words text-danger">{loadError}</div>
              </div>
            </div>
            <div className="mt-4">
              <Button variant="secondary" icon={<RefreshIcon size={16} />} onClick={load}>
                重试
              </Button>
            </div>
          </div>
        ) : accounts === null ? (
          // 读取中与空态这两句原本是裸字，从前靠内页的纸底托着；图铺满全站之后身下是照片，
          // 必须自带一档材质。w-fit 让这块纸只包住那一行，不至于为一句话铺满整行。
          <p className="surface-panel w-fit rounded-panel px-4 py-3 font-mono text-[12px] tracking-[0.06em] text-ink/75">
            正在读取账户…
          </p>
        ) : accounts.length === 0 ? (
          <div className="surface-panel rounded-panel px-5 py-2">
            <EmptyState icon={<UserIcon />} title="还没有账户，用下方入口添加一个开始游戏。" />
          </div>
        ) : (
          <div className="grid gap-3 sm:grid-cols-2">
            {/* 删除账户后 load() 重取会让卡片凭空消失、栅格硬跳位；靠 AnimatePresence 与 layout 让退场和重排连续可见。 */}
            <AnimatePresence initial={false}>
              {accounts.map((a) => {
                const isCurrent = a.uuid === currentUuid;
                // 「当前」那一档强调用 outline 而不是 border 或 ring：材质的描边是 ink/9 的 inset
                // 阴影，强调不出「当前」；border 会让两态差出 1px 把栅格挤歪（原来靠两态都留边框绕开），
                // 而 ring 在 Tailwind v4 里也是写 box-shadow，会被材质那条无层规则整条覆盖掉。
                // outline 画在边框盒之外、不占布局、不与 box-shadow 争同一个属性。
                return (
                  <motion.div
                    key={a.uuid}
                    layout
                    initial={{ opacity: 0, scale: 0.96 }}
                    animate={{ opacity: 1, scale: 1 }}
                    exit={{ opacity: 0, scale: 0.96 }}
                    transition={springs.settle}
                    className={[
                      "surface-panel flex flex-col rounded-panel p-4",
                      isCurrent ? "outline-1 outline-ink" : "",
                    ].join(" ")}
                  >
                    <div className="flex items-center gap-3.5">
                      <SkinHead uuid={a.uuid} name={a.name} size={44} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-[15px] font-bold">{a.name}</span>
                          {/* 切换当前账户时徽标在两张卡片间共享，避免一边灭一边亮的瞬移感。 */}
                          {isCurrent && (
                            <motion.span
                              layoutId="current-account-badge"
                              transition={springs.soft}
                              // 实心 accent 底 + 纸色字（4.77）。原先的 bg-accent/12 淡底配朱红字
                              // 在暗照片上只有 2.93，连图标的 3.0 都不到——徽标恰恰是最该一眼认出的那类字。
                              className="shrink-0 rounded-chip bg-accent px-1.5 py-0.5 text-[10px] font-bold tracking-[0.08em] text-paper-on"
                            >
                              当前
                            </motion.span>
                          )}
                        </div>
                        <div className="mt-0.5 text-[12px] text-ink/75">
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
                  </motion.div>
                );
              })}
            </AnimatePresence>
          </div>
        )}
      </motion.div>

      {/* 添加入口收进一块分区面板：小节标题本来是裸字，压在照片上读不出来；
          而三颗按钮各自的控件底是寄生层，也需要一层自足材质托着才合法。一块面板同时解决两件事。 */}
      {/* 常驻页尾：空态那句话写的就是「用下方入口添加」，它必须始终在下方够得着，
          不能随账户变多被推到滚动区里面去。 */}
      <motion.div
        variants={pageItem}
        className="surface-panel mt-9 shrink-0 rounded-panel px-5 py-5"
      >
        <h2 className="mb-4 text-[12px] font-bold tracking-[0.16em] text-ink/75">添加账户</h2>
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
            <p className="text-[13.5px] text-ink/75">
              在浏览器打开下方网址，并输入配对码完成登录：
            </p>
            {/* 配对码是要照抄的一串字符，按代码块处理：下沉块托底，寄生在弹窗那层材质上。 */}
            <div className="surface-sunken mt-3 rounded-panel px-4 py-3 text-center">
              <div className="font-mono text-[26px] font-bold tracking-[0.3em] text-ink tabular-nums">
                {deviceCode.user_code}
              </div>
            </div>
            <div className="mt-4 flex flex-col gap-2.5">
              <div className="font-mono text-[13px] break-all text-ink/75">
                {deviceCode.verification_uri}
              </div>
              <div>
                <Button variant="primary" onClick={() => openVerify(deviceCode.verification_uri)}>
                  打开验证网址
                </Button>
              </div>
            </div>
            <p className="mt-4 text-[12.5px] text-ink/75">{deviceCode.message}</p>
          </div>
        ) : (
          <p className="text-[13.5px] text-ink/75">正在向微软申请配对码，请稍候…</p>
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
        <p className="text-[14px] text-ink/75">
          确定删除账户
          <span className="font-bold text-ink">「{removeTarget?.name}」</span>
          吗？此操作不可撤销。
        </p>
      </Modal>
    </>
  );
}
