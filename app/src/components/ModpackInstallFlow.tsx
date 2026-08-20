// 一键把游戏装上：地址输入 + 四步进度 + 失败现场。
//
// 场景已从「在资源浏览器里再装一个整合包」变成「这台启动器唯一的装游戏途径」——
// Aurora 收敛为 World of Kivotos 专用启动器后，下载页不再有游戏版本与整合包 tab，
// 玩家拿到游戏的唯一入口就是启动屏上的这套流程，措辞按「把游戏装上」写。
//
// 材质分层（与本组另外三个文件同一套）：整块面板直接压在照片上，只有它挂 .surface-panel；
// 里面的步骤条、失败块、完成条一律不挂第二层玻璃，靠描边与语义色区分。
// 地址输入框是「下沉块」，用寄生的 .surface-sunken——寄生层不能直接铺在照片上，
// 但它此刻套在面板里，合规；也正因为如此，这个组件必须始终自带面板底，不能被剥成裸片使用。

import { useState } from "react";
import { Button } from "./Button";
import {
  ModpackSyncFailureView,
  ModpackSyncProgressView,
} from "./ManagedModpackPanel";
import { AlertIcon, CheckIcon, DownloadIcon, PackageIcon } from "./icons";
import {
  validateModpackPointerUrl,
  type ModpackSyncFailure,
  type ModpackSyncProgress,
  type ModpackSyncStage,
} from "../lib/modpack-ui";

export interface BuiltInModpack {
  label: string;
  pointer_url: string;
}

export interface ModpackInstallProblem {
  stage: ModpackSyncStage;
  title: string;
  detail: string;
  action: string;
}

export type ModpackInstallState =
  | { kind: "idle" }
  | {
      kind: "running";
      pointer_url: string;
      target_version: string;
      progress: ModpackSyncProgress;
    }
  | {
      kind: "failed";
      pointer_url: string;
      target_version: string | null;
      problem:
        | { kind: "setup"; failure: ModpackInstallProblem }
        | { kind: "sync"; stage: ModpackSyncStage; failure: ModpackSyncFailure };
    }
  | {
      kind: "complete";
      pointer_url: string;
      instance_id: string;
      installed_version: string;
    };

export interface ModpackInstallFlowProps {
  builtIn: BuiltInModpack | null;
  initialPointerUrl?: string;
  state: ModpackInstallState;
  onInstall: (pointerUrl: string) => void;
  onOpenInstance?: (instanceId: string) => void;
  /**
   * 地址每次变动都回报一次。
   *
   * 启动屏右下角那个「安装游戏」与这块面板里的按钮装的必须是同一个地址：
   * 玩家把地址改成自建整合包、却从角上点了安装，结果装回官方的，是最难查的那类不一致。
   * 地址仍由本组件自持（它才是编辑它的人），只把当前值同步给外面那颗按钮。
   */
  onPointerUrlChange?: (pointerUrl: string) => void;
}

const STEPS: { label: string; stages: ModpackSyncStage[] }[] = [
  { label: "读取整合包", stages: ["resolving_manifest"] },
  { label: "安装 Minecraft", stages: ["installing_minecraft"] },
  { label: "安装加载器", stages: ["installing_loader"] },
  {
    label: "同步整合包",
    stages: ["downloading_files", "deleting_files", "writing_snapshot"],
  },
];

function stageOf(state: ModpackInstallState): ModpackSyncStage | null {
  if (state.kind === "running") return state.progress.stage;
  if (state.kind === "failed" && state.problem.kind === "setup") return state.problem.failure.stage;
  if (state.kind === "failed" && state.problem.kind === "sync") return state.problem.stage;
  return null;
}

