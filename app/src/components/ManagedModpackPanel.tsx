// 受管整合包面板：一个实例的整合包身份、版本差与同步进度/失败现场。
//
// 材质分层（与 UpdatePanel / ModpackInstallFlow / InstallPlanPreview 同一套）：
// 只有 ManagedModpackPanel 这一整块直接压在照片上，故只有它挂 .surface-panel；
// 里面的进度块、失败块、完成条一律不挂材质，靠描边与语义色（accent / danger）区分。
// 分层深的界面上，第二层玻璃买不到任何层次，只会把两层背景糊在一起。
//
// ModpackSyncProgressView / ModpackSyncFailureView 被 ModpackInstallFlow 复用，那边同样是
// 「面板里面」的位置，所以这两个视图默认无材质这件事在两个宿主下都成立，不需要开参数。
//
// 朱红只作填充不作文字：accent 压在 .surface-panel 上实算 3.41，够图标（3.0）不够小字（4.5）。
// 因此徽标从「朱红 10% 底 + 朱红字」翻成「朱红实底 + 纸色字」（纸色压朱红 4.78）；
// 图标与描边继续用 accent，那是图形不是文字。

import { Button } from "./Button";
import { AlertIcon, CheckIcon, DownloadIcon, PackageIcon, RefreshIcon } from "./icons";
import {
  SYNC_STAGE_LABEL,
  formatModpackBytes,
  modpackUpdateAvailable,
  presentSyncFailure,
  syncProgressRatio,
  type KnownModpackVersions,
  type ManagedModpackStatus,
  type ModpackFileOwner,
  type ModpackSyncFailure,
  type ModpackSyncProgress,
  type ModpackSyncState,
} from "../lib/modpack-ui";

export interface ManagedModpackPanelProps {
  status: ManagedModpackStatus;
  /** 当前实例 id。只在同步冲突那一支用到：要指名道姓地说清哪个目录会被留在磁盘上。 */
  instanceId: string;
  sync: ModpackSyncState;
  onCheck: () => void;
  onSync: (targetVersion: string) => void;
  onInstallNewVersion: () => void;
}

function versionsOf(status: ManagedModpackStatus): KnownModpackVersions | null {
  switch (status.kind) {
    case "checking":
    case "unavailable":
      return status.last_known;
    case "ready":
      return status.versions;
  }
}

function VersionValue({ label, value }: { label: string; value: string | null }) {
  return (
    <div className="min-w-0 flex-1">
      <div className="text-[11px] font-bold tracking-[0.16em] text-ink/75">{label}</div>
      <div className="mt-1 truncate font-mono text-[17px] font-extrabold text-ink tabular-nums">
        {value ?? "尚未同步"}
      </div>
    </div>
  );
}

export function ModpackSyncProgressView({
  progress,
}: {
  progress: ModpackSyncProgress;
}) {
  const ratio = syncProgressRatio(progress);
  const percent = Math.round(ratio * 100);
  const count =
    progress.total_files > 0
      ? `${progress.completed_files}/${progress.total_files} 个文件`
      : "正在准备文件清单";
  const bytes =
    progress.total_bytes !== null && progress.total_bytes > 0
      ? `${formatModpackBytes(progress.downloaded_bytes)} / ${formatModpackBytes(progress.total_bytes)}`
      : null;

  return (
    <div className="mt-4 rounded-panel border border-accent/25 px-4 py-3.5" role="status">
      <div className="flex items-baseline gap-3">
        <span className="text-[13px] font-bold text-ink">{SYNC_STAGE_LABEL[progress.stage]}</span>
        <span className="font-mono text-[11.5px] text-ink/75 tabular-nums">{count}</span>
        {bytes && <span className="ml-auto font-mono text-[11.5px] text-ink/75 tabular-nums">{bytes}</span>}
      </div>
      {/* 进度轨是下沉块（.surface-sunken），圆角走 control 档——圆角角色与材质角色是两条正交的轴。 */}
      <div
        className="surface-sunken mt-2.5 h-1.5 overflow-hidden rounded-control"
        role="progressbar"
        aria-label="整合包同步进度"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
      >
        <div className="h-full bg-accent transition-[width]" style={{ width: `${percent}%` }} />
      </div>
      {progress.current_file && (
        <p className="mt-2 truncate font-mono text-[11.5px] text-ink/75" title={progress.current_file}>
          {progress.current_file}
        </p>
      )}
    </div>
  );
}

