// 设置页：接通 aurora-core 配置（getConfig/updateConfig/setGameDirectory）+ Java 检测/下载 + 无障碍减少动效。
// 保存策略：Select/Toggle 即时保存（乐观更新，失败回滚 + toast）；数字输入 blur/Enter 提交；
// 游戏目录与 client_id 走显性按钮。错误一律 toast(String(e),"error")，不吞不掩盖。

import { useEffect, useState, type ReactNode } from "react";
import { motion } from "framer-motion";
import { PageHeader } from "../components/PageHeader";
import { Card } from "../components/Card";
import { Button } from "../components/Button";
import { Toggle } from "../components/Toggle";
import { Select } from "../components/Select";
import { EmptyState } from "../components/EmptyState";
import { PackageIcon, AlertIcon, RefreshIcon } from "../components/icons";
import { useToast } from "../components/Toast";
import { useMotionPref } from "../lib/motion-pref";
import { pageItem } from "../lib/motion";
import {
  getConfig,
  updateConfig,
  setGameDirectory,
  detectJava,
  installJava,
  onCoreEvent,
  type ConfigDto,
  type ConfigPatch,
  type DownloadSourcePolicy,
  type IsolationPolicy,
  type JavaInstallationDto,
} from "../lib/ipc";

const inputCls =
  "w-full rounded-[3px] border border-ink/16 bg-paper px-3.5 py-2.5 text-[14px] text-ink transition-colors placeholder:text-ink/35 focus-visible:border-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent";

const DOWNLOAD_SOURCE_OPTIONS: { value: DownloadSourcePolicy; label: string }[] = [
  { value: "auto", label: "自动（按网络择优）" },
  { value: "official_first", label: "官方源优先" },
  { value: "mirror_first", label: "镜像源优先" },
];

const ISOLATION_OPTIONS: { value: IsolationPolicy; label: string }[] = [
  { value: "disabled", label: "关闭隔离" },
  { value: "mod_loaders_only", label: "仅 Mod 加载器版本" },
  { value: "non_release_only", label: "仅非正式版本" },
  { value: "mod_loaders_and_non_release", label: "Mod 加载器与非正式版本" },
  { value: "all", label: "全部版本隔离" },
];

const JAVA_SOURCE_LABEL: Record<JavaInstallationDto["source"], string> = {
  registry: "注册表",
  common_dir: "常用目录",
  path: "PATH",
  managed: "托管",
};

// 分区外壳：小标题 + 下沉卡片，整块参与页面 stagger 入场。
function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <motion.div variants={pageItem} className="mt-7 first:mt-0">
      <h2 className="mb-3 text-[11px] font-bold tracking-[0.22em] text-ink/40">{title}</h2>
      <Card>{children}</Card>
    </motion.div>
  );
}

// 设置行：左侧标题+说明，右侧控件槽（定宽）。
function Row({ title, desc, control }: { title: string; desc: string; control: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-6 border-b border-ink/9 py-[18px] first:pt-0 last:border-b-0 last:pb-0">
      <div className="min-w-0">
        <div className="text-[15px] font-bold">{title}</div>
        <div className="mt-1 text-[12.5px] text-ink/60">{desc}</div>
      </div>
      <div className="shrink-0">{control}</div>
    </div>
  );
}

