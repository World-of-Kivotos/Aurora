//! 模组安装到实例 + 本地模组管理。
//!
//! 门面把 aurora-modplatform 的双平台客户端（Modrinth / CurseForge）与本地 `mods/` 目录管理串起来。
//! 安装不是「下一个文件」这么简单：一次安装是一份计划（玩家选的那个 Mod + 它的必需前置），整份计划
//! 要么全部落盘、要么一个文件都不落。为此走三段式——先全部下到 `<工作目录>/.aurora/staging/`，逐个
//! 校验完整性契约，全过之后才原子移进 `mods/`。半套安装比装不上更难排查：玩家拿到的是一个「装了却
//! 进不去」的实例，而缺的那一半没有任何记录可查。
//!
//! 落盘之后、报成功之前必须写卷宗（[`crate::ledger`]）与变更历史（[`crate::history`]）。这个时机是
//! 硬要求：卷宗是 Mod 身份的唯一来源，历史是回滚的唯一凭据，报成功却没写等于凭空造出一批查无此人的
//! jar，更新检查、回滚、崩溃归因会同时失灵。
//!
//! 目标文件名已被同一工程的旧版本占着时走更新路径：旧文件就地改名为 `<旧文件名>.old` 留在 `mods/`，
//! 新文件移入，历史记 [`HistoryEvent::Update`]。那个 `.old` 就是回滚的全部凭据（见
//! [`Aurora::rollback`]），没有它「更新」就是一条单行道。
//!
//! 实例的 mods 目录不是固定的 `.minecraft/mods`——它随版本隔离判定走：隔离版本落到
//! `versions/<id>/mods`，共享版本落到 `.minecraft/mods`。判定统一取自 [`Aurora::resolve_working_dir`]
//! （与启动链路同源，含版本级隔离覆盖），避免「装进去的 mod 启动时不生效」。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use aurora_download::DownloadTask;
use aurora_instance::AURORA_META_DIR;
use aurora_modplatform::{
    CurseForgeClient, InstalledMod, ModrinthClient, Platform, disable_mod, enable_mod,
    parse_mod_metadata, scan_mods_dir,
};

use crate::deps::PlannedItem;
use crate::error::{CoreError, Result};
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::Aurora;
use crate::history::HistoryEvent;
use crate::ledger::{Ledger, LedgerEntry};

/// 实例工作目录下的模组目录名。
const MODS_DIR: &str = "mods";

/// 暂存目录名，位于 `<工作目录>/.aurora/staging`。
///
/// 与 mods 目录同处一个工作目录之下，因此「暂存 -> mods」必定是同卷改名，是真原子的，不会出现移到
/// 一半的半截文件。
const STAGING_DIR: &str = "staging";

/// 更新时旧文件的备份后缀。与 [`crate::history`] 的回滚约定同一份口径，改这里必须同步改那里。
const BACKUP_SUFFIX: &str = ".old";

/// 禁用态模组的文件名后缀。
///
/// aurora_modplatform 里的同名常量未导出，这里按同一约定复述：卷宗的 join 键是启用态文件名，磁盘上
/// 带该后缀的文件仍然算「装了」，只是没被加载器读。
const DISABLED_SUFFIX: &str = ".disabled";

/// 被顶替的文件在卷宗里查无来源时，更新事件里记下的旧版本。
///
/// 手动丢进 `mods/` 的 jar 没有任何版本信息可继承。写一个明确的「未知」而不是编一个版本号——历史是
/// 证据，编造的证据比没有更坏。
const UNKNOWN_VERSION: &str = "未知";

/// 一次模组安装的结果。
///
/// 描述的是玩家主动选的那个 Mod。本次可能连带装了若干必需前置，它们的落盘明细记在变更历史的
/// [`HistoryEvent::Install`] 里，而不是塞进本结构——调用方要的是「我点的那个装到哪了」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModInstallOutcome {
    /// 落盘的模组文件名。
    pub file_name: String,
    /// 模组文件完整路径。
    pub path: PathBuf,
    /// 来源平台。
    pub platform: Platform,
}

/// 一个进入下载队列的计划项：计划里的原始信息 + 它在暂存目录的落点与完整性契约。
struct StagedItem<'a> {
    /// 计划里的原始项（携带 `required_by` 与版本身份）。
    item: &'a PlannedItem,
    /// 平台在下载时给出的文件名，也是最终落进 `mods/` 的名字。
    file_name: String,
    /// 下载任务，`dest` 指向暂存目录。
    task: DownloadTask,
}

/// 被本次安装顶替掉的旧文件。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Replaced {
    /// 旧文件的启用态文件名（卷宗的 join 键）。
    old_file: String,
    /// 旧文件对应的平台版本标识；卷宗里查不到时为 [`UNKNOWN_VERSION`]。
    from_version: String,
}

/// 一个文件真正落进 `mods/` 之后的事实，供随后写卷宗与历史。
struct Placement {
    file_name: String,
    platform: Platform,
    project_id: String,
    version_id: String,
    sha1: Option<String>,
    /// 因谁被带进来（project_id）；玩家主动装的为 `None`。
    required_by: Option<String>,
    /// 走更新路径时被备份掉的旧文件；新装为 `None`。
    replaced: Option<Replaced>,
}

