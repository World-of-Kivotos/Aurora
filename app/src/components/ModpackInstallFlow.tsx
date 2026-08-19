// 一键安装整合包：地址输入 + 四步进度 + 失败现场。
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
}: ModpackInstallFlowProps) {
  const [pointerUrl, setPointerUrl] = useState(initialPointerUrl ?? builtIn?.pointer_url ?? "");
  const [validationError, setValidationError] = useState<string | null>(null);
  const running = state.kind === "running";

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
            安装服务器整合包
          </h2>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink/75">
            Aurora 会从地址读取版本要求，安装 Minecraft 与加载器，再用同一套同步流程写入整合包文件。
          </p>
        </div>
      </div>

      <div className="mt-5">
        <label htmlFor="modpack-pointer-url" className="text-[12px] font-bold text-ink/75">
          整合包地址
        </label>
        <div className="mt-1.5 flex gap-2">
          <input
            id="modpack-pointer-url"
            type="url"
            value={pointerUrl}
            onChange={(event) => {
              setPointerUrl(event.target.value);
              setValidationError(null);
            }}
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
              onClick={() => {
                setPointerUrl(builtIn.pointer_url);
                setValidationError(null);
              }}
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
            已创建实例 <span className="font-mono font-bold text-ink">{state.instance_id}</span>
            <span className="text-ink/75">整合包 {state.installed_version}</span>
          </div>
        </div>
      )}

      <div className="mt-5 flex items-center gap-3">
        {state.kind === "complete" && onOpenInstance ? (
          <Button variant="primary" onClick={() => onOpenInstance(state.instance_id)}>
            打开实例
          </Button>
        ) : (
          <Button
            variant="primary"
            icon={<DownloadIcon size={16} />}
            disabled={running}
            onClick={submit}
          >
            {running ? "正在安装" : state.kind === "failed" ? "重新安装" : "检查并安装"}
          </Button>
        )}
        <span className="text-[11.5px] text-ink/75">安装与后续更新共用同一同步路径</span>
      </div>
    </section>
  );
}