export function Settings() {
  const { toast } = useToast();
  const { reduceMotion, setReduceMotion } = useMotionPref();

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
    } catch (e) {
      toast(String(e), "error");
    } finally {
      setSavingGameDir(false);
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

  return (
    <>
      <motion.div variants={pageItem}>
        <PageHeader title="设置" subtitle="下载源、内存与目录" />
      </motion.div>

      {configLoading && (
        <motion.div variants={pageItem}>
          <Card>
            <p className="py-2 text-[13.5px] text-ink/55">载入配置中…</p>
          </Card>
        </motion.div>
      )}

      {!configLoading && configError && (
        <motion.div variants={pageItem}>
          <Card className="border-danger/40">
            <div className="flex items-start gap-3">
              <span className="text-danger">
                <AlertIcon size={20} />
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-[14px] font-bold text-danger">配置载入失败</div>
                <p className="mt-1 text-[13px] break-words text-ink/70">{configError}</p>
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
          </Section>

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
                  <span className="text-ink/35">/</span>
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

          <Section title="目录">
            <div className="border-b border-ink/9 py-[18px] first:pt-0">
              <div className="text-[15px] font-bold">游戏目录</div>
              <div className="mt-1 text-[12.5px] text-ink/60">.minecraft 所在位置，变更后需重新扫描版本</div>
              <div className="mt-3 flex items-center gap-2.5">
                <input
                  type="text"
                  aria-label="游戏目录"
                  className={inputCls}
                  value={gameDirInput}
                  onChange={(e) => setGameDirInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void applyGameDir();
                  }}
                />
                <Button variant="secondary" onClick={() => void applyGameDir()} disabled={savingGameDir}>
                  {savingGameDir ? "应用中…" : "应用"}
                </Button>
              </div>
            </div>
            <div className="py-[18px] last:pb-0">
              <div className="text-[12.5px] text-ink/45">数据目录</div>
              <div className="mt-1 font-mono text-[12px] break-all text-ink/55">{config.data_dir}</div>
            </div>
          </Section>

          <Section title="账户凭据">
            <div className="py-[18px] first:pt-0 last:pb-0">
              <div className="flex items-center justify-between gap-4">
                <div className="text-[15px] font-bold">微软 client_id</div>
                <span
                  className={[
                    "rounded-[2px] px-2 py-0.5 text-[11px] font-bold tracking-[0.08em]",
                    config.has_client_id ? "bg-ink text-paper-on" : "border border-ink/20 text-ink/50",
                  ].join(" ")}
                >
                  {config.has_client_id ? "已配置" : "未配置"}
                </span>
              </div>
              <div className="mt-1 text-[12.5px] text-ink/60">
                自定义 Azure 应用的 client_id，用于微软正版登录；出于安全不回显已保存的值
              </div>
              <div className="mt-3 flex items-center gap-2.5">
                <input
                  type="text"
                  aria-label="微软 client_id"
                  placeholder={config.has_client_id ? "输入以覆盖现有 client_id" : "输入 client_id"}
                  className={inputCls}
                  value={clientIdInput}
                  onChange={(e) => setClientIdInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void applyClientId();
                  }}
                />
                <Button variant="secondary" onClick={() => void applyClientId()} disabled={savingClientId}>
                  {savingClientId ? "保存中…" : "保存"}
                </Button>
              </div>
            </div>
          </Section>
        </>
      )}

      <Section title="Java 运行时">
        <div className="border-b border-ink/9 pb-[18px]">
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

          {javaLoading && <p className="py-2 text-[13.5px] text-ink/55">扫描本机 Java…</p>}

          {!javaLoading && javaError && (
            <div className="flex items-start gap-3 py-1">
              <span className="text-danger">
                <AlertIcon size={18} />
              </span>
              <p className="text-[13px] break-words text-ink/70">{javaError}</p>
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
                  className="flex items-center justify-between gap-4 rounded-[2px] border border-ink/9 bg-paper px-3.5 py-2.5"
                >
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-[14px] font-bold tabular-nums">Java {j.version.major}</span>
                      <span className="font-mono text-[11px] text-ink/45">{j.version.raw}</span>
                    </div>
                    <div className="mt-0.5 truncate font-mono text-[11.5px] text-ink/50">{j.path}</div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <span className="rounded-[2px] border border-ink/16 px-2 py-0.5 text-[11px] text-ink/60">
                      {j.is_64bit ? "64 位" : "32 位"}
                    </span>
                    <span className="rounded-[2px] border border-ink/16 px-2 py-0.5 text-[11px] text-ink/60">
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
          <div className="mt-1 text-[12.5px] text-ink/60">按主版本号获取由启动器托管的 JRE（如 8 / 17 / 21）</div>
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
            <p className="mt-2.5 font-mono text-[12px] break-words text-ink/60">{coreStatus}</p>
          )}
        </div>
      </Section>

      <Section title="无障碍">
        <Row
          title="减少动态效果"
          desc="降低或关闭界面动画（防晕动 / 低性能设备）"
          control={
            <Toggle ariaLabel="减少动态效果" checked={reduceMotion} onChange={setReduceMotion} />
          }
        />
      </Section>
    </>
  );
}
