// 崩溃提示条：游戏异常退出后由主页被动挂出。纯展示 + 就地止损动作，自身不取数，
// 出现时机完全由调用方控制（report 由外部传入），避免横条自己去轮询日志抢主页的加载节奏。
// 文案纪律：规则命中只是线索不是定论，一律写「日志指向 X」，绝不写「X 导致崩溃」。

import { useState } from "react";
import { motion } from "framer-motion";
import { springs } from "../lib/motion";
import { setModEnabled, type CrashDiagnosis, type CrashReport } from "../lib/ipc";
import { Button } from "./Button";
import { useToast } from "./Toast";
import { AlertIcon, CheckIcon } from "./icons";

interface CrashBannerProps {
  report: CrashReport;
  versionId: string;
  onDismiss: () => void;
  /** 跳实例卷宗页的诊断区。 */
  onOpenDetail: () => void;
  /** 是否浮在背景图上。由调用方下发而不是自己读外观，免得与外壳的判定漂移。 */
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
      // 有图时改用磨砂：外壳整体是磨砂之后，一块全实心的纸就成了整窗唯一不透光的东西，
      // 那正是「像贴纸」的由来。取更实的 92% 一档而不是外壳的 85%——这是报警面板，
      // danger 文字压在它上面仍有 6.83:1（实心 paper-sink 基线是 7.65:1），可读性没让步。
      className={[
        "rounded-[3px] border border-danger p-4",
        onPhoto ? "paper-frost-strong" : "bg-paper",
      ].join(" ")}
    >
      <div className="flex items-start gap-3">
        <AlertIcon size={20} className="mt-px shrink-0 text-danger" />
        <div className="min-w-0 flex-1">
          <div className="flex items-baseline justify-between gap-3">
            <p className="min-w-0 text-[14px] font-extrabold text-ink">
              {/* 空诊断不是异常，是「日志里没有已知特征」这个真实结论，如实说出来而不是拿空串糊过去。 */}
              {primary ? primary.summary : "游戏异常退出，日志里没有匹配到已知的崩溃特征"}
            </p>
            <span className="shrink-0 truncate text-[12px] text-ink/60">{versionId}</span>
          </div>
          {primary && <p className="mt-1.5 text-[13px] leading-relaxed text-ink/60">{primary.advice}</p>}
        </div>
      </div>

      {shown.length > 0 && (
        <ul className="mt-3.5 flex flex-col gap-1.5">
          {shown.map((suspect) => {
            const file = suspect.file_name;
            const state = (file ? states[file] : undefined) ?? "idle";
            return (
              <li
                key={`${suspect.mod_id}:${file ?? ""}`}
                className="flex items-center justify-between gap-3 rounded-[3px] bg-paper-sink px-3 py-1"
              >
                <span className="flex min-w-0 flex-1 items-baseline gap-2 text-[13px] text-ink/75">
                  <span className="shrink-0">日志指向</span>
                  <span className="truncate font-extrabold text-ink">{file ?? suspect.mod_id}</span>
                  {/* 卷宗对不上文件时只能报 mod id，此时没有可禁用的目标，说清楚而不是给个点不动的按钮。 */}
                  {!file && <span className="shrink-0 text-[12px] text-ink/60">（卷宗未对上文件）</span>}
                </span>
                {file && (
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
              </li>
            );
          })}
          {overflow > 0 && <li className="px-3 pt-0.5 text-[12.5px] text-ink/60">等 {overflow} 个</li>}
        </ul>
      )}

      <div className="mt-4 flex items-center gap-2.5">
        <Button variant="primary" className="h-10" onClick={onOpenDetail}>
          查看详情
        </Button>
        <Button variant="secondary" className="h-10" onClick={onDismiss}>
          关闭
        </Button>
        {extraDiagnoses > 0 && <span className="text-[12.5px] text-ink/60">另有 {extraDiagnoses} 条诊断</span>}
      </div>
    </motion.section>
  );
}