impl Aurora {
    /// 按安装计划把某平台上的一个模组版本（连同它的必需前置）安装到指定实例的 mods 目录。
    ///
    /// `project_id` / `mod_version_id` 的语义随平台而定：Modrinth 为工程 id/slug 与版本 id；CurseForge
    /// 为数字 modId 与 fileId（以十进制字符串传入）。
    ///
    /// 流程：[`Aurora::plan_install`] 算出计划 -> 全部文件下到暂存目录 -> 逐个校验 sha1/大小 ->
    /// 全过才原子移入 `mods/` -> 写卷宗与变更历史 -> 报成功。校验没全过就一个文件都不移，清掉暂存并
    /// 冒泡错误；下载失败会冒泡为 [`CoreError::Download`]，不静默。
    ///
    /// 计划里 `already_satisfied` 的项直接跳过：同工程同版本的文件已经躺在 `mods/` 里，重下一份只会
    /// 得到一模一样的字节。它在卷宗里的那条记录原样保留，身份不动。
    pub async fn install_mod(
        &self,
        version_id: &str,
        platform: Platform,
        project_id: &str,
        mod_version_id: &str,
        events: Option<&EventSink>,
    ) -> Result<ModInstallOutcome> {
        emit(
            events,
            CoreEvent::stage(format!("准备从 {} 安装模组到 {version_id}", platform.display_name())),
        );

        // 计划先行：这一步会校验版本确已安装（未装则在触网前短路）、展开必需依赖、判定哪些其实已装。
        let plan = self
            .plan_install(version_id, platform, project_id, mod_version_id)
            .await?;
        let items = plan.items;
        // plan_install 在结构上保证计划至少含主项。这里给错误而不是 panic：万一那条保证被后续改动
        // 打破，玩家该看到一句能读懂的失败，而不是整个命令被 panic 掀掉。
        let Some(main) = items.first() else {
            return Err(CoreError::ModVersionNotFound {
                platform: platform.display_name(),
                project_id: project_id.to_owned(),
                version_id: mod_version_id.to_owned(),
            });
        };

        // 工作目录只解析一次，mods 与暂存目录都从它派生：两处各解析一次就等于给隔离判定留了分叉的机会。
        let resolved = self.resolve_working_dir(version_id).await?;
        let mods_dir = resolved.working_dir.join(MODS_DIR);
        let staging_dir = resolved.working_dir.join(AURORA_META_DIR).join(STAGING_DIR);

        let pending: Vec<&PlannedItem> = items
            .iter()
            .filter(|item| !item.already_satisfied)
            .collect();
        emit(
            events,
            CoreEvent::stage(format!(
                "安装计划共 {} 项，其中 {} 个文件需要下载",
                items.len(),
                pending.len()
            )),
        );

        let mut staged: Vec<StagedItem<'_>> = Vec::with_capacity(pending.len());
        for &item in &pending {
            let (file_name, task) = self.staging_task(item, &staging_dir).await?;
            staged.push(StagedItem {
                item,
                file_name,
                task,
            });
        }
        ensure_unique_targets(&staged, &mods_dir)?;

        if let Err(err) = self.download_and_verify(&staged, events).await {
            discard_staging(&staged, &staging_dir, events).await;
            return Err(err);
        }

        // 落位决策全部先算清楚再动第一次盘：与已有文件的冲突必须在移动开始之前暴露，而不是移到一半
        // 才发现，那时候撤不回来。
        let ledger = self.ledger_store(version_id).load().await?;
        let mut decisions = Vec::with_capacity(staged.len());
        for item in &staged {
            decisions.push(plan_placement(&ledger, &mods_dir, item).await?);
        }

        tokio::fs::create_dir_all(&mods_dir)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: mods_dir.clone(),
                source,
            })?;

        let mut placements: Vec<Placement> = Vec::with_capacity(staged.len());
        let mut place_failure: Option<CoreError> = None;
        for (item, replaced) in staged.iter().zip(decisions) {
            match place_file(&mods_dir, item, replaced).await {
                Ok(placement) => {
                    emit(events, CoreEvent::stage(describe_placement(&placement)));
                    placements.push(placement);
                }
                Err(err) => {
                    place_failure = Some(err);
                    break;
                }
            }
        }

        // 落位即便中途失败（Windows 上游戏正在跑会把 jar 锁住），已经躺进 mods 的文件也必须立刻拿到
        // 身份：磁盘上多出一批查不到来源的 jar，比这次安装失败本身糟得多。
        let recorded = self.record_placements(version_id, &placements).await;
        discard_staging(&staged, &staging_dir, events).await;

        if let Some(err) = place_failure {
            if let Err(record_err) = recorded {
                emit(
                    events,
                    CoreEvent::warning(format!("已落盘文件的卷宗/历史写入同样失败：{record_err}")),
                );
            }
            return Err(err);
        }
        recorded?;

        // 主项本次真下载了就用它实际落盘的名字；已满足则用计划里的名字——那就是磁盘上那份的名字。
        let main_file = match staged.iter().find(|s| s.item.required_by.is_none()) {
            Some(staged_main) => staged_main.file_name.clone(),
            None => main.version.file_name.clone(),
        };
        let dest = mods_dir.join(&main_file);
        emit(
            events,
            CoreEvent::stage(format!("模组 {main_file} 已安装到 {}", dest.display())),
        );
        Ok(ModInstallOutcome {
            file_name: main_file,
            path: dest,
            platform,
        })
    }

    /// 扫描指定实例的 mods 目录，列出已装模组（含禁用态）。
    ///
    /// 目录尚不存在（该版本从未装过模组）返回空列表——这是「零已装模组」的正常态，而非错误；真实
    /// IO 故障（如权限不足）仍向上冒泡。
    pub async fn list_mods(&self, version_id: &str) -> Result<Vec<InstalledMod>> {
        let mods_dir = self.resolve_mods_dir(version_id).await?;
        let exists = tokio::fs::try_exists(&mods_dir)
            .await
            .map_err(|source| aurora_base::Error::Io {
                path: mods_dir.clone(),
                source,
            })?;
        if !exists {
            return Ok(Vec::new());
        }
        Ok(scan_mods_dir(&mods_dir).await?)
    }

    /// 启用或禁用指定实例里的某个模组，返回切换后的新路径。
    ///
    /// `file_name` 应为 [`Aurora::list_mods`] 返回的磁盘文件名（禁用态带 `.disabled` 后缀）。启禁以
    /// 文件重命名实现；目标名已存在（同名启用/禁用副本冲突）会冒泡 [`aurora_modplatform`] 的冲突错误。
    pub async fn set_mod_enabled(
        &self,
        version_id: &str,
        file_name: &str,
        enabled: bool,
    ) -> Result<PathBuf> {
        let mods_dir = self.resolve_mods_dir(version_id).await?;
        let path = mods_dir.join(file_name);
        let switched = if enabled {
            enable_mod(&path).await?
        } else {
            disable_mod(&path).await?
        };
        Ok(switched)
    }

    /// 解析某已安装版本对应的实例 mods 目录（`<工作目录>/mods`）。
    ///
    /// 工作目录一律取自 [`Aurora::resolve_working_dir`]——与启动链路同一个函数、同一份版本级隔离覆盖，
    /// 所以「装进去的 mod 启动时一定被读到」这条不变量由结构保证，而不是靠两处各自维护相同的参数。
    /// 版本本地未安装返回 [`CoreError::VersionNotInstalled`]。
    pub(crate) async fn resolve_mods_dir(&self, version_id: &str) -> Result<PathBuf> {
        let resolved = self.resolve_working_dir(version_id).await?;
        Ok(resolved.working_dir.join(MODS_DIR))
    }

    /// 为一个计划项生成下载任务，落点在暂存目录。
    async fn staging_task(
        &self,
        item: &PlannedItem,
        staging_dir: &Path,
    ) -> Result<(String, DownloadTask)> {
        // 平台按计划项自身的来源分发，而不是按入参平台：依赖与主项理论上同源，但把这条隐含前提写死
        // 在分发逻辑里，将来支持跨平台依赖时会静默走错客户端。
        let version = &item.version;
        match version.platform {
            Platform::Modrinth => {
                self.modrinth_task(&version.project_id, &version.version_id, staging_dir)
                    .await
            }
            Platform::CurseForge => {
                self.curseforge_task(&version.project_id, &version.version_id, staging_dir)
                    .await
            }
        }
    }

    /// 整批下载到暂存目录，再逐个核对完整性契约。任一环节失败即整体失败。
    async fn download_and_verify(
        &self,
        staged: &[StagedItem<'_>],
        events: Option<&EventSink>,
    ) -> Result<()> {
        let tasks: Vec<DownloadTask> = staged.iter().map(|s| s.task.clone()).collect();
        let report = self.download_pool().download_all(tasks, None).await?;
        // 有失败即冒泡其最终错误（重试换源后仍失败），绝不当作成功。
        if let Some(failure) = report.failures.into_iter().next() {
            return Err(failure.error.into());
        }

        let total = staged.len();
        for (index, item) in staged.iter().enumerate() {
            verify_staged(&item.task).await?;
            emit(
                events,
                CoreEvent::stage(format!(
                    "({}/{total}) {} 下载完成并通过校验",
                    index + 1,
                    item.file_name
                )),
            );
        }
        Ok(())
    }

    /// 把已落盘的文件写进卷宗与变更历史。
    ///
    /// 顺序是先卷宗后历史：卷宗决定「这个 jar 是谁」，历史只描述「发生过什么」。万一写到一半断电，
    /// 有身份没历史还能靠更新检查自愈，有历史没身份则连是哪个工程都说不出。
    async fn record_placements(&self, version_id: &str, placements: &[Placement]) -> Result<()> {
        if placements.is_empty() {
            return Ok(());
        }
        let at = now_unix();

        let store = self.ledger_store(version_id);
        let mut ledger = store.load().await?;
        for placement in placements {
            // 改名式更新：旧文件名那条记录必须先摘掉，否则卷宗里会同时挂着新旧两个文件名，而磁盘上
            // 旧的那个已经变成 .old 备份了。
            if let Some(replaced) = &placement.replaced
                && replaced.old_file != placement.file_name
            {
                ledger.remove(&replaced.old_file);
            }
            ledger.upsert(LedgerEntry {
                file_name: placement.file_name.clone(),
                platform: placement.platform,
                project_id: placement.project_id.clone(),
                version_id: placement.version_id.clone(),
                sha1: placement.sha1.clone(),
                installed_at: at,
                installed_as_dependency_of: placement.required_by.clone(),
            });
        }
        store.save(&ledger).await?;

        let fresh: Vec<String> = placements
            .iter()
            .filter(|p| p.replaced.is_none())
            .map(|p| p.file_name.clone())
            .collect();
        if !fresh.is_empty() {
            self.append_event(version_id, at, |id| HistoryEvent::Install {
                id,
                at,
                files: fresh,
            })
            .await?;
        }
        // 更新逐条单记：回滚要的是「哪个文件从哪个版本换到了哪个版本」，一条批量事件表达不了这个。
        for placement in placements {
            let Some(replaced) = &placement.replaced else {
                continue;
            };
            self.append_event(version_id, at, |id| HistoryEvent::Update {
                id,
                at,
                file_name: placement.file_name.clone(),
                old_file: replaced.old_file.clone(),
                from_version: replaced.from_version.clone(),
                to_version: placement.version_id.clone(),
            })
            .await?;
        }
        Ok(())
    }

    /// 追加一条变更事件，事件 id 现取现用。
    ///
    /// 每条都重新读一遍历史再取 id：一次安装会连写好几条同秒事件，序号必须避开刚写进去的那几条。
    async fn append_event(
        &self,
        version_id: &str,
        at: u64,
        make: impl FnOnce(String) -> HistoryEvent,
    ) -> Result<()> {
        let store = self.history_store(version_id);
        let id = store.load().await?.next_event_id(at);
        store.append(make(id)).await
    }

    /// Modrinth：列出工程版本，取 `mod_version_id` 对应版本的主文件，生成下载任务。
    async fn modrinth_task(
        &self,
        project_id: &str,
        mod_version_id: &str,
        dest_dir: &Path,
    ) -> Result<(String, DownloadTask)> {
        let client = ModrinthClient::new(self.http()).with_base_url(self.modrinth_base());
        // Modrinth 无「按版本 id 单取」端点，故列出工程全部版本后按 id 精确匹配。
        let version = client
            .versions(project_id, &[], &[])
            .await?
            .into_iter()
            .find(|v| v.id == mod_version_id)
            .ok_or_else(|| CoreError::ModVersionNotFound {
                platform: Platform::Modrinth.display_name(),
                project_id: project_id.to_owned(),
                version_id: mod_version_id.to_owned(),
            })?;
        let file = version
            .primary_file()
            .ok_or_else(|| CoreError::ModVersionNotFound {
                platform: Platform::Modrinth.display_name(),
                project_id: project_id.to_owned(),
                version_id: mod_version_id.to_owned(),
            })?;
        let dest = safe_join(dest_dir, &file.filename)?;
        Ok((file.filename.clone(), file.to_download_task(dest)))
    }

    /// CurseForge：列出工程文件，取 `file_id` 对应文件，生成下载任务（downloadUrl 为空时走直链兜底）。
    async fn curseforge_task(
        &self,
        project_id: &str,
        mod_version_id: &str,
        dest_dir: &Path,
    ) -> Result<(String, DownloadTask)> {
        // CurseForge 的 id 是数字；非数字串不可能对应任何文件，直接判为未找到。
        let not_found = || CoreError::ModVersionNotFound {
            platform: Platform::CurseForge.display_name(),
            project_id: project_id.to_owned(),
            version_id: mod_version_id.to_owned(),
        };
        let mod_id: u32 = project_id.parse().map_err(|_| not_found())?;
        let file_id: u32 = mod_version_id.parse().map_err(|_| not_found())?;

        let client = CurseForgeClient::from_env(self.http())?.with_base_url(self.curseforge_base());
        let file = client
            .mod_files(mod_id, None, None)
            .await?
            .into_iter()
            .find(|f| f.id == file_id)
            .ok_or_else(not_found)?;
        let dest = safe_join(dest_dir, &file.file_name)?;

        // 文件对象自带 downloadUrl 时直接用；为空则走 download-url 端点取直链兜底，并补上完整性契约。
        let task = match file.to_download_task(&dest) {
            Some(task) => task,
            None => {
                let url = client.file_download_url(mod_id, file_id).await?;
                let mut task = DownloadTask::new(url, &dest);
                if let Some(size) = file.file_length {
                    task = task.with_size(size);
                }
                if let Some(sha1) = file.sha1() {
                    task = task.with_sha1(sha1.to_string());
                }
                task
            }
        };
        Ok((file.file_name.clone(), task))
    }
}

