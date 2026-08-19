// 崩溃提示条：游戏异常退出后由主页被动挂出。纯展示 + 就地止损动作，自身不取数，
// 出现时机完全由调用方控制（report 由外部传入），避免横条自己去轮询日志抢主页的加载节奏。
// 文案纪律：规则命中只是线索不是定论，一律写「日志指向 X」，绝不写「X 导致崩溃」。

import { useState } from "react";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { setModEnabled, type CrashDiagnosis, type CrashReport } from "../lib/ipc";
import type { ModpackFileOwner } from "../lib/modpack-ui";
import { Button } from "./Button";
import { useToast } from "./Toast";
import { AlertIcon, CheckIcon } from "./icons";

interface CrashBannerProps {
  report: CrashReport;
  versionId: string;
  onDismiss: () => void;
  /** 跳实例卷宗页的诊断区。 */
  onOpenDetail: () => void;
  /** 每个可疑文件独立判定归属；归属未确认时必须关闭破坏性动作。 */
  ownerOf: (fileName: string) => ModpackFileOwner | null;
  /**
   * 是否浮在背景图上。由调用方下发而不是自己读外观，免得与外壳的判定漂移。
   * 现在只决定投影：材质档不再随它变（见下方注释）。
   */
  onPhoto: boolean;
}

// 横条只铺最靠前的几个可疑文件，其余交给详情页——它是止损入口不是完整清单。
const MAX_SUSPECTS = 3;

type DisableState = "idle" | "pending" | "done";

const disableLabel: Record<DisableState, string> = {
  idle: "禁用它",
  pending: "禁用中",
  done: "已禁用",
};

export function CrashBanner({
  report,
  versionId,
  onDismiss,
  onOpenDetail,
  ownerOf,
  onPhoto,
}: CrashBannerProps) {
  const { toast } = useToast();
  // 稀疏表：只记录动过的文件，没碰过的按 idle 处理。
  const [states, setStates] = useState<Record<string, DisableState | undefined>>({});

  // tsconfig 未开 noUncheckedIndexedAccess，显式标注让「一条诊断都没命中」这个真实状态在类型上现形。
  const primary: CrashDiagnosis | undefined = report.diagnoses[0];
  const shown = report.suspects.slice(0, MAX_SUSPECTS);
  const overflow = report.suspects.length - shown.length;
  const extraDiagnoses = report.diagnoses.length - (primary ? 1 : 0);

  const disable = async (fileName: string) => {
    if (ownerOf(fileName) !== "player") return;
    setStates((s) => ({ ...s, [fileName]: "pending" }));
    try {
      await setModEnabled(versionId, fileName, false);
      setStates((s) => ({ ...s, [fileName]: "done" }));
      toast(`已禁用 ${fileName}`, "success");
    } catch (e) {
      // 事件处理器是这条调用链的最外层，必须退回 idle 让玩家能重试，
      // 停在 pending 等于假装还在跑，比直接报错更糟。
      setStates((s) => ({ ...s, [fileName]: "idle" }));
      toast(String(e), "error");
    }
  };

  return (
    <motion.section
      role="alert"
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: -8 }}
      transition={springs.settle}
      // 材质取最实的 .surface-panel-strong（96%）而不是默认的 .surface-panel（86%）：
      // 这是报警面板，玩家要在上面逐条读文件名再做破坏性决定，判据与「长列表买 AAA 余量」同源。
      // 实算：danger 图标 7.48、ink/75 正文 7.20，都越过 7:1；在 86% 那档只有 6.28。
      // 材质不再随 onPhoto 分支——96% 的纸色压在纸底上与压在照片上观感一致，
      // onPhoto 只剩投影这一件事：压在照片上是实打实的两个平面要有影，
      // 落在纸底页面上则是纸对纸，按 .surface-nested 把影子摘掉。
      className={[
        "rounded-panel border border-danger p-4 surface-panel-strong",
        onPhoto ? "" : "surface-nested",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className="flex items-start gap-3">
        <AlertIcon size={20} className="mt-px shrink-0 text-danger" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-3">
            <p className="min-w-0 text-[14px] font-extrabold text-ink">
              {/* 空诊断不是异常，是「日志里没有已知特征」这个真实结论，如实说出来而不是拿空串糊过去。 */}
              {primary ? primary.summary : "游戏异常退出，日志里没有匹配到已知的崩溃特征"}
            </p>
            <span className="shrink-0 truncate text-[12px] text-ink/75">{versionId}</span>
          </div>
          {primary && <p className="mt-1.5 text-[13px] leading-relaxed text-ink/75">{primary.advice}</p>}
        </div>
      </div>

      {shown.length > 0 && (
        <ul className="mt-3.5 flex flex-col gap-1.5">
          {shown.map((suspect) => {
            const file = suspect.file_name;
            const owner = file ? ownerOf(file) : null;
            const state = (file ? states[file] : undefined) ?? "idle";
            return (
              <li
                key={`${suspect.mod_id}:${file ?? ""}`}
                // 可疑文件行是下沉块而不是控件底：整行不可点，可点的只有行尾那颗按钮。
                // 「寄生层不得寄生在寄生层上」那条纪律是按最透的外壳（72%）解出来的，
                // 这里的宿主是最实的 96%：行底 8% 叠按钮底 4% 合计 12% 墨洗后，
                // ink/75 实算仍有 6.36（纯黑图端），离 4.5 还有一大截，故这一处按数走而不是按结论走。
                className="flex items-center justify-between gap-3 rounded-control surface-sunken px-3 py-1"
              >
                <span className="flex min-w-0 flex-1 items-baseline gap-2 text-[13px] text-ink/75">
                  <span className="shrink-0">日志指向</span>
                  <span className="truncate font-extrabold text-ink">{file ?? suspect.mod_id}</span>
                  {/* 卷宗对不上文件时只能报 mod id，此时没有可禁用的目标，说清楚而不是给个点不动的按钮。 */}
                  {!file && <span className="shrink-0 text-[12px] text-ink/75">（卷宗未对上文件）</span>}
                </span>
                {file && owner === "player" && (
                  <Button
                    variant="secondary"
                    className="h-10 shrink-0"
                    disabled={state !== "idle"}
                    onClick={() => void disable(file)}
                    icon={state === "done" ? <CheckIcon size={14} /> : undefined}
                  >
                    {disableLabel[state]}
                  </Button>
                )}
                {file && owner === "modpack" && (
                  <span className="shrink-0 text-[12px] text-ink/75">由整合包统一维护，不能单独禁用</span>
                )}
                {file && owner === null && (
                  <span className="shrink-0 text-[12px] text-ink/75">文件归属尚未确认，暂不提供禁用操作</span>
                )}
              </li>
            );
          })}
          {overflow > 0 && <li className="px-3 pt-0.5 text-[12.5px] text-ink/75">等 {overflow} 个</li>}
        </ul>
      )}

      <div className="mt-4 flex items-center gap-2.5">
        <Button variant="primary" className="h-10" onClick={onOpenDetail}>
          查看详情
        </Button>
        <Button variant="secondary" className="h-10" onClick={onDismiss}>
          关闭
        </Button>
        {extraDiagnoses > 0 && <span className="text-[12.5px] text-ink/75">另有 {extraDiagnoses} 条诊断</span>}
      </div>
    </motion.section>
  );
}
