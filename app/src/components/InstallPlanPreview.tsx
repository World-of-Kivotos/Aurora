// 安装前的依赖清单预览：把「这一下点下去，究竟会往 mods/ 里放什么」摊开给玩家看之后再动手。
// 这是有实打实竞品空白的位置——CurseForge 官方在 CF-I-7577 里承认至今没做这个面板，玩家只能装完再自己数文件。
// 三条不可退让的底线：
// 1) 主项与依赖项在视觉上必须分得开，否则玩家会以为自己只装了一个 Mod；
// 2) 后端 skipped 里的每一条都要原样列出——为了界面干净而藏掉「有东西没被自动装」是最坏的一种体面；
// 3) 平台没给 file_size 时如实写「大小未知」，绝不拿 0 顶上凑一个好看的合计数字。

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { motion } from "framer-motion";
import { Button } from "./Button";
import { EmptyState } from "./EmptyState";
import { Skeleton } from "./Skeleton";
import { AlertIcon, CheckIcon, DownloadIcon, PackageIcon, RefreshIcon } from "./icons";
import { springs } from "../lib/motion";
import {
  planInstall,
  type InstallPlan,
  type PlannedItem,
  type PlatformId,
  type ReleaseChannel,
} from "../lib/ipc";

export interface InstallPlanPreviewProps {
  versionId: string;
  platform: PlatformId;
  projectId: string;
  modVersionId: string;
  onCancel: () => void;
  onConfirm: () => void;
}

/** 同行控件统一 40px 高，与下载页的工具条口径一致。 */
const CTRL = "h-10";

/** B/KB/MB 三档。为一个纯函数引第三方格式化库不值当，就地写。 */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  return `${(kb / 1024).toFixed(1)} MB`;
}

const CHANNEL_LABEL: Record<ReleaseChannel, string> = {
  release: "正式版",
  beta: "测试版",
  alpha: "预览版",
};

/** 计划汇总。已满足项不计入下载量——它们本次根本不落盘，算进去等于虚报体积。 */
interface PlanSummary {
  /** 本次真要下载的文件数。 */
  pending: number;
  /** 已装同版本、本次跳过的项数。 */
  satisfied: number;
  /** 已知大小项的字节合计。 */
  bytes: number;
  /** 平台没给大小的项数，用来决定合计要不要标「以上」。 */
  unknownSize: number;
}

function summaryText(s: PlanSummary): string {
  if (s.pending === 0) return "全部已满足，本次无需下载任何文件";
  if (s.unknownSize === s.pending) return `共 ${s.pending} 个文件，平台未提供文件大小`;
  if (s.unknownSize > 0) {
    return `共 ${s.pending} 个文件，合计 ${formatBytes(s.bytes)} 以上（${s.unknownSize} 项未提供大小）`;
  }
  return `共 ${s.pending} 个文件，合计 ${formatBytes(s.bytes)}`;
}

/** 非正式版必须扎眼：玩家在不知情下装到预览版，日后崩溃时根本想不到是发布通道的问题。 */
function ChannelBadge({ channel }: { channel: ReleaseChannel }) {
  const preview = channel !== "release";
  return (
    <span
      className={[
        "shrink-0 rounded-[2px] px-1.5 py-0.5",
        preview ? "bg-accent/10 font-bold text-accent" : "bg-ink/[0.07] text-ink/60",
      ].join(" ")}
    >
      {CHANNEL_LABEL[channel]}
    </span>
  );
}

function ErrorBar({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <div className="flex items-start gap-3 rounded-[3px] border border-danger/35 bg-danger/[0.04] px-4 py-3">
      <span className="mt-0.5 shrink-0 text-danger [&_svg]:h-[18px] [&_svg]:w-[18px]">
        <AlertIcon />
      </span>
      {/* 后端错误原文照登，方便玩家直接复制上报；不做二次包装、不降级成「出错了」。 */}
      <span className="min-w-0 flex-1 text-[13px] leading-snug break-words text-danger">{message}</span>
      <Button variant="secondary" className="shrink-0" icon={<RefreshIcon size={15} />} onClick={onRetry}>
        重试
      </Button>
    </div>
  );
}