/// 把平台给的文件名拼到目录下，并挡住任何不是「纯文件名」的东西。
///
/// 文件名来自第三方平台的响应，这一个串随后要被用来拼暂存路径、`mods/` 落点与 `.old` 备份名。带路径
/// 分隔符（或形如 `..`）的名字会让这三处一起写到目录外去，故在唯一入口处一次挡掉，而不是每处各防一遍。
fn safe_join(dir: &Path, file_name: &str) -> Result<PathBuf> {
    if Path::new(file_name).file_name().and_then(|n| n.to_str()) != Some(file_name) {
        return Err(refuse(
            dir,
            std::io::ErrorKind::InvalidInput,
            format!("平台给出的文件名 {file_name} 不是合法的纯文件名，已拒绝安装"),
        ));
    }
    Ok(dir.join(file_name))
}

/// 计划里的多个文件不得写向同一个落点。
///
/// 两个不同工程的 jar 同名时，后者会覆盖前者，报告却说两个都装好了——玩家得到一个「装了但没有」的
/// Mod。这种冲突在触盘之前就能看出来，那就在触盘之前拒绝。
fn ensure_unique_targets(staged: &[StagedItem<'_>], mods_dir: &Path) -> Result<()> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(staged.len());
    for item in staged {
        if !seen.insert(item.file_name.as_str()) {
            return Err(refuse(
                &mods_dir.join(&item.file_name),
                std::io::ErrorKind::AlreadyExists,
                format!(
                    "计划里有多个工程都要写入同一个文件 {}，无法同时安装",
                    item.file_name
                ),
            ));
        }
    }
    Ok(())
}

/// 校验暂存文件确实符合平台声明的完整性契约。
///
/// 下载引擎在合并落盘时已经校过一遍，这里再校一次不是复读：引擎校的是它自己的临时文件，而「能不能
/// 移进 mods」这个决定必须建立在「此刻暂存目录里躺着的就是对的东西」之上。平台两样都没给时，能验的
/// 只剩「不是个空壳」——0 字节的 jar 必然是错误页或断流，装进去只会在启动时报一句看不懂的异常。
async fn verify_staged(task: &DownloadTask) -> Result<()> {
    let meta = tokio::fs::metadata(&task.dest)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: task.dest.clone(),
            source,
        })?;
    if meta.len() == 0 {
        return Err(refuse(
            &task.dest,
            std::io::ErrorKind::InvalidData,
            format!("下载得到的文件是 0 字节，来源 {}", task.url),
        ));
    }
    if let Some(expected) = task.size
        && meta.len() != expected
    {
        return Err(aurora_download::Error::SizeMismatch {
            url: task.url.clone(),
            expected,
            actual: meta.len(),
        }
        .into());
    }
    if let Some(sha1) = &task.sha1 {
        aurora_base::fs::verify_sha1(&task.dest, sha1).await?;
    }
    Ok(())
}

