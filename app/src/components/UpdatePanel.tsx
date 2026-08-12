// Mod 更新面板：列出可更新项，多选后走一次风险确认，再逐个串行更新。
//
// 三条产品决策，改动前请先读懂：
// 1. 更新前必须弹风险确认（抄自 PCL2）。Mod 更新不是「装个新东西」而是「换掉正在用的东西」，
//    存档兼容性与整合包版本锁定都可能被打破，界面有义务在动手前把代价讲清楚。
// 2. 串行而非并发。并发更新会让「第几个失败了」变得无法归因，且多个 install 同时改 mods/ 目录
//    与卷宗，失败现场会被搅乱。慢一点换一个能讲清楚的失败点。
// 3. 中途失败即停手，已落盘的不回退。后端 install 是「全部下到 staging 校验通过才移入」，
//    每个文件要么完整要么没动，所以停在半路是安全状态；替玩家回滚反而是二次破坏。
//    只要有一个成功就调 onUpdated——磁盘已经变了，父级的列表必须重取才不撒谎。

import { useMemo, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { Modal } from "./Modal";
import { useToast } from "./Toast";
import { AlertIcon, CheckIcon, DownloadIcon, PackageIcon, RefreshIcon } from "./icons";
import { springs } from "../lib/motion";
import { installMod, type ReleaseChannel, type UpdateCandidate } from "../lib/ipc";

interface UpdatePanelProps {
  versionId: string;
  candidates: UpdateCandidate[];
  onRefresh: () => void;
  onUpdated: () => void;
}

/** 同行控件统一 40px 高，与下载页口径一致。 */
const CTRL = "h-10";

const CHANNEL_LABEL: Record<ReleaseChannel, string> = {
  release: "正式版",
  beta: "测试版",
  alpha: "预览版",
};

// 预发布通道借用 accent 描边示警：这是少数「玩家不知情就会踩坑」的场景，值得用掉一次强调色。
const CHANNEL_CLASS: Record<ReleaseChannel, string> = {
  release: "bg-ink/[0.07] text-ink/60",
  beta: "border border-accent/40 text-accent",
  alpha: "border border-accent/40 text-accent",
};

/** 后端在平台缺 date_published 时给空串，如实说「未知」而不是编一个日期出来。 */
function fmtDate(iso: string): string {
  return iso ? iso.slice(0, 10) : "日期未知";
}

interface RunState {
  /** 从 1 开始，直接用于「第 i / N 个」文案。 */
  index: number;
  total: number;
  fileName: string;
}

/** 勾选记号：行本身是 role=checkbox 的按钮，这里只画形状，不接事件（避免按钮套按钮）。 */
function CheckGlyph({ state }: { state: "on" | "off" | "mixed" }) {
  return (
    <span
      aria-hidden="true"
      className={[
        "grid h-[18px] w-[18px] shrink-0 place-items-center rounded-[3px] border transition-colors",
        state === "off" ? "border-ink/30 text-transparent" : "border-ink bg-ink text-paper-on",
      ].join(" ")}
    >
      {state === "mixed" ? <span className="h-[2px] w-[9px] bg-paper-on" /> : <CheckIcon size={12} />}
    </span>
  );
}

export function UpdatePanel({ versionId, candidates, onRefresh, onUpdated }: UpdatePanelProps) {
  const { toast } = useToast();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirming, setConfirming] = useState(false);
  const [running, setRunning] = useState<RunState | null>(null);
  const [done, setDone] = useState<Set<string>>(new Set());
  const [error, setError] = useState<string | null>(null);

  // 选中集以 file_name 为键，与父级刷新后的新列表求交——否则已消失的旧项会把计数撑虚。
  const picked = useMemo(
    () => candidates.filter((c) => selected.has(c.file_name)),
    [candidates, selected],
  );
  const busy = running !== null;
  const allOn = candidates.length > 0 && picked.length === candidates.length;
  const headState = allOn ? "on" : picked.length > 0 ? "mixed" : "off";

  const toggleOne = (fileName: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(fileName)) next.delete(fileName);
      else next.add(fileName);
      return next;
    });
  };

  const toggleAll = () => {
    setSelected(allOn ? new Set() : new Set(candidates.map((c) => c.file_name)));
  };

  const runUpdate = async () => {
    const queue = picked;
    if (queue.length === 0) return;
    setConfirming(false);
    setError(null);
    setDone(new Set());

    let succeeded = 0;
    let failure: { fileName: string; message: string } | null = null;

    for (let i = 0; i < queue.length; i += 1) {
      const item = queue[i];
      setRunning({ index: i + 1, total: queue.length, fileName: item.file_name });
      try {
        await installMod(versionId, item.latest.platform, item.latest.project_id, item.latest.version_id);
      } catch (e) {
        // 这里是前端最外层：错误不能继续往上冒（没人接），必须留在界面上让玩家看见失败在哪一个。
        failure = { fileName: item.file_name, message: String(e) };
        break;
      }
      succeeded += 1;
      setDone((prev) => new Set(prev).add(item.file_name));
    }

    setRunning(null);
    // 失败项与其后未开始的项继续留在选中态，玩家排除故障后可原地重试。
    setSelected(new Set(queue.slice(succeeded).map((c) => c.file_name)));

    if (failure) {
      const rest = queue.length - succeeded - 1;
      setError(
        `${failure.fileName} 更新失败：${failure.message}` +
          `（已成功 ${succeeded} 个${rest > 0 ? `，剩余 ${rest} 个未开始` : ""}；已更新的不会回退）`,
      );
      toast(`${failure.fileName} 更新失败`, "error");
    } else {
      toast(`已更新 ${succeeded} 个 Mod`, "success");
    }

    if (succeeded > 0) onUpdated();
  };

  if (candidates.length === 0) {
    return (
      <EmptyState
        icon={<PackageIcon />}
        title="全部都是最新的"
        action={{ label: "重新检查", onClick: onRefresh }}
      />
    );
  }

  const pct = running ? Math.round(((running.index - 1) / running.total) * 100) : 0;

  return (
    <section aria-busy={busy} className="min-w-0">
      <div className="mb-4 flex items-center gap-3">
        <button
          type="button"
          role="checkbox"
          aria-checked={headState === "mixed" ? "mixed" : headState === "on"}
          onClick={toggleAll}
          disabled={busy}
          className={`${CTRL} inline-flex cursor-pointer items-center gap-2.5 rounded-[3px] px-1 text-[13px] font-bold text-ink/75 transition-colors hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:pointer-events-none disabled:opacity-45`}
        >
          <CheckGlyph state={headState} />
          全选
        </button>
        <span className="font-mono text-[12px] text-ink/60 tabular-nums">
          {candidates.length} 个可更新
        </span>
        <Button
          variant="secondary"
          className={`${CTRL} ml-auto shrink-0`}
          icon={<RefreshIcon size={15} />}
          onClick={onRefresh}
          disabled={busy}
        >
          重新检查
        </Button>
      </div>

      {error && (
        <div className="mb-4 flex items-start gap-3 rounded-[3px] border border-danger/35 bg-danger/[0.04] px-4 py-3">
          <span className="mt-0.5 shrink-0 text-danger [&_svg]:h-[18px] [&_svg]:w-[18px]">
            <AlertIcon />
          </span>
          <span className="flex-1 text-[13px] break-words text-danger">{error}</span>
        </div>
      )}

      <ul className="m-0 flex list-none flex-col gap-2 p-0">
        {candidates.map((c, i) => {
          const on = selected.has(c.file_name);
          const updated = done.has(c.file_name);
          const active = running?.fileName === c.file_name;
          return (
            <motion.li
              key={c.file_name}
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ ...springs.soft, delay: Math.min(i, 12) * 0.02 }}
            >
              <button
                type="button"
                role="checkbox"
                aria-checked={on}
                onClick={() => toggleOne(c.file_name)}
                disabled={busy}
                className={[
                  "flex w-full cursor-pointer items-center gap-3 rounded-[3px] border px-3.5 py-3 text-left transition-colors",
                  "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent",
                  "disabled:cursor-default disabled:opacity-55",
                  active
                    ? "border-ink bg-paper-sink"
                    : on
                      ? "border-ink/45 bg-paper-sink"
                      : "border-ink/10 bg-paper-sink/60 hover:border-ink/35",
                ].join(" ")}
              >
                <CheckGlyph state={on ? "on" : "off"} />

                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate text-[14px] leading-tight font-extrabold">{c.file_name}</span>
                  <span className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 font-mono text-[11px] text-ink/60 tabular-nums">
                    <span className="truncate line-through decoration-ink/30">{c.current_version_id}</span>
                    <span aria-hidden="true">{"->"}</span>
                    <span className="truncate font-bold text-ink/75">{c.latest.version_number}</span>
                    <span>{fmtDate(c.latest.date_published)}</span>
                  </span>
                </span>

                <span
                  className={`shrink-0 rounded-[3px] px-1.5 py-0.5 text-[11px] ${CHANNEL_CLASS[c.latest.release_channel]}`}
                >
                  {CHANNEL_LABEL[c.latest.release_channel]}
                </span>

                {updated && (
                  <span className="inline-flex shrink-0 items-center gap-1 text-[11px] font-bold text-ink/60">
                    <CheckIcon size={13} />
                    已更新
                  </span>
                )}
              </button>
            </motion.li>
          );
        })}
      </ul>

      {/* 悬浮工具条（形态抄自 PCL2）：sticky 贴在滚动区底部，选中或更新中才出现，不选就不占位。 */}
      <AnimatePresence initial={false}>
        {(picked.length > 0 || busy) && (
          <motion.div
            initial={{ opacity: 0, y: 14 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 14 }}
            transition={springs.pop}
            className="sticky bottom-4 z-10 mt-4"
          >
            <div className="rounded-[3px] border border-ink bg-paper-sink px-4 py-3">
              {running ? (
                <>
                  <div className="flex items-center gap-3">
                    <span className="text-[13px] font-bold text-ink">
                      更新中 第 {running.index} / {running.total} 个
                    </span>
                    <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink/60">
                      {running.fileName}
                    </span>
                  </div>
                  <div className="mt-2.5 h-[3px] w-full overflow-hidden rounded-[3px] bg-ink/12">
                    <motion.span
                      className="block h-full bg-ink"
                      initial={{ width: 0 }}
                      animate={{ width: `${pct}%` }}
                      transition={springs.settle}
                    />
                  </div>
                </>
              ) : (
                <div className="flex items-center gap-3">
                  <span className="text-[13px] font-bold text-ink tabular-nums">
                    已选择 {picked.length} 个
                  </span>
                  <Button
                    variant="primary"
                    className={`${CTRL} ml-auto shrink-0 !py-0`}
                    icon={<DownloadIcon size={16} />}
                    onClick={() => setConfirming(true)}
                  >
                    更新所选
                  </Button>
                  <Button
                    variant="secondary"
                    className={`${CTRL} shrink-0`}
                    onClick={() => setSelected(new Set())}
                  >
                    取消选择
                  </Button>
                </div>
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      <Modal
        open={confirming}
        onClose={() => setConfirming(false)}
        title="更新前请先确认"
        footer={
          <>
            <Button variant="secondary" onClick={() => setConfirming(false)}>
              再想想
            </Button>
            <Button variant="primary" onClick={() => void runUpdate()}>
              确认更新 {picked.length} 个
            </Button>
          </>
        }
      >
        <p className="text-[13.5px] text-ink/75">
          即将更新 {picked.length} 个 Mod。更新是「换掉正在用的文件」，请先了解三件事：
        </p>
        <ol className="mt-3 mb-0 flex list-none flex-col gap-2.5 p-0 text-[13.5px] leading-relaxed">
          <li className="flex gap-2.5">
            <span className="shrink-0 font-mono font-bold text-ink/60">1</span>
            <span>新版可能与旧存档或其它 Mod 不兼容，轻则功能异常，重则存档读不出来。</span>
          </li>
          <li className="flex gap-2.5">
            <span className="shrink-0 font-mono font-bold text-ink/60">2</span>
            <span>
              正在玩整合包时不建议自行更新单个 Mod——整合包作者锁定的版本组合一旦被打破，很容易连锁崩溃。
            </span>
          </li>
          <li className="flex gap-2.5">
            <span className="shrink-0 font-mono font-bold text-ink/60">3</span>
            <span>动手前建议先备份存档（saves 目录），出问题时才有退路。</span>
          </li>
        </ol>
        <p className="mt-3.5 mb-0 text-[12.5px] text-ink/60">
          旧文件会以 .old 保留在原目录并记入变更历史，可在历史记录里回滚。
        </p>
      </Modal>
    </section>
  );
}