export function ModpackSyncFailureView({
  failure,
}: {
  failure: ModpackSyncFailure;
}) {
  const presentation = presentSyncFailure(failure);
  return (
    <div className="mt-4 rounded-panel border border-danger/35 px-4 py-3.5" role="alert">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 shrink-0 text-danger">
          <AlertIcon size={18} />
        </span>
        <div className="min-w-0">
          <p className="font-mono text-[13px] font-bold break-all text-danger">{presentation.title}</p>
          <p className="mt-1 text-[12.5px] leading-relaxed text-danger">{presentation.reason}</p>
          <p className="mt-1.5 text-[12px] leading-relaxed text-ink/75">{presentation.action}</p>
        </div>
      </div>
    </div>
  );
}

export function ManagedModpackPanel({
  status,
  instanceId,
  sync,
  onCheck,
  onSync,
  onInstallNewVersion,
}: ManagedModpackPanelProps) {
  const versions = versionsOf(status);
  const displayedVersions =
    versions && sync.kind === "complete"
      ? { ...versions, installed_version: sync.installed_version }
      : versions;
  const updating = sync.kind === "running";
  const target = status.kind === "ready" ? status.versions.latest.version : null;
  const updateAvailable = status.kind === "ready" && displayedVersions !== null && modpackUpdateAvailable(displayedVersions);

  return (
    <section aria-labelledby="managed-modpack-title" className="surface-panel rounded-panel p-[18px]">
      <div className="flex flex-wrap items-start gap-4">
        <span className="mt-0.5 shrink-0 text-accent">
          <PackageIcon size={20} />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2.5 gap-y-1">
            <h2 id="managed-modpack-title" className="text-[15px] font-extrabold text-ink">
              受管整合包
            </h2>
            <span className="rounded-chip bg-accent px-1.5 py-0.5 text-[11px] font-bold text-paper-on">
              {status.subscription.pack_id}
            </span>
            {status.kind === "ready" && status.source === "cache" && (
              <span className="rounded-chip surface-sunken px-1.5 py-0.5 text-[11px] text-ink/75">
                上次成功缓存
              </span>
            )}
          </div>
          <p className="mt-1 truncate font-mono text-[11px] text-ink/75" title={status.subscription.pointer_url}>
            {status.subscription.pointer_url}
          </p>
        </div>

        {status.kind === "checking" ? (
          <Button variant="secondary" icon={<RefreshIcon size={15} />} disabled>
            正在检查
          </Button>
        ) : status.kind === "unavailable" ? (
          <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={onCheck}>
            重新检查
          </Button>
        ) : updateAvailable ? (
          <Button
            variant="primary"
            icon={<DownloadIcon size={16} />}
            disabled={updating}
            onClick={() => onSync(status.versions.latest.version)}
          >
            {updating ? "正在同步" : displayedVersions?.installed_version === null ? "开始同步" : "更新整合包"}
          </Button>
        ) : (
          <Button variant="secondary" icon={<RefreshIcon size={15} />} disabled={updating} onClick={onCheck}>
            检查更新
          </Button>
        )}
      </div>

      <div className="mt-4 flex gap-5 border-y border-ink/9 py-3.5">
        <VersionValue label="当前版本" value={displayedVersions?.installed_version ?? null} />
        <span className="w-px shrink-0 bg-ink/9" aria-hidden="true" />
        <VersionValue label="可用版本" value={displayedVersions?.latest.version ?? null} />
      </div>

      {displayedVersions?.latest.note && <p className="mt-3 text-[12.5px] leading-relaxed text-ink/75">{displayedVersions.latest.note}</p>}

      {status.kind === "unavailable" && (
        <div className="mt-4 flex items-start gap-3 rounded-panel border border-danger/35 px-3.5 py-3 text-danger" role="alert">
          <AlertIcon size={17} className="mt-0.5 shrink-0" />
          <div className="min-w-0">
            <p className="text-[13px] font-bold">暂时无法检查可用版本</p>
            <p className="mt-1 text-[12px] leading-relaxed break-words">{status.detail}</p>
            {status.last_known && <p className="mt-1.5 text-[11.5px] text-ink/75">上次成功缓存仍可用于展示；现有实例可以继续启动。</p>}
          </div>
        </div>
      )}

      {sync.kind === "running" && <ModpackSyncProgressView progress={sync.progress} />}
      {sync.kind === "failed" && (
        <>
          <ModpackSyncFailureView failure={sync.failure} />
          <div className="mt-3">
            {sync.failure.kind === "conflict" ? (
              // 措辞是单实例收敛后校准过的：这颗按钮不会「新增一个与旧实例并存的实例」——
              // 装出来的新实例会顶掉 config.selected_version 成为启动器认的那一个，
              // 旧目录留在磁盘上但从此没有任何界面能进它。承诺并存会让人以为随时切得回去，
              // 所以按钮只说「装新版本」，代价另起一行明说。
              <>
                <Button
                  variant="primary"
                  icon={<DownloadIcon size={16} />}
                  onClick={onInstallNewVersion}
                >
                  安装新版本
                </Button>
                <p className="mt-2 text-[12px] leading-relaxed text-ink/75">
                  装好之后启动器改认新实例。当前实例{" "}
                  <span className="font-mono font-bold">{instanceId}</span>{" "}
                  的目录（含存档与自装 mod）会原样留在游戏目录里，但不再出现在启动器中；需要腾出空间时请手动删除。
                </p>
              </>
            ) : sync.failure.kind === "invalid_metadata" ||
              sync.failure.kind === "launcher_too_old" ? (
              <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={onCheck}>
                重新检查
              </Button>
            ) : (
              <Button variant="secondary" icon={<RefreshIcon size={15} />} onClick={() => onSync(sync.target_version)}>
                重试同步
              </Button>
            )}
          </div>
        </>
      )}
      {sync.kind === "complete" && (
        <div className="mt-4 flex items-center gap-2.5 rounded-panel border border-ink/12 px-3.5 py-3 text-[13px] text-ink/75" role="status">
          <CheckIcon size={16} />
          已同步到 <span className="font-mono font-bold">{sync.installed_version}</span>
        </div>
      )}

      <p className="mt-4 text-[11.5px] leading-relaxed text-ink/75">
        该实例的受管 Mod 由整合包统一维护，单独的平台更新检查已停用，避免产生无法进入服务器的版本偏差。
      </p>
      {target && sync.kind === "idle" && !updateAvailable && (
        <p className="mt-1 text-[11.5px] text-ink/75">当前已是可用版本 {target}。</p>
      )}
    </section>
  );
}

export function ModpackFileOwnership({ owner }: { owner: ModpackFileOwner }) {
  const managed = owner === "modpack";
  return (
    <span
      className={`shrink-0 rounded-chip px-1.5 py-0.5 text-[11px] font-semibold ${
        managed ? "bg-accent text-paper-on" : "surface-sunken text-ink/75"
      }`}
      title={managed ? "由整合包统一维护，不能单独移除" : "玩家自行安装，可按普通 Mod 管理"}
    >
      {managed ? "整合包提供" : "玩家自装"}
    </span>
  );
}

export type ModpackFileManagementProps =
  | { owner: "modpack" }
  | { owner: "player"; removing: boolean; onRemove: () => void };

/** 受管文件的分支没有删除回调，因此调用方无法误画一个无效的删除按钮。 */
export function ModpackFileManagement(props: ModpackFileManagementProps) {
  return (
    <div className="flex items-center gap-2.5">
      <ModpackFileOwnership owner={props.owner} />
      {props.owner === "player" && (
        <Button variant="secondary" disabled={props.removing} onClick={props.onRemove}>
          {props.removing ? "正在移除" : "移除"}
        </Button>
      )}
    </div>
  );
}