/// 算出一个待落盘文件该怎么落：是新装，还是先把同工程的旧文件备份掉再覆上去。
async fn plan_placement(
    ledger: &Ledger,
    mods_dir: &Path,
    item: &StagedItem<'_>,
) -> Result<Option<Replaced>> {
    let version = &item.item.version;
    let replaced = find_previous(
        ledger,
        mods_dir,
        version.platform,
        &version.project_id,
        &item.file_name,
        &item.task.dest,
    )
    .await?;

    // 改名式更新时目标名本该是空的；被别的文件占着说明那是另一个来路不明的 jar，直接改名过去会让它
    // 无声消失（Windows 的 rename 覆盖同名文件不报错）。
    if let Some(previous) = &replaced
        && previous.old_file != item.file_name
        && present(mods_dir, &item.file_name).await?
    {
        return Err(refuse(
            &mods_dir.join(&item.file_name),
            std::io::ErrorKind::AlreadyExists,
            format!(
                "mods 目录里已有文件 {}，它不属于本次要更新的 {}，为避免覆盖已中止安装",
                item.file_name, previous.old_file
            ),
        ));
    }
    Ok(replaced)
}

/// 在卷宗与磁盘里找这个工程已装的旧文件。
///
/// 先按卷宗里的平台 + 工程标识找：这是「同一个 Mod 的旧版本」最可靠的判据，文件名从 `sodium-0.5.jar`
/// 换成 `sodium-0.6.jar` 也照样认得出。只比 project_id 不够——两个平台的工程标识各自成命名空间，撞上
/// 就会把别人的 jar 当成旧版备份掉。
///
/// 卷宗认不出、但目标文件名已经被占时同样按「顶替」处理：那是玩家手动丢进来或老启动器装的文件，版本
/// 无从查考（记为 [`UNKNOWN_VERSION`]），但绝不能就这么覆盖掉。
///
/// 最后一道兜底比 jar 里的 `mod_id`：卷宗按「平台 + 工程」认，认不出跨来源的同一个 Mod——
/// 同一个 Sodium 从 Modrinth 装一次、再从 CurseForge 装一次，两个平台的工程 id 各自成命名空间，
/// 前两道都判「没装过」，于是两个 jar 共存，游戏抛 DuplicateModsFoundException。
/// 而 `mod_id` 正是加载器眼中的唯一身份，手动丢进来的 jar 也一样带着它。
async fn find_previous(
    ledger: &Ledger,
    mods_dir: &Path,
    platform: Platform,
    project_id: &str,
    new_file_name: &str,
    staged_file: &Path,
) -> Result<Option<Replaced>> {
    for entry in &ledger.entries {
        if entry.platform != platform || entry.project_id != project_id {
            continue;
        }
        // 磁盘是权威：卷宗有记录但文件被玩家删了，那就是没装，本次按新装处理。
        if present(mods_dir, &entry.file_name).await? {
            return Ok(Some(Replaced {
                old_file: entry.file_name.clone(),
                from_version: entry.version_id.clone(),
            }));
        }
    }
    if present(mods_dir, new_file_name).await? {
        return Ok(Some(Replaced {
            old_file: new_file_name.to_owned(),
            from_version: UNKNOWN_VERSION.to_owned(),
        }));
    }
    find_previous_by_mod_id(ledger, mods_dir, staged_file).await
}

/// 按 `mod_id` 在 mods 目录里找同一个 Mod 的另一份拷贝（跨平台、跨来源）。
///
/// 解析失败一律当作「认不出」而不是报错：这道兜底是额外收益，读不出元数据的 jar
/// （损坏包、非标准打包）不该让整次安装失败。
async fn find_previous_by_mod_id(
    ledger: &Ledger,
    mods_dir: &Path,
    staged_file: &Path,
) -> Result<Option<Replaced>> {
    let Some(new_id) = mod_id_of(staged_file).await else {
        return Ok(None);
    };

    // 目录不存在说明一个 Mod 都还没装，没什么可比的。
    let Ok(installed) = scan_mods_dir(mods_dir).await else {
        return Ok(None);
    };
    for item in installed {
        let Some(meta) = &item.metadata else {
            continue;
        };
        if meta.mod_id != new_id {
            continue;
        }
        // 卷宗里查得到就带上它的版本号，查不到（手动丢进来的）记未知，回滚仍能靠 .old 复原。
        let key = ledger_key(&item.file_name).to_owned();
        let from_version = ledger
            .find(&key)
            .map(|e| e.version_id.clone())
            .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());
        return Ok(Some(Replaced {
            old_file: key,
            from_version,
        }));
    }
    Ok(None)
}

/// 读一个 jar 的 `mod_id`；解析不出或没有元数据都返回 `None`。
async fn mod_id_of(path: &Path) -> Option<String> {
    parse_mod_metadata(path)
        .await
        .ok()
        .flatten()
        .map(|meta| meta.mod_id)
}

/// 取与卷宗 join 的键：卷宗记的是启用态文件名，磁盘上禁用态会多一个 `.disabled` 后缀。
fn ledger_key(file_name: &str) -> &str {
    file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name)
}

/// 把一个暂存文件移进 mods 目录，必要时先给旧文件留一份 `.old` 备份。
async fn place_file(
    mods_dir: &Path,
    item: &StagedItem<'_>,
    replaced: Option<Replaced>,
) -> Result<Placement> {
    if let Some(previous) = &replaced {
        backup_previous(mods_dir, &previous.old_file).await?;
    }
    let dest = mods_dir.join(&item.file_name);
    tokio::fs::rename(&item.task.dest, &dest)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: dest.clone(),
            source,
        })?;

    let version = &item.item.version;
    Ok(Placement {
        file_name: item.file_name.clone(),
        platform: version.platform,
        project_id: version.project_id.clone(),
        version_id: version.version_id.clone(),
        // 记下真正校验用的那个 sha1，而不是计划里的：它才是磁盘上这份字节的凭据。
        sha1: item.task.sha1.clone(),
        required_by: item.item.required_by.clone(),
        replaced,
    })
}

/// 把旧文件改名为 `<old_file>.old` 留在原目录，这是回滚的全部凭据。
///
/// 启用态与禁用态两种名字都试：玩家把旧版禁用了是常见操作，只认启用态会在 mods 里留下一份没人管的
/// `<old_file>.disabled`，下次更新检查还会拿它当一个独立的已装 Mod 去查。
async fn backup_previous(mods_dir: &Path, old_file: &str) -> Result<()> {
    let backup = mods_dir.join(format!("{old_file}{BACKUP_SUFFIX}"));
    for candidate in [
        mods_dir.join(old_file),
        mods_dir.join(format!("{old_file}{DISABLED_SUFFIX}")),
    ] {
        match tokio::fs::rename(&candidate, &backup).await {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(aurora_base::Error::Io {
                    path: candidate,
                    source,
                }
                .into());
            }
        }
    }
    // 判定阶段还看得见这个文件，真去改名时它没了：只可能是外部同时在动这个目录。这时候继续往下走会
    // 直接覆盖掉未知状态，不如停在这里。
    Err(refuse(
        &mods_dir.join(old_file),
        std::io::ErrorKind::NotFound,
        format!("待备份的旧文件 {old_file} 在备份前消失了，已中止安装"),
    ))
}

/// 文件是否在 mods 目录里（启用态或禁用态都算）。
async fn present(mods_dir: &Path, file_name: &str) -> Result<bool> {
    if try_exists(&mods_dir.join(file_name)).await? {
        return Ok(true);
    }
    try_exists(&mods_dir.join(format!("{file_name}{DISABLED_SUFFIX}"))).await
}

/// 存在性探测，IO 故障（权限不足等）如实冒泡，不当作「不存在」。
async fn try_exists(path: &Path) -> Result<bool> {
    tokio::fs::try_exists(path)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: path.to_owned(),
            source,
        })
        .map_err(Into::into)
}

/// 一个文件落位后发给 UI 的那句话。
fn describe_placement(placement: &Placement) -> String {
    match &placement.replaced {
        Some(previous) => format!(
            "{} 已更新，旧版 {} 备份为 {}{BACKUP_SUFFIX}",
            placement.file_name, previous.old_file, previous.old_file
        ),
        None => format!("{} 已放入 mods 目录", placement.file_name),
    }
}