function PlanRowSkeleton({ delay, indent }: { delay: number; indent: boolean }) {
  return (
    <li
      className={`rounded-[3px] border border-ink/8 bg-paper-sink/60 px-3.5 py-3 ${indent ? "ml-6" : ""}`}
    >
      <Skeleton className="h-[13px] w-2/3" delay={delay} />
      <Skeleton className="mt-2 h-[11px] w-1/3" delay={delay + 0.08} />
    </li>
  );
}

function PlanRow({
  item,
  requiredByLabel,
  index,
}: {
  item: PlannedItem;
  /** 依赖项的来源展示名；主项（用户主动选的那个）为 null。 */
  requiredByLabel: string | null;
  index: number;
}) {
  const v = item.version;
  const skipped = item.already_satisfied;
  const isDependency = requiredByLabel !== null;

  return (
    <motion.li
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ ...springs.soft, delay: Math.min(index, 10) * 0.03 }}
      className={[
        "rounded-[3px] border px-3.5 py-3",
        // 依赖项整体右缩进：层级靠版面位置表达，不靠额外的连线或图标。
        isDependency ? "ml-6" : "",
        skipped ? "border-ink/8 bg-paper-sink/45 opacity-55" : "border-ink/10 bg-paper-sink",
      ].join(" ")}
    >
      <div className="flex items-start gap-2">
        {/* 文件名是卷宗与磁盘 join 的键，等宽字体给足辨识度；长名换行而不是截断，玩家要能整串看到。 */}
        <span className="min-w-0 flex-1 font-mono text-[13px] leading-snug font-bold break-all text-ink">
          {v.file_name}
        </span>
        {isDependency ? (
          <span
            title={`因 ${requiredByLabel} 需要`}
            className="max-w-[46%] shrink-0 truncate rounded-[2px] bg-ink/[0.07] px-1.5 py-0.5 text-[11px] text-ink/60"
          >
            因 {requiredByLabel} 需要
          </span>
        ) : (
          <span className="shrink-0 rounded-[2px] bg-ink px-1.5 py-0.5 text-[11px] font-bold text-paper-on">
            你选择的
          </span>
        )}
      </div>

      <div className="mt-1.5 flex flex-wrap items-center gap-2 text-[11px] text-ink/60">
        <span className="font-mono tabular-nums">{v.version_number}</span>
        <ChannelBadge channel={v.release_channel} />
        <span className="font-mono tabular-nums">
          {v.file_size === null ? "大小未知" : formatBytes(v.file_size)}
        </span>
        {skipped && (
          <span className="inline-flex items-center gap-1 rounded-[2px] border border-ink/20 px-1.5 py-0.5 text-ink/60">
            <CheckIcon size={11} />
            已满足，将跳过
          </span>
        )}
      </div>
    </motion.li>
  );
}