function InstallSteps({ state }: { state: ModpackInstallState }) {
  const currentStage = stageOf(state);
  const current = currentStage === null ? -1 : STEPS.findIndex((step) => step.stages.includes(currentStage));
  const finished = state.kind === "complete";

  return (
    <ol className="m-0 mt-5 grid list-none grid-cols-4 gap-2 border-y border-ink/9 py-3.5">
      {STEPS.map((step, index) => {
        const done = finished || current > index;
        const active = current === index;
        return (
          <li key={step.label} className="min-w-0">
            <div
              className={`mb-1.5 flex h-5 w-5 items-center justify-center rounded-full border text-[10px] font-bold ${
                done
                  ? "border-ink bg-ink text-paper-on"
                  : active
                    ? "border-accent bg-accent text-paper-on"
                    : "border-ink/30 text-ink/75"
              }`}
              aria-hidden="true"
            >
              {done ? <CheckIcon size={11} /> : index + 1}
            </div>
            <span className={`block text-[11.5px] leading-snug ${active || done ? "font-bold text-ink" : "text-ink/75"}`}>
              {step.label}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function SetupFailure({ failure }: { failure: ModpackInstallProblem }) {
  return (
    <div className="mt-4 flex items-start gap-3 rounded-panel border border-danger/35 px-4 py-3.5" role="alert">
      <span className="mt-0.5 shrink-0 text-danger">
        <AlertIcon size={18} />
      </span>
      <div className="min-w-0">
        <p className="text-[13px] font-bold text-danger">{failure.title}</p>
        <p className="mt-1 text-[12.5px] leading-relaxed break-words text-danger">{failure.detail}</p>
        <p className="mt-1.5 text-[12px] leading-relaxed text-ink/75">{failure.action}</p>
      </div>
    </div>
  );
}

export function ModpackInstallFlow({
  builtIn,
  initialPointerUrl,
  state,
  onInstall,
  onOpenInstance,
  onPointerUrlChange,
}: ModpackInstallFlowProps) {
  const [pointerUrl, setPointerUrl] = useState(initialPointerUrl ?? builtIn?.pointer_url ?? "");
  const [validationError, setValidationError] = useState<string | null>(null);
  const running = state.kind === "running";

  const changePointerUrl = (next: string) => {
    setPointerUrl(next);
    setValidationError(null);
    onPointerUrlChange?.(next);
  };

  const submit = () => {
    const error = validateModpackPointerUrl(pointerUrl);
    setValidationError(error);
    if (error !== null) return;
    onInstall(pointerUrl.trim());
  };

  return (
    <section aria-labelledby="modpack-install-title" className="surface-panel rounded-panel p-[18px]">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 shrink-0 text-accent">
          <PackageIcon size={20} />
        </span>
        <div>
          <h2 id="modpack-install-title" className="text-[16px] font-extrabold text-ink">
            安装 World of Kivotos
          </h2>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink/75">
            Aurora 会从服务器读取本期版本要求，装好 Minecraft 与加载器，再写入整合包文件。全程一步到位，装完即可开始游戏。
          </p>
        </div>
      </div>

      <div className="mt-5">
        <label htmlFor="modpack-pointer-url" className="text-[12px] font-bold text-ink/75">
          整合包地址（默认指向官方服务器，测试服才需要改）
        </label>
        <div className="mt-1.5 flex gap-2">
          <input
            id="modpack-pointer-url"
            type="url"
            value={pointerUrl}
            onChange={(event) => changePointerUrl(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !running) submit();
            }}
            disabled={running}
            spellCheck={false}
            placeholder="https://example.com/api/v1/pack/latest"
            aria-invalid={validationError !== null}
            aria-describedby={validationError ? "modpack-pointer-error" : undefined}
            // 输入框改用下沉块：不透明度在暗图与亮图上方向相反，「凹进去」只有墨洗表达得稳。
            // 原来的 hover/focus 换描边一并去掉——文本框在 Chromium 里点击也会命中 :focus-visible，
            // 朱红焦点环对鼠标用户同样出现，再叠一圈换色描边只是重复告知。
            // 占位符走 ink/75：13px 常规字重不吃大字豁免，而 ink/60 在下沉块上只有 3.27~3.82，
            // 全站另外五处输入框都已迁到 ink/75，这里是唯一漏掉的一处。
            className="surface-sunken h-10 min-w-0 flex-1 rounded-control px-3.5 font-mono text-[13px] text-ink outline-none placeholder:text-ink/75 focus-visible:outline-2 focus-visible:outline-accent focus-visible:outline-offset-2 disabled:opacity-55"
          />
          {builtIn && pointerUrl.trim() !== builtIn.pointer_url && (
            <Button
              variant="secondary"
              className="shrink-0"
              disabled={running}
              onClick={() => changePointerUrl(builtIn.pointer_url)}
            >
              使用{builtIn.label}
            </Button>
          )}
        </div>
        {validationError && (
          <p id="modpack-pointer-error" className="mt-1.5 text-[12px] text-danger" role="alert">
            {validationError}
          </p>
        )}
      </div>

      <InstallSteps state={state} />

      {state.kind === "running" && (
        <ModpackSyncProgressView progress={state.progress} />
      )}
      {state.kind === "failed" && state.problem.kind === "setup" && (
        <SetupFailure failure={state.problem.failure} />
      )}
      {state.kind === "failed" && state.problem.kind === "sync" && (
        <ModpackSyncFailureView failure={state.problem.failure} />
      )}
      {state.kind === "complete" && (
        <div className="mt-4 rounded-panel border border-ink/12 px-4 py-3.5" role="status">
          <div className="flex items-center gap-2.5 text-[13px] text-ink/75">
            <CheckIcon size={16} />
            游戏已装好 <span className="font-mono font-bold text-ink">{state.instance_id}</span>
            <span className="text-ink/75">整合包 {state.installed_version}</span>
          </div>
        </div>
      )}

      {/*
       * 这里刻意没有「安装游戏」按钮, 别再加回来。
       *
       * 启动屏右下角那颗主操作键在未安装时本身就是 Download, 它与这块面板要做的是同一件事。
       * 两处并列会让新玩家对着两个「下载」发愣, 也让「这一屏的主操作只有一个」这条版面规则失效。
       * 这块面板的职责是说清将要发生什么、以及给测试服改地址, 触发交给主操作位。
       *
       * 保留的两颗按钮都不是重复: complete 的「进入管理」是装完之后的去处,
       * failed 的「重新安装」是兜底 —— 安装若在创建实例之后才失败, current 已非空,
       * 主操作键会回到 Start 语义, 那时这颗就是唯一的重试入口。
       */}
      <div className="mt-5 flex items-center gap-3">
        {state.kind === "complete" && onOpenInstance ? (
          <Button variant="primary" onClick={() => onOpenInstance(state.instance_id)}>
            进入管理
          </Button>
        ) : state.kind === "failed" ? (
          <Button
            variant="primary"
            icon={<DownloadIcon size={16} />}
            disabled={running}
            onClick={submit}
          >
            重新安装
          </Button>
        ) : null}
        <span className="text-[11.5px] text-ink/75">
          {state.kind === "complete" || state.kind === "failed"
            ? "安装与日后更新共用同一条同步路径"
            : running
              ? "安装进行中, 进度显示在右下角"
              : "地址确认后点右下角的 Download 开始, 也可以在地址栏直接回车"}
        </span>
      </div>
    </section>
  );
}