/// 清理本次用到的暂存文件，并尝试删掉空的暂存目录。
///
/// 清理失败只发告警不冒泡：它既不该把「已经装好了」翻案成失败，也不该盖住真正的根因错误。暂存目录
/// 删不掉通常只说明另一次安装正在用它，不算异常，故不报。
async fn discard_staging(
    staged: &[StagedItem<'_>],
    staging_dir: &Path,
    events: Option<&EventSink>,
) {
    for item in staged {
        if let Err(err) = tokio::fs::remove_file(&item.task.dest).await
            && err.kind() != std::io::ErrorKind::NotFound
        {
            emit(
                events,
                CoreEvent::warning(format!(
                    "清理暂存文件 {} 失败：{err}",
                    item.task.dest.display()
                )),
            );
        }
    }
    let _ = tokio::fs::remove_dir(staging_dir).await;
}

/// 构造一条带中文原因的 IO 类错误。
///
/// `CoreError` 目前没有安装流程专用的变体，`error.rs` 也不在本次改动范围内，故借
/// [`aurora_base::Error::Io`] 承载：`path` 指向真正出问题的那个文件，完整原因挂在 `source` 上，不吞。
fn refuse(path: &Path, kind: std::io::ErrorKind, reason: String) -> CoreError {
    CoreError::Base(aurora_base::Error::Io {
        path: path.to_owned(),
        source: std::io::Error::new(kind, reason),
    })
}

/// 当前 Unix 秒。
///
/// 系统时钟早于 1970 时退化为 0：这是环境异常而非业务错误，事件 id 仍唯一且可排序，为它中断一次已经
/// 落完盘的安装得不偿失。口径与 `history.rs`、`launch.rs` 的同名函数一致。
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use aurora_instance::IsolationPolicy;
    use sha1::{Digest, Sha1};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 在 versions/<id>/<id>.json 落一份最小合法版本 JSON（正式原版，不装加载器）。
    async fn put_version(mc: &std::path::Path, id: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","type":"release","mainClass":"m"}}"#),
        )
        .await
        .unwrap();
    }

    fn sha1_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(bytes);
        hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 拼一条 Modrinth 版本 JSON。`deps` 原样填进 dependencies 数组，`hashes` 原样填进文件哈希对象。
    #[allow(clippy::too_many_arguments)]
    fn modrinth_version(
        id: &str,
        project: &str,
        date: &str,
        file_name: &str,
        url: &str,
        hashes: &str,
        size: usize,
        deps: &str,
    ) -> String {
        format!(
            r#"{{"id":"{id}","project_id":"{project}","name":"{project} {id}",
                "version_number":"{id}","version_type":"release","date_published":"{date}",
                "game_versions":["1.21"],"loaders":[],"dependencies":[{deps}],
                "files":[{{"hashes":{hashes},"url":"{url}","filename":"{file_name}",
                    "primary":true,"size":{size}}}]}}"#
        )
    }

    /// 挂一个工程版本列表端点。
    async fn mount_versions(server: &MockServer, project: &str, body: String) {
        Mock::given(method("GET"))
            .and(path(format!("/project/{project}/version")))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    /// 挂一个 jar 直链。
    async fn mount_jar(server: &MockServer, name: &str, bytes: Vec<u8>) {
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes))
            .mount(server)
            .await;
    }

    /// 造一个隔离档位为 All、已装 1.21 的测试门面。
    fn isolated_aurora(mc: &std::path::Path, base: String) -> Aurora {
        let mut aurora = Aurora::for_test(AuroraConfig::default(), mc.to_path_buf(), mc.to_path_buf());
        aurora.set_isolation_policy(IsolationPolicy::All);
        aurora.with_modrinth_base(base)
    }

    /// 全隔离档位下装 Modrinth 模组：文件落到 versions/<id>/mods（实例隔离目录），随后能扫出、可启禁。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_modrinth_mod_lands_in_isolated_mods_dir_then_lists_and_toggles() {
        let server = MockServer::start().await;
        let base = server.uri();
        let jar_bytes = b"sodium-jar-payload".to_vec();
        let sha1 = sha1_hex(&jar_bytes);

        // 工程版本列表：含目标版本 modver1，主文件 sodium.jar 走 mock 直链，带 sha1/大小契约。
        let versions_body = format!(
            r#"[{{"id":"modver1","project_id":"sodium","name":"Sodium 0.5",
                "version_number":"0.5","version_type":"release",
                "date_published":"2026-01-01T00:00:00Z",
                "files":[{{"hashes":{{"sha1":"{sha1}"}},"url":"{base}/sodium.jar",
                    "filename":"sodium.jar","primary":true,"size":{}}}]}}]"#,
            jar_bytes.len()
        );
        mount_versions(&server, "sodium", versions_body).await;
        mount_jar(&server, "sodium.jar", jar_bytes.clone()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let outcome = aurora
            .install_mod("1.21", Platform::Modrinth, "sodium", "modver1", Some(&tx))
            .await
            .unwrap();

        // 隔离档位 All -> 工作目录进版本文件夹 -> mods 落 versions/1.21/mods。
        let expected = mc.join("versions").join("1.21").join("mods").join("sodium.jar");
        assert_eq!(outcome.file_name, "sodium.jar");
        assert_eq!(outcome.path, expected);
        assert_eq!(outcome.platform, Platform::Modrinth);
        // 文件确已落盘且内容一致。
        assert_eq!(tokio::fs::read(&expected).await.unwrap(), jar_bytes);
        // 暂存目录已清空。
        assert!(
            !tokio::fs::try_exists(mc.join("versions").join("1.21").join(".aurora").join("staging"))
                .await
                .unwrap()
        );

        // list_mods 能扫出这枚模组，且为启用态。
        let listed = aurora.list_mods("1.21").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, "sodium.jar");
        assert!(listed[0].enabled);

        // 禁用：文件重命名为 .disabled 后缀，原文件消失。
        let disabled = aurora.set_mod_enabled("1.21", "sodium.jar", false).await.unwrap();
        assert_eq!(disabled.file_name().unwrap(), "sodium.jar.disabled");
        assert!(!tokio::fs::try_exists(&expected).await.unwrap());
        assert!(tokio::fs::try_exists(&disabled).await.unwrap());
        let listed = aurora.list_mods("1.21").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].file_name, "sodium.jar.disabled");
        assert!(!listed[0].enabled);

        // 重新启用：回到原文件名。
        let enabled = aurora
            .set_mod_enabled("1.21", "sodium.jar.disabled", true)
            .await
            .unwrap();
        assert_eq!(enabled, expected);
        assert!(tokio::fs::try_exists(&expected).await.unwrap());

        // 至少发出「已安装」阶段事件。
        drop(tx);
        let mut stages = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let CoreEvent::Stage(s) = ev {
                stages.push(s);
            }
        }
        assert!(stages.iter().any(|s| s.contains("模组 sodium.jar 已安装")));
        assert!(stages.iter().any(|s| s.contains("(1/1) sodium.jar 下载完成并通过校验")));
    }

    /// 共享档位（关闭隔离）下装模组：文件落到 .minecraft/mods 根，验证 mods 目录随隔离策略切换。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_lands_in_shared_root_mods_when_isolation_disabled() {
        let server = MockServer::start().await;
        let base = server.uri();
        let jar_bytes = b"lithium-jar".to_vec();
        let sha1 = sha1_hex(&jar_bytes);
        let versions_body = format!(
            r#"[{{"id":"v1","project_id":"lithium","name":"Lithium","version_number":"0.11",
                "version_type":"release","date_published":"2026-01-01T00:00:00Z",
                "files":[{{"hashes":{{"sha1":"{sha1}"}},"url":"{base}/lithium.jar",
                    "filename":"lithium.jar","primary":true,"size":{}}}]}}]"#,
            jar_bytes.len()
        );
        mount_versions(&server, "lithium", versions_body).await;
        mount_jar(&server, "lithium.jar", jar_bytes.clone()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;

        // 关闭隔离：正式原版共享 .minecraft 根 -> mods 落 .minecraft/mods。
        let mut aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc.clone());
        aurora.set_isolation_policy(IsolationPolicy::Disabled);
        let aurora = aurora.with_modrinth_base(base);

        let outcome = aurora
            .install_mod("1.21", Platform::Modrinth, "lithium", "v1", None)
            .await
            .unwrap();
        assert_eq!(outcome.path, mc.join("mods").join("lithium.jar"));
        assert_eq!(tokio::fs::read(&outcome.path).await.unwrap(), jar_bytes);
    }

    /// 未安装的版本：install_mod 在触网前就冒泡 VersionNotInstalled。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_into_uninstalled_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc);

        let err = aurora
            .install_mod("ghost", Platform::Modrinth, "sodium", "modver1", None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::VersionNotInstalled { id } if id == "ghost"));
    }

    /// 平台上找不到请求的版本 id：冒泡 ModVersionNotFound。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_missing_platform_version_errors() {
        let server = MockServer::start().await;
        // 工程存在但版本列表里没有请求的 id。
        mount_versions(&server, "sodium", "[]".to_owned()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc)
            .with_modrinth_base(server.uri());

        let err = aurora
            .install_mod("1.21", Platform::Modrinth, "sodium", "does-not-exist", None)
            .await
            .unwrap_err();
        match err {
            CoreError::ModVersionNotFound {
                project_id,
                version_id,
                ..
            } => {
                assert_eq!(project_id, "sodium");
                assert_eq!(version_id, "does-not-exist");
            }
            other => panic!("期望 ModVersionNotFound，得到 {other:?}"),
        }
    }

    /// 从未装过模组的版本：list_mods 返回空列表而非报错。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_mods_on_version_without_mods_dir_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc);

        let listed = aurora.list_mods("1.21").await.unwrap();
        assert!(listed.is_empty());
    }

    /// 必需依赖被一并装入，且卷宗为每个文件记下正确身份（含「因谁而来」）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn required_dependency_is_installed_alongside_and_recorded_in_ledger() {
        let server = MockServer::start().await;
        let base = server.uri();
        let main_bytes = b"sodium-payload".to_vec();
        let dep_bytes = b"fabric-api-payload-longer".to_vec();
        let main_sha1 = sha1_hex(&main_bytes);
        let dep_sha1 = sha1_hex(&dep_bytes);

        mount_versions(
            &server,
            "sodium",
            format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "sodium",
                    "2026-01-02T00:00:00Z",
                    "sodium.jar",
                    &format!("{base}/sodium.jar"),
                    &format!(r#"{{"sha1":"{main_sha1}"}}"#),
                    main_bytes.len(),
                    r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
                )
            ),
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            format!(
                "[{}]",
                modrinth_version(
                    "fv1",
                    "fabric-api",
                    "2026-01-01T00:00:00Z",
                    "fabric-api.jar",
                    &format!("{base}/fabric-api.jar"),
                    &format!(r#"{{"sha1":"{dep_sha1}"}}"#),
                    dep_bytes.len(),
                    "",
                )
            ),
        )
        .await;
        mount_jar(&server, "sodium.jar", main_bytes.clone()).await;
        mount_jar(&server, "fabric-api.jar", dep_bytes.clone()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        let before = now_unix();
        let outcome = aurora
            .install_mod("1.21", Platform::Modrinth, "sodium", "v1", None)
            .await
            .unwrap();
        let after = now_unix();

        let mods = mc.join("versions").join("1.21").join("mods");
        assert_eq!(outcome.file_name, "sodium.jar");
        assert_eq!(
            tokio::fs::read(mods.join("sodium.jar")).await.unwrap(),
            main_bytes
        );
        assert_eq!(
            tokio::fs::read(mods.join("fabric-api.jar")).await.unwrap(),
            dep_bytes
        );

        let ledger = aurora.ledger_store("1.21").load().await.unwrap();
        assert_eq!(ledger.entries.len(), 2);
        let main_entry = ledger.find("sodium.jar").expect("卷宗应有主项记录");
        assert_eq!(main_entry.platform, Platform::Modrinth);
        assert_eq!(main_entry.project_id, "sodium");
        assert_eq!(main_entry.version_id, "v1");
        assert_eq!(main_entry.sha1.as_deref(), Some(main_sha1.as_str()));
        assert_eq!(main_entry.installed_as_dependency_of, None);
        assert!(main_entry.installed_at >= before && main_entry.installed_at <= after);

        let dep_entry = ledger.find("fabric-api.jar").expect("卷宗应有依赖记录");
        assert_eq!(dep_entry.project_id, "fabric-api");
        assert_eq!(dep_entry.version_id, "fv1");
        assert_eq!(dep_entry.sha1.as_deref(), Some(dep_sha1.as_str()));
        assert_eq!(
            dep_entry.installed_as_dependency_of.as_deref(),
            Some("sodium")
        );
    }

    /// 安装追加一条 Install 事件，files 列全本次落盘的每一个文件（主项在前，依赖在后）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn install_appends_history_event_listing_every_landed_file() {
        let server = MockServer::start().await;
        let base = server.uri();
        let main_bytes = b"main-jar".to_vec();
        let dep_bytes = b"dep-jar".to_vec();

        mount_versions(
            &server,
            "carpet",
            format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "carpet",
                    "2026-02-02T00:00:00Z",
                    "carpet.jar",
                    &format!("{base}/carpet.jar"),
                    &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&main_bytes)),
                    main_bytes.len(),
                    r#"{"project_id":"cloth","dependency_type":"required"}"#,
                )
            ),
        )
        .await;
        mount_versions(
            &server,
            "cloth",
            format!(
                "[{}]",
                modrinth_version(
                    "cv1",
                    "cloth",
                    "2026-02-01T00:00:00Z",
                    "cloth.jar",
                    &format!("{base}/cloth.jar"),
                    &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&dep_bytes)),
                    dep_bytes.len(),
                    "",
                )
            ),
        )
        .await;
        mount_jar(&server, "carpet.jar", main_bytes).await;
        mount_jar(&server, "cloth.jar", dep_bytes).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        aurora
            .install_mod("1.21", Platform::Modrinth, "carpet", "v1", None)
            .await
            .unwrap();

        let history = aurora.history("1.21").await.unwrap();
        assert_eq!(history.events.len(), 1);
        match &history.events[0] {
            HistoryEvent::Install { id, at, files } => {
                assert_eq!(files, &vec!["carpet.jar".to_owned(), "cloth.jar".to_owned()]);
                assert!(*at > 0);
                assert_eq!(id, &format!("{at}-001"));
            }
            other => panic!("期望 Install 事件，得到 {other:?}"),
        }
    }

    /// 依赖校验不过（服务端给的字节与声明的 sha1 不符）：mods 目录一个文件都不多，卷宗与历史都不写。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn integrity_failure_leaves_mods_dir_empty_and_writes_nothing() {
        let server = MockServer::start().await;
        let base = server.uri();
        let main_bytes = b"good-main-jar".to_vec();
        let dep_bytes = b"promised-dep-jar".to_vec();

        mount_versions(
            &server,
            "rei",
            format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "rei",
                    "2026-03-02T00:00:00Z",
                    "rei.jar",
                    &format!("{base}/rei.jar"),
                    &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&main_bytes)),
                    main_bytes.len(),
                    r#"{"project_id":"arch","dependency_type":"required"}"#,
                )
            ),
        )
        .await;
        mount_versions(
            &server,
            "arch",
            format!(
                "[{}]",
                modrinth_version(
                    "av1",
                    "arch",
                    "2026-03-01T00:00:00Z",
                    "arch.jar",
                    &format!("{base}/arch.jar"),
                    // 声明的是 dep_bytes 的 sha1 与长度，实际发的是另一串同长度字节。
                    &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&dep_bytes)),
                    dep_bytes.len(),
                    "",
                )
            ),
        )
        .await;
        mount_jar(&server, "rei.jar", main_bytes).await;
        mount_jar(&server, "arch.jar", b"tampered-dep-jar".to_vec()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        let err = aurora
            .install_mod("1.21", Platform::Modrinth, "rei", "v1", None)
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::Download(_)), "得到 {err:?}");

        // 主项明明下载成功，也一样不许进 mods：全过才移，是这条流程的全部意义。
        assert!(aurora.list_mods("1.21").await.unwrap().is_empty());
        assert!(aurora.ledger_store("1.21").load().await.unwrap().entries.is_empty());
        assert!(aurora.history("1.21").await.unwrap().events.is_empty());
        // 暂存目录连同里面的残片一起清掉。
        let staging = mc.join("versions").join("1.21").join(".aurora").join("staging");
        assert!(!tokio::fs::try_exists(&staging).await.unwrap());
    }

    /// 平台既没给 sha1、声明大小又是 0：0 字节的 jar 被挡在 mods 之外。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zero_byte_payload_is_rejected_before_touching_mods_dir() {
        let server = MockServer::start().await;
        let base = server.uri();

        mount_versions(
            &server,
            "ghostmod",
            format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "ghostmod",
                    "2026-04-01T00:00:00Z",
                    "ghostmod.jar",
                    &format!("{base}/ghostmod.jar"),
                    "{}",
                    0,
                    "",
                )
            ),
        )
        .await;
        mount_jar(&server, "ghostmod.jar", Vec::new()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        let err = aurora
            .install_mod("1.21", Platform::Modrinth, "ghostmod", "v1", None)
            .await
            .unwrap_err();
        match err {
            CoreError::Base(aurora_base::Error::Io { source, .. }) => {
                assert_eq!(source.kind(), std::io::ErrorKind::InvalidData);
                assert!(source.to_string().contains("0 字节"), "{source}");
            }
            other => panic!("期望 0 字节被拒，得到 {other:?}"),
        }
        assert!(aurora.list_mods("1.21").await.unwrap().is_empty());
    }

    /// 装同一工程的更高版本：旧文件改名为 .old 留下，历史记 Update，卷宗只剩新文件那一条，且可回滚。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn installing_newer_version_backs_up_old_file_and_records_update() {
        let server = MockServer::start().await;
        let base = server.uri();
        let old_bytes = b"iris-1.6.jar-bytes".to_vec();
        let new_bytes = b"iris-1.7.jar-bytes-longer".to_vec();

        let body = format!(
            "[{},{}]",
            modrinth_version(
                "v2",
                "iris",
                "2026-05-02T00:00:00Z",
                "iris-1.7.jar",
                &format!("{base}/iris-1.7.jar"),
                &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&new_bytes)),
                new_bytes.len(),
                "",
            ),
            modrinth_version(
                "v1",
                "iris",
                "2026-05-01T00:00:00Z",
                "iris-1.6.jar",
                &format!("{base}/iris-1.6.jar"),
                &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&old_bytes)),
                old_bytes.len(),
                "",
            )
        );
        mount_versions(&server, "iris", body).await;
        mount_jar(&server, "iris-1.6.jar", old_bytes.clone()).await;
        mount_jar(&server, "iris-1.7.jar", new_bytes.clone()).await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        aurora
            .install_mod("1.21", Platform::Modrinth, "iris", "v1", None)
            .await
            .unwrap();
        aurora
            .install_mod("1.21", Platform::Modrinth, "iris", "v2", None)
            .await
            .unwrap();

        let mods = mc.join("versions").join("1.21").join("mods");
        assert_eq!(
            tokio::fs::read(mods.join("iris-1.7.jar")).await.unwrap(),
            new_bytes
        );
        // 旧文件不再以 jar 形式存在（不会被加载器读到两份），但字节原样留在 .old 备份里。
        assert!(!tokio::fs::try_exists(mods.join("iris-1.6.jar")).await.unwrap());
        assert_eq!(
            tokio::fs::read(mods.join("iris-1.6.jar.old")).await.unwrap(),
            old_bytes
        );

        // 卷宗只认新文件，旧文件名那条记录被摘掉。
        let ledger = aurora.ledger_store("1.21").load().await.unwrap();
        assert_eq!(ledger.entries.len(), 1);
        let entry = ledger.find("iris-1.7.jar").expect("卷宗应改挂到新文件名上");
        assert_eq!(entry.version_id, "v2");
        assert!(ledger.find("iris-1.6.jar").is_none());

        // 历史：先 Install 后 Update，Update 记全新旧文件名与新旧版本。
        let history = aurora.history("1.21").await.unwrap();
        assert_eq!(history.events.len(), 2);
        assert!(matches!(history.events[0], HistoryEvent::Install { .. }));
        let update_id = match &history.events[1] {
            HistoryEvent::Update {
                id,
                file_name,
                old_file,
                from_version,
                to_version,
                ..
            } => {
                assert_eq!(file_name, "iris-1.7.jar");
                assert_eq!(old_file, "iris-1.6.jar");
                assert_eq!(from_version, "v1");
                assert_eq!(to_version, "v2");
                id.clone()
            }
            other => panic!("期望 Update 事件，得到 {other:?}"),
        };

        // 备份就位即意味着这条更新可回滚——安装侧写下的备份名必须与回滚侧读的那个一致。
        let checks = aurora.rollback_checks("1.21").await.unwrap();
        let check = checks
            .iter()
            .find(|c| c.event_id == update_id)
            .expect("更新事件应出现在回滚判定里");
        assert!(check.can_rollback, "原因：{:?}", check.reason);
        assert_eq!(aurora.backup_size("1.21").await.unwrap(), old_bytes.len() as u64);
    }

    /// 同版本重复安装：不再下载、不写新历史，卷宗那条身份原样留着。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reinstalling_same_version_skips_download_and_keeps_ledger_entry() {
        let server = MockServer::start().await;
        let base = server.uri();
        let jar_bytes = b"already-here".to_vec();

        mount_versions(
            &server,
            "modmenu",
            format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "modmenu",
                    "2026-06-01T00:00:00Z",
                    "modmenu.jar",
                    &format!("{base}/modmenu.jar"),
                    &format!(r#"{{"sha1":"{}"}}"#, sha1_hex(&jar_bytes)),
                    jar_bytes.len(),
                    "",
                )
            ),
        )
        .await;
        // 只允许下载一次：第二次安装若还去下，wiremock 在服务器析构时会因期望不符而失败。
        Mock::given(method("GET"))
            .and(path("/modmenu.jar"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(jar_bytes.clone()))
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        put_version(&mc, "1.21").await;
        let aurora = isolated_aurora(&mc, base);

        aurora
            .install_mod("1.21", Platform::Modrinth, "modmenu", "v1", None)
            .await
            .unwrap();
        let first_ledger = aurora.ledger_store("1.21").load().await.unwrap();

        let outcome = aurora
            .install_mod("1.21", Platform::Modrinth, "modmenu", "v1", None)
            .await
            .unwrap();

        assert_eq!(
            outcome.path,
            mc.join("versions").join("1.21").join("mods").join("modmenu.jar")
        );
        // 身份没被第二次安装动过（时间戳都还是第一次那个），历史也没多出一条。
        assert_eq!(aurora.ledger_store("1.21").load().await.unwrap(), first_ledger);
        assert_eq!(aurora.history("1.21").await.unwrap().events.len(), 1);
        assert!(!tokio::fs::try_exists(
            mc.join("versions").join("1.21").join("mods").join("modmenu.jar.old")
        )
        .await
        .unwrap());
    }

    /// 带路径分隔符或形如 `..` 的文件名一律拒绝，绝不允许拼出目录之外的落点。
    #[test]
    fn safe_join_rejects_names_that_escape_the_directory() {
        let dir = Path::new("C:/mc/mods");
        assert_eq!(
            safe_join(dir, "sodium.jar").unwrap(),
            dir.join("sodium.jar")
        );
        for bad in ["../evil.jar", "sub/evil.jar", "..", "", "."] {
            let err = safe_join(dir, bad).unwrap_err();
            assert!(
                matches!(
                    err,
                    CoreError::Base(aurora_base::Error::Io { ref source, .. })
                        if source.kind() == std::io::ErrorKind::InvalidInput
                ),
                "文件名 {bad:?} 本该被拒，得到 {err:?}"
            );
        }
    }

    /// 卷宗按「平台 + 工程」认旧版本：文件名换了照样认得出，禁用态也算装着。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_previous_matches_by_project_even_when_file_name_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().to_path_buf();
        tokio::fs::create_dir_all(&mods).await.unwrap();
        tokio::fs::write(mods.join("sodium-0.5.jar.disabled"), b"old")
            .await
            .unwrap();

        let mut ledger = Ledger::default();
        ledger.upsert(LedgerEntry {
            file_name: "sodium-0.5.jar".to_owned(),
            platform: Platform::Modrinth,
            project_id: "sodium".to_owned(),
            version_id: "v1".to_owned(),
            sha1: None,
            installed_at: 1_700_000_000,
            installed_as_dependency_of: None,
        });

        // 暂存文件传一个不存在的路径：本用例验证的是卷宗那道判据，mod_id 兜底不该介入。
        let staged = tmp.path().join("staged-not-there.jar");
        let found = find_previous(
            &ledger,
            &mods,
            Platform::Modrinth,
            "sodium",
            "sodium-0.6.jar",
            &staged,
        )
        .await
        .unwrap();
        assert_eq!(
            found,
            Some(Replaced {
                old_file: "sodium-0.5.jar".to_owned(),
                from_version: "v1".to_owned(),
            })
        );

        // 平台不同即不是同一个工程：两边的工程标识各自成命名空间，撞名不代表同一个 Mod。
        let other_platform = find_previous(
            &ledger,
            &mods,
            Platform::CurseForge,
            "sodium",
            "sodium-0.6.jar",
            &staged,
        )
        .await
        .unwrap();
        assert_eq!(other_platform, None);
    }

    /// 卷宗查无来源、但目标文件名已被占：按顶替处理，旧版本记为「未知」，绝不静默覆盖。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_previous_treats_untracked_name_clash_as_replacement() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().to_path_buf();
        tokio::fs::create_dir_all(&mods).await.unwrap();
        tokio::fs::write(mods.join("manual.jar"), b"dropped-by-hand")
            .await
            .unwrap();

        let staged = tmp.path().join("staged-not-there.jar");
        let found = find_previous(
            &Ledger::default(),
            &mods,
            Platform::Modrinth,
            "whatever",
            "manual.jar",
            &staged,
        )
        .await
        .unwrap();
        assert_eq!(
            found,
            Some(Replaced {
                old_file: "manual.jar".to_owned(),
                from_version: UNKNOWN_VERSION.to_owned(),
            })
        );

        // 卷宗有记录但磁盘上没那个文件：玩家删了就是没装，本次按新装处理。
        let mut ledger = Ledger::default();
        ledger.upsert(LedgerEntry {
            file_name: "vanished.jar".to_owned(),
            platform: Platform::Modrinth,
            project_id: "gone".to_owned(),
            version_id: "v1".to_owned(),
            sha1: None,
            installed_at: 1_700_000_000,
            installed_as_dependency_of: None,
        });
        let missing = find_previous(
            &ledger,
            &mods,
            Platform::Modrinth,
            "gone",
            "gone-2.jar",
            &tmp.path().join("staged-not-there.jar"),
        )
            .await
            .unwrap();
        assert_eq!(missing, None);
    }

    /// 造一个带 fabric.mod.json 的最小 jar，用于验证按 mod_id 的跨来源判据。
    fn build_mod_jar(path: &Path, mod_id: &str) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("fabric.mod.json", options).unwrap();
        zip.write_all(
            format!(r#"{{"schemaVersion":1,"id":"{mod_id}","version":"1.0.0"}}"#).as_bytes(),
        )
        .unwrap();
        zip.finish().unwrap();
    }

    /// 同一个 Mod 从另一个平台再装一次：卷宗按「平台 + 工程」认不出，文件名也不撞，
    /// 但 jar 里的 mod_id 一致——必须顶替，否则两份共存直接是 DuplicateModsFoundException。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_previous_matches_across_platforms_by_mod_id() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("mods");
        tokio::fs::create_dir_all(&mods).await.unwrap();

        // 已装的那份来自 Modrinth，卷宗有记录。
        build_mod_jar(&mods.join("sodium-modrinth.jar"), "sodium");
        let mut ledger = Ledger::default();
        ledger.upsert(LedgerEntry {
            file_name: "sodium-modrinth.jar".to_owned(),
            platform: Platform::Modrinth,
            project_id: "AANobbMI".to_owned(),
            version_id: "v-modrinth".to_owned(),
            sha1: None,
            installed_at: 1_700_000_000,
            installed_as_dependency_of: None,
        });

        // 这次要装的是 CurseForge 的同一个 Mod：平台不同、工程 id 不同、文件名也不同。
        let staged = tmp.path().join("sodium-curseforge.jar");
        build_mod_jar(&staged, "sodium");

        let found = find_previous(
            &ledger,
            &mods,
            Platform::CurseForge,
            "394468",
            "sodium-curseforge.jar",
            &staged,
        )
        .await
        .unwrap();

        assert_eq!(
            found,
            Some(Replaced {
                old_file: "sodium-modrinth.jar".to_owned(),
                from_version: "v-modrinth".to_owned(),
            }),
            "跨平台的同一个 Mod 必须被认出来顶替，而不是放任两份共存"
        );
    }

    /// mod_id 不同就是两个 Mod：不能因为都装在同一个目录就互相顶替。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_previous_by_mod_id_leaves_unrelated_mods_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("mods");
        tokio::fs::create_dir_all(&mods).await.unwrap();
        build_mod_jar(&mods.join("lithium.jar"), "lithium");

        let staged = tmp.path().join("sodium.jar");
        build_mod_jar(&staged, "sodium");

        let found = find_previous(
            &Ledger::default(),
            &mods,
            Platform::Modrinth,
            "AANobbMI",
            "sodium.jar",
            &staged,
        )
        .await
        .unwrap();
        assert_eq!(found, None);
    }

    /// 手动丢进 mods 的同一个 Mod（卷宗完全没记录）：一样按 mod_id 认出来顶替，版本记未知。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn find_previous_by_mod_id_covers_hand_dropped_jar() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("mods");
        tokio::fs::create_dir_all(&mods).await.unwrap();
        build_mod_jar(&mods.join("我自己下的-sodium.jar"), "sodium");

        let staged = tmp.path().join("sodium-0.6.jar");
        build_mod_jar(&staged, "sodium");

        let found = find_previous(
            &Ledger::default(),
            &mods,
            Platform::Modrinth,
            "AANobbMI",
            "sodium-0.6.jar",
            &staged,
        )
        .await
        .unwrap();
        assert_eq!(
            found,
            Some(Replaced {
                old_file: "我自己下的-sodium.jar".to_owned(),
                from_version: UNKNOWN_VERSION.to_owned(),
            })
        );
    }
}