export function InstallPlanPreview({
  versionId,
  platform,
  projectId,
  modVersionId,
  onCancel,
  onConfirm,
}: InstallPlanPreviewProps) {
  const [plan, setPlan] = useState<InstallPlan | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // 请求令牌：换了目标 Mod 之后迟到的旧响应必须丢弃，否则会把上一个 Mod 的计划贴到当前面板上。
  const requestRef = useRef(0);

  const load = useCallback(async () => {
    const token = ++requestRef.current;
    setLoading(true);
    setError(null);
    try {
      const result = await planInstall(versionId, platform, projectId, modVersionId);
      if (requestRef.current !== token) return;
      setPlan(result);
    } catch (e) {
      // 这里是 UI 最外层，错误落到界面上原样呈现，不吞、不降级。
      if (requestRef.current !== token) return;
      setError(String(e));
      setPlan(null);
    } finally {
      if (requestRef.current === token) setLoading(false);
    }
  }, [versionId, platform, projectId, modVersionId]);

  useEffect(() => {
    void load();
  }, [load]);

  // required_by 是 project_id，直接显示一串 id 对玩家毫无意义；用计划内同工程项的名字换成人话，
  // 换不到就退回原始 id——宁可露出丑陋的 id，也不编一个好看的名字。
  const nameByProject = useMemo(() => {
    const map = new Map<string, string>();
    if (plan === null) return map;
    for (const item of plan.items) map.set(item.version.project_id, item.version.name);
    return map;
  }, [plan]);

  const summary = useMemo<PlanSummary>(() => {
    const acc: PlanSummary = { pending: 0, satisfied: 0, bytes: 0, unknownSize: 0 };
    if (plan === null) return acc;
    for (const item of plan.items) {
      if (item.already_satisfied) {
        acc.satisfied += 1;
        continue;
      }
      acc.pending += 1;
      const size = item.version.file_size;
      if (size === null) acc.unknownSize += 1;
      else acc.bytes += size;
    }
    return acc;
  }, [plan]);

  const footerText =
    plan === null ? (loading ? "正在解析依赖关系" : "尚无可执行的计划") : summaryText(summary);

  return (
    <section aria-label="安装计划" aria-busy={loading} className="flex min-w-0 flex-col">
      <header className="mb-3 flex items-baseline gap-3">
        <h2 className="shrink-0 text-[10px] font-bold tracking-[0.2em] text-ink/60">安装计划</h2>
        <span className="min-w-0 truncate text-[12px] text-ink/60">落位到 {versionId}</span>
        <span className="ml-auto shrink-0 font-mono text-[12px] text-ink/60 tabular-nums">
          {plan === null ? "" : `${plan.items.length} 项`}
        </span>
      </header>

      {/* 依赖多的 Mod 计划可能十几项；列表内部滚动，保证底部的合计与按钮始终在视野里。 */}
      <div className="max-h-[42vh] min-h-0 overflow-y-auto pr-1">
        {plan === null && loading ? (
          <ul className="m-0 flex list-none flex-col gap-2 p-0">
            <PlanRowSkeleton delay={0} indent={false} />
            <PlanRowSkeleton delay={0.1} indent />
            <PlanRowSkeleton delay={0.2} indent />
          </ul>
        ) : error !== null ? (
          <ErrorBar message={error} onRetry={() => void load()} />
        ) : plan !== null && plan.items.length === 0 ? (
          <EmptyState
            icon={<PackageIcon />}
            title="后端没有给出任何可安装的文件，计划是空的"
            action={{ label: "重试", onClick: () => void load(), disabled: loading }}
          />
        ) : plan !== null ? (
          <>
            <ul className="m-0 flex list-none flex-col gap-2 p-0">
              {plan.items.map((item, i) => (
                <PlanRow
                  key={`${item.version.platform}-${item.version.version_id}`}
                  item={item}
                  requiredByLabel={
                    item.required_by === null
                      ? null
                      : (nameByProject.get(item.required_by) ?? item.required_by)
                  }
                  index={i}
                />
              ))}
            </ul>

            {/* 未自动处理：可选依赖、找不到匹配版本的依赖都在这里。这一块存在的全部意义就是不让玩家蒙在鼓里。 */}
            {plan.skipped.length > 0 && (
              <section className="mt-4 rounded-[3px] border border-ink/12 px-3.5 py-3">
                <div className="flex items-center gap-2">
                  <span className="text-ink/30 [&_svg]:h-[15px] [&_svg]:w-[15px]">
                    <AlertIcon />
                  </span>
                  <h3 className="text-[10px] font-bold tracking-[0.2em] text-ink/60">未自动处理</h3>
                </div>
                <ul className="m-0 mt-2 flex list-none flex-col gap-1.5 p-0">
                  {plan.skipped.map((note, i) => (
                    <li key={`${i}-${note}`} className="flex gap-2 text-[12.5px] leading-snug text-ink/60">
                      <span aria-hidden="true" className="shrink-0 text-ink/30">
                        -
                      </span>
                      <span className="min-w-0">{note}</span>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </>
        ) : null}
      </div>

      <footer className="mt-4 flex items-center gap-3 border-t border-ink/12 pt-4">
        <div className="min-w-0">
          <p className="text-[13px] leading-tight font-bold text-ink">{footerText}</p>
          {summary.satisfied > 0 && (
            <p className="mt-1 text-[11px] text-ink/60">
              另有 {summary.satisfied} 项已装同版本，将跳过
            </p>
          )}
        </div>
        <Button
          variant="primary"
          className={`${CTRL} ml-auto shrink-0 !py-0`}
          icon={<DownloadIcon size={16} />}
          onClick={onConfirm}
          // 计划取不到、或后端给了个空计划，就没有任何东西可装，按钮必须是死的。
          disabled={plan === null || plan.items.length === 0}
        >
          确认安装
        </Button>
        <Button variant="secondary" className={`${CTRL} shrink-0`} onClick={onCancel}>
          取消
        </Button>
      </footer>
    </section>
  );
}
