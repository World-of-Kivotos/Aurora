//! 依赖图与安装计划。
//!
//! 安装一个 Mod 往往要连带装它的前置。这里把「要装哪些」先算成一份可检视的计划再落盘，而不是边下
//! 边解析：计划算完才知道总共几个文件、哪些其实已经装过、哪些依赖压根找不到匹配版本——这些都必须
//! 在玩家按下确认之前摆到他面前。
//!
//! 两条决策：
//!
//! 一、只自动收 Required 依赖。Optional 装了未必想要，Incompatible 装了必炸，Embedded 已经打包在
//! 主文件里再装一份会重复加载。这三类一律跳过，但跳过的事实要写进 [`InstallPlan::skipped`] 如实告知，
//! 而不是假装没有这回事。
//!
//! 二、找不到匹配版本的依赖不中断整个计划。主 Mod 仍然可以装，代价（大概率启动即崩）由 UI 明说，
//! 决定权交还玩家——直接失败会把「我只是想试试」这种完全合理的诉求也一并堵死。

use std::collections::HashSet;
use std::path::Path;

use aurora_instance::discover_versions;
use aurora_modplatform::{DependencyKind, ModLoader, ModVersionInfo, Platform, parse_loader_name};
use aurora_version::identify_mc_version;
use serde::Serialize;

use crate::compat::{Compatibility, classify};
use crate::error::{CoreError, Result};
use crate::facade::Aurora;
use crate::ledger::Ledger;
use crate::modversions::sort_by_published_desc;

/// 禁用态文件名后缀。被玩家禁用的 Mod 仍然躺在 `mods/` 里，装过就是装过——再下一份只会得到一对
/// 同名的启用/禁用副本，两边加载器各认一个。
const DISABLED_SUFFIX: &str = ".disabled";

/// 计划里的一项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedItem {
    /// 待安装的版本。
    pub version: ModVersionInfo,
    /// 因谁被带进来（project_id）；用户主动装的为 `None`。
    pub required_by: Option<String>,
    /// 该实例里已装同工程同版本，本次会跳过下载。
    pub already_satisfied: bool,
}

/// 一次安装的完整计划。
///
/// `items[0]` 恒为用户主动选的那个（`required_by` 为 `None`），其余为按依赖关系展开的前置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallPlan {
    /// 待安装项，主 Mod 在前，依赖在后。
    pub items: Vec<PlannedItem>,
    /// 被跳过的非必需依赖说明（每条一句中文），供 UI 如实告知而不是假装没有。
    pub skipped: Vec<String>,
}

/// 判定依赖版本所需的实例事实。
struct InstanceFacts {
    /// 实例的 MC 版本；版本 JSON 给不出任何线索（自定义命名的整合版本）时为 `None`。
    mc_version: Option<String>,
    /// 已装加载器的小写名。OptiFine 这类非 Mod 加载器也在其中，由 [`classify`] 自行忽略。
    loaders: Vec<String>,
}

impl Aurora {
    /// 解析依赖并产出安装计划。
    ///
    /// 栈式深度优先展开，`visited` 按 project_id 去重：真实数据里存在 A 依赖 B、B 又反过来声明依赖 A
    /// 的环（两个 Mod 互相集成时很常见），不去重会直接死循环。同一工程被多个父项要求时也只进计划一次。
    ///
    /// 只自动收 [`DependencyKind::Required`]；其余类型连同「声明里没给工程标识」的残缺依赖一律进
    /// [`InstallPlan::skipped`] 说明。找不到适配本实例版本的必需依赖同样只是进 `skipped`，主 Mod 仍在
    /// 计划里——决定要不要冒这个险是玩家的事，不是启动器的事。
    ///
    /// 网络故障与平台报错照常冒泡：那是「查不到」而非「没有」，把它压成一条 skipped 说明会让玩家以为
    /// 依赖真的不存在。
    pub async fn plan_install(
        &self,
        version_id: &str,
        platform: Platform,
        project_id: &str,
        mod_version_id: &str,
    ) -> Result<InstallPlan> {
        // 先把本地事实拿全：实例不存在时在触网之前就短路。
        let facts = instance_facts(self.game_dir(), version_id).await?;
        let mods_dir = self.resolve_mods_dir(version_id).await?;
        let ledger = self.ledger_store(version_id).load().await?;

        // 主项按版本 id 精确取，不做择优、也不带实例过滤：玩家在版本下拉里选了哪个就是哪个。
        // 带上过滤条件会把他刻意选的「不完全匹配但想试试」的版本筛没，然后报一句「版本不存在」。
        let root = self
            .list_mod_versions(platform, project_id, &[], &[])
            .await?
            .into_iter()
            .find(|v| v.version_id == mod_version_id)
            .ok_or_else(|| CoreError::ModVersionNotFound {
                platform: platform.display_name(),
                project_id: project_id.to_owned(),
                version_id: mod_version_id.to_owned(),
            })?;

        let mut items: Vec<PlannedItem> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        // 去重键取平台返回的规范 project_id，而不是入参——入参可能是 slug，与依赖声明里的 id 对不上。
        let mut visited: HashSet<String> = HashSet::from([root.project_id.clone()]);
        let mut stack: Vec<(ModVersionInfo, Option<String>)> = vec![(root, None)];

        while let Some((version, required_by)) = stack.pop() {
            let mut children: Vec<(ModVersionInfo, Option<String>)> = Vec::new();

            for dep in &version.dependencies {
                match dep.kind {
                    DependencyKind::Required => {
                        let Some(dep_project) = dep.project_id.clone() else {
                            skipped.push(format!(
                                "{} 的一条必需依赖没有给出工程标识，无法自动解析",
                                version.file_name
                            ));
                            continue;
                        };
                        // 已展开过（含环）就不再进计划：它要么已在 items 里，要么已被判为不可用。
                        if !visited.insert(dep_project.clone()) {
                            continue;
                        }
                        let pinned = dep.version_id.as_deref();
                        match resolve_required_dependency(
                            self,
                            platform,
                            &dep_project,
                            pinned,
                            &facts,
                        )
                        .await?
                        {
                            Some(chosen) => {
                                children.push((chosen, Some(version.project_id.clone())));
                            }
                            // 「作者钉的版本没了」与「这个实例用不上任何版本」是两回事，玩家该拿到
                            // 哪一种，取决于他接下来是去找替代 Mod 还是去换实例。
                            None => skipped.push(match pinned {
                                Some(pinned) => format!(
                                    "依赖 {dep_project} 指定的版本 {pinned} 在平台上已不存在，未加入计划"
                                ),
                                None => format!(
                                    "依赖 {dep_project} 没有适配该实例的版本，未加入计划"
                                ),
                            }),
                        }
                    }
                    DependencyKind::Optional => skipped.push(format!(
                        "可选依赖 {} 未自动安装，需要时请手动添加",
                        describe_project(dep.project_id.as_deref())
                    )),
                    DependencyKind::Incompatible => skipped.push(format!(
                        "{} 声明与 {} 不兼容，未做任何处理",
                        version.file_name,
                        describe_project(dep.project_id.as_deref())
                    )),
                    DependencyKind::Embedded => skipped.push(format!(
                        "内嵌依赖 {} 已打包在 {} 内，无需单独安装",
                        describe_project(dep.project_id.as_deref()),
                        version.file_name
                    )),
                    DependencyKind::Tool => skipped.push(format!(
                        "工具类关联 {} 未自动安装",
                        describe_project(dep.project_id.as_deref())
                    )),
                }
            }

            // 反序压栈，让出栈顺序与依赖声明顺序一致：计划是给人看的，顺序抖动会让人以为算错了。
            for child in children.into_iter().rev() {
                stack.push(child);
            }

            let satisfied = already_satisfied(&ledger, &mods_dir, &version).await?;
            items.push(PlannedItem {
                version,
                required_by,
                already_satisfied: satisfied,
            });
        }

        Ok(InstallPlan { items, skipped })
    }
}

/// 取判定所需的实例事实；版本本地未安装返回 [`CoreError::VersionNotInstalled`]。
async fn instance_facts(game_dir: &Path, version_id: &str) -> Result<InstanceFacts> {
    let scan = discover_versions(game_dir).await?;
    let target = scan
        .versions
        .into_iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| CoreError::VersionNotInstalled {
            id: version_id.to_owned(),
        })?;

    Ok(InstanceFacts {
        mc_version: identify_mc_version(&target.json).value,
        loaders: target
            .loaders
            .iter()
            .map(|info| info.kind.as_str().to_lowercase())
            .collect(),
    })
}

/// 解析一条必需依赖，选出该实例可用的版本；无版本可用返回 `None`（由调用方给出中文原因）。
///
/// 依赖声明里带了精确版本号时原样采用、不再择优：那是 Mod 作者钉死的搭配，启动器凭「更新」二字
/// 推翻它，换来的通常是一个作者从未测过的组合。
async fn resolve_required_dependency(
    aurora: &Aurora,
    platform: Platform,
    dep_project: &str,
    pinned_version: Option<&str>,
    facts: &InstanceFacts,
) -> Result<Option<ModVersionInfo>> {
    // 把实例事实作为过滤条件带下去。这不是为了省流量：CurseForge 的文件端点是分页的，只取首页，
    // 一个动辄几百个文件的前置（Fabric API 这种）不带条件查，首页可能一个适配本实例的文件都没有，
    // 结果是「明明有版本却报没有适配版本」。派生自 facts 而不另存一份，避免同一事实两处漂移。
    let game_versions: Vec<String> = facts.mc_version.clone().into_iter().collect();
    let loaders: Vec<ModLoader> = facts
        .loaders
        .iter()
        .filter_map(|name| parse_loader_name(name))
        .collect();
    let candidates = aurora
        .list_mod_versions(platform, dep_project, &game_versions, &loaders)
        .await?;

    Ok(match pinned_version {
        Some(pinned) => candidates.into_iter().find(|v| v.version_id == pinned),
        None => pick_latest_compatible(candidates, facts),
    })
}

/// 从候选里挑该实例可用的最新版本。
///
/// 显式再排一次而不是依赖上游返回顺序：择优的全部意义就是「最新」，这条前提不该跨模块靠约定维持。
/// 判定不为 Mismatch 即可用——[`Compatibility::Unknown`]（平台没给元数据）按放行处理，与落位层的
/// 软提示口径一致。
fn pick_latest_compatible(
    mut candidates: Vec<ModVersionInfo>,
    facts: &InstanceFacts,
) -> Option<ModVersionInfo> {
    sort_by_published_desc(&mut candidates);
    candidates
        .into_iter()
        .find(|version| !matches!(judge(version, facts), Compatibility::Mismatch { .. }))
}

/// 按实例事实判定某版本是否可用。
fn judge(version: &ModVersionInfo, facts: &InstanceFacts) -> Compatibility {
    match facts.mc_version.as_deref() {
        Some(mc) => classify(version, mc, &facts.loaders),
        None => {
            // 实例 MC 版本识别不出时，把版本侧的 MC 维度一并抹掉后交给同一个判定函数，只判加载器。
            // 拿空版本号去比对会把每个候选都判成「不支持 MC 」——那是因为我们不知道而惩罚玩家。
            let loader_only = ModVersionInfo {
                game_versions: Vec::new(),
                ..version.clone()
            };
            classify(&loader_only, "", &facts.loaders)
        }
    }
}

/// 该版本是否已装在此实例（卷宗有同工程同版本的记录，且其文件确实在磁盘上）。
///
/// 磁盘是权威：卷宗有记录但文件被玩家删了就是没装，此时必须重新下载。
async fn already_satisfied(
    ledger: &Ledger,
    mods_dir: &Path,
    version: &ModVersionInfo,
) -> Result<bool> {
    let Some(entry) = ledger
        .entries
        .iter()
        .find(|e| e.project_id == version.project_id && e.version_id == version.version_id)
    else {
        return Ok(false);
    };
    file_present(mods_dir, &entry.file_name).await
}

/// 文件是否在 mods 目录里（启用态或禁用态都算）。
async fn file_present(mods_dir: &Path, file_name: &str) -> Result<bool> {
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

/// 跳过说明里对依赖工程的称呼。
///
/// 依赖声明只带工程标识、不带可读名称，换成人话得再打一轮网络请求。宁可显示一串 id，也不能因为
/// 名字取不到就把这条依赖从说明里抹掉。
fn describe_project(project_id: Option<&str>) -> String {
    match project_id {
        Some(id) => id.to_owned(),
        None => "未标明工程的依赖项".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use crate::ledger::LedgerEntry;
    use aurora_modplatform::ReleaseChannel;
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn version(project_id: &str, version_id: &str) -> ModVersionInfo {
        ModVersionInfo {
            version_id: version_id.to_owned(),
            project_id: project_id.to_owned(),
            platform: Platform::Modrinth,
            name: "Sodium 0.5.3".to_owned(),
            version_number: "mc1.20.1-0.5.3".to_owned(),
            release_channel: ReleaseChannel::Release,
            game_versions: vec!["1.20.1".to_owned()],
            loaders: Vec::new(),
            file_name: format!("{project_id}.jar"),
            file_size: None,
            sha1: None,
            date_published: "2026-01-02T03:04:05Z".to_owned(),
            dependencies: Vec::new(),
        }
    }

    /// 一条 Modrinth 版本 JSON。`deps` 直接拼进 `dependencies` 数组（可为空串）。
    fn modrinth_version(
        id: &str,
        project: &str,
        date: &str,
        game_versions: &[&str],
        loaders: &[&str],
        deps: &str,
    ) -> String {
        let game_versions = serde_json::to_string(game_versions).unwrap();
        let loaders = serde_json::to_string(loaders).unwrap();
        format!(
            r#"{{"id":"{id}","project_id":"{project}","name":"{project} {id}",
                "version_number":"{id}","version_type":"release","date_published":"{date}",
                "game_versions":{game_versions},"loaders":{loaders},"dependencies":[{deps}],
                "files":[{{"hashes":{{"sha1":"aabbcc"}},"url":"https://example.invalid/{id}.jar",
                    "filename":"{project}-{id}.jar","primary":true,"size":1024}}]}}"#
        )
    }

    /// 挂载某工程的版本列表端点。
    async fn mount_versions(server: &MockServer, project: &str, versions: &[String]) {
        let body = format!("[{}]", versions.join(","));
        Mock::given(method("GET"))
            .and(path(format!("/project/{project}/version")))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    /// 落一个 Fabric 实例：inheritsFrom 给出 MC 1.20.1，库坐标让加载器探测命中 Fabric。
    async fn put_fabric_instance(mc: &Path, id: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            format!(
                r#"{{"id":"{id}","inheritsFrom":"1.20.1","type":"release",
                    "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
                    "libraries":[{{"name":"net.fabricmc:fabric-loader:0.15.11"}}]}}"#
            ),
        )
        .await
        .unwrap();
    }

    fn aurora_at(mc: &Path, modrinth_base: String) -> Aurora {
        Aurora::for_test(AuroraConfig::default(), mc.to_path_buf(), mc.to_path_buf())
            .with_modrinth_base(modrinth_base)
    }

    #[test]
    fn plan_serializes_with_ipc_field_names() {
        let plan = InstallPlan {
            items: vec![
                PlannedItem {
                    version: version("AANobbMI", "IZskiJmZ"),
                    required_by: None,
                    already_satisfied: false,
                },
                PlannedItem {
                    version: version("P7dR8mSH", "QwErTyUi"),
                    required_by: Some("AANobbMI".to_owned()),
                    already_satisfied: true,
                },
            ],
            skipped: vec!["跳过可选依赖 Mod Menu（非必需）".to_owned()],
        };

        let json = serde_json::to_value(&plan).unwrap();
        // 主 Mod 在前且无来源，依赖在后并标出因谁而来。
        assert_eq!(json["items"][0]["required_by"], serde_json::Value::Null);
        assert_eq!(json["items"][0]["already_satisfied"], false);
        assert_eq!(json["items"][0]["version"]["version_id"], "IZskiJmZ");
        assert_eq!(json["items"][1]["required_by"], "AANobbMI");
        assert_eq!(json["items"][1]["already_satisfied"], true);
        assert_eq!(json["items"][1]["version"]["project_id"], "P7dR8mSH");
        assert_eq!(json["skipped"][0], "跳过可选依赖 Mod Menu（非必需）");
    }

    #[test]
    fn empty_plan_keeps_both_arrays_present() {
        // UI 直接读 items/skipped 的长度，字段不能因为空而消失。
        let json = serde_json::to_value(InstallPlan {
            items: Vec::new(),
            skipped: Vec::new(),
        })
        .unwrap();
        assert_eq!(json["items"], serde_json::json!([]));
        assert_eq!(json["skipped"], serde_json::json!([]));
    }

    /// 无依赖的工程：计划里只有玩家选的那一项。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn plan_without_dependencies_contains_only_the_chosen_version() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].version.version_id, "v1");
        assert_eq!(plan.items[0].version.file_name, "sodium-v1.jar");
        assert_eq!(plan.items[0].required_by, None);
        assert!(!plan.items[0].already_satisfied);
        assert!(plan.skipped.is_empty());
    }

    /// 必需依赖被收进计划，且取的是「适配本实例的最新版本」而非全局最新。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn required_dependency_resolves_to_newest_compatible_version() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[
                // 全局最新，但只支持 1.21：不能被选中。
                modrinth_version(
                    "for-1-21",
                    "fabric-api",
                    "2026-06-01T00:00:00Z",
                    &["1.21"],
                    &["fabric"],
                    "",
                ),
                // 适配本实例的最新版。
                modrinth_version(
                    "for-1-20-1-new",
                    "fabric-api",
                    "2026-02-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    "",
                ),
                // 同样适配但更旧。
                modrinth_version(
                    "for-1-20-1-old",
                    "fabric-api",
                    "2025-02-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    "",
                ),
            ],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[0].version.version_id, "v1");
        assert_eq!(plan.items[0].required_by, None);
        assert_eq!(plan.items[1].version.project_id, "fabric-api");
        assert_eq!(plan.items[1].version.version_id, "for-1-20-1-new");
        assert_eq!(plan.items[1].required_by, Some("sodium".to_owned()));
        assert!(plan.skipped.is_empty());
    }

    /// A 依赖 B、B 反过来依赖 A：必须收敛成两项，且不重复展开。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cyclic_dependencies_do_not_loop_forever() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "alpha",
            &[modrinth_version(
                "a1",
                "alpha",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"beta","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "beta",
            &[modrinth_version(
                "b1",
                "beta",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"alpha","dependency_type":"required"}"#,
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "alpha", "a1")
            .await
            .unwrap();

        let planned: Vec<&str> = plan
            .items
            .iter()
            .map(|item| item.version.project_id.as_str())
            .collect();
        assert_eq!(planned, vec!["alpha", "beta"]);
        // 回指主项的那条依赖既不该重复入计划，也不该被当成「跳过的非必需依赖」报出来。
        assert!(plan.skipped.is_empty());
    }

    /// 非必需依赖一律不自动安装，但每一条都要在 skipped 里说清楚。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn non_required_dependencies_are_skipped_with_reasons() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                concat!(
                    r#"{"project_id":"modmenu","dependency_type":"optional"},"#,
                    r#"{"project_id":"optifabric","dependency_type":"incompatible"},"#,
                    r#"{"project_id":"cloth-config","dependency_type":"embedded"}"#
                ),
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        // 三条都不进计划。
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].version.version_id, "v1");

        assert_eq!(plan.skipped.len(), 3);
        assert_eq!(
            plan.skipped[0],
            "可选依赖 modmenu 未自动安装，需要时请手动添加"
        );
        assert_eq!(
            plan.skipped[1],
            "sodium-v1.jar 声明与 optifabric 不兼容，未做任何处理"
        );
        assert_eq!(
            plan.skipped[2],
            "内嵌依赖 cloth-config 已打包在 sodium-v1.jar 内，无需单独安装"
        );
    }

    /// 依赖没有适配本实例的版本：进 skipped 说明，主 Mod 仍留在计划里。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dependency_without_compatible_version_is_skipped_but_main_stays() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[
                // MC 版本对不上。
                modrinth_version(
                    "old",
                    "fabric-api",
                    "2024-01-01T00:00:00Z",
                    &["1.19.2"],
                    &["fabric"],
                    "",
                ),
                // 加载器对不上。
                modrinth_version(
                    "forge-only",
                    "fabric-api",
                    "2026-05-01T00:00:00Z",
                    &["1.20.1"],
                    &["forge"],
                    "",
                ),
            ],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.items[0].version.version_id, "v1");
        assert_eq!(
            plan.skipped,
            vec!["依赖 fabric-api 没有适配该实例的版本，未加入计划".to_owned()]
        );
    }

    /// 依赖钉死了精确版本：原样采用，不因为有更新的版本就改口。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinned_dependency_version_is_taken_verbatim() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"fabric-api","version_id":"pinned","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[
                modrinth_version(
                    "newer",
                    "fabric-api",
                    "2026-06-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    "",
                ),
                modrinth_version(
                    "pinned",
                    "fabric-api",
                    "2025-01-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    "",
                ),
            ],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[1].version.version_id, "pinned");
        assert!(plan.skipped.is_empty());
    }

    /// 钉死的版本在平台上已被删除：说明具体是哪个版本没了，而不是笼统说「找不到」。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pinned_dependency_version_gone_reports_which_version() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"fabric-api","version_id":"gone","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[modrinth_version(
                "still-here",
                "fabric-api",
                "2026-06-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.skipped,
            vec!["依赖 fabric-api 指定的版本 gone 在平台上已不存在，未加入计划".to_owned()]
        );
    }

    /// 必需依赖没给工程标识：无从解析，如实记一条而不是当没看见。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn required_dependency_without_project_id_is_reported() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"file_name":"some-lib.jar","dependency_type":"required"}"#,
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.skipped,
            vec!["sodium-v1.jar 的一条必需依赖没有给出工程标识，无法自动解析".to_owned()]
        );
    }

    /// already_satisfied 以磁盘为准：卷宗有记录但文件不在就是没装；禁用态文件仍然算装了。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn already_satisfied_follows_disk_not_ledger() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[modrinth_version(
                "ok1",
                "fabric-api",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        // 卷宗声称两个都装过。
        let mut ledger = Ledger::default();
        for (file_name, project_id, version_id) in [
            ("sodium-v1.jar", "sodium", "v1"),
            ("fabric-api-ok1.jar", "fabric-api", "ok1"),
        ] {
            ledger.upsert(LedgerEntry {
                file_name: file_name.to_owned(),
                platform: Platform::Modrinth,
                project_id: project_id.to_owned(),
                version_id: version_id.to_owned(),
                sha1: None,
                installed_at: 1_754_000_000,
                installed_as_dependency_of: None,
            });
        }
        aurora
            .ledger_store("1.20.1-fabric")
            .save(&ledger)
            .await
            .unwrap();

        // 磁盘上只有主 Mod，且是禁用态。
        let mods_dir = aurora.resolve_mods_dir("1.20.1-fabric").await.unwrap();
        tokio::fs::create_dir_all(&mods_dir).await.unwrap();
        tokio::fs::write(mods_dir.join("sodium-v1.jar.disabled"), b"jar")
            .await
            .unwrap();

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();
        assert_eq!(plan.items.len(), 2);
        // 禁用只是改了文件名，装过就是装过。
        assert!(plan.items[0].already_satisfied);
        // 卷宗有、磁盘没有：必须判为未装，否则这个依赖永远补不回来。
        assert!(!plan.items[1].already_satisfied);

        // 依赖文件补齐后再算一次，转为已满足。
        tokio::fs::write(mods_dir.join("fabric-api-ok1.jar"), b"jar")
            .await
            .unwrap();
        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();
        assert!(plan.items[1].already_satisfied);
    }

    /// 卷宗里同工程但版本号不同：不算已满足（要装的是另一个版本）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn different_version_of_same_project_is_not_satisfied() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v2",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let mut ledger = Ledger::default();
        ledger.upsert(LedgerEntry {
            file_name: "sodium-v1.jar".to_owned(),
            platform: Platform::Modrinth,
            project_id: "sodium".to_owned(),
            version_id: "v1".to_owned(),
            sha1: None,
            installed_at: 1_754_000_000,
            installed_as_dependency_of: None,
        });
        aurora
            .ledger_store("1.20.1-fabric")
            .save(&ledger)
            .await
            .unwrap();
        let mods_dir = aurora.resolve_mods_dir("1.20.1-fabric").await.unwrap();
        tokio::fs::create_dir_all(&mods_dir).await.unwrap();
        tokio::fs::write(mods_dir.join("sodium-v1.jar"), b"jar")
            .await
            .unwrap();

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v2")
            .await
            .unwrap();
        assert_eq!(plan.items.len(), 1);
        assert!(!plan.items[0].already_satisfied);
    }

    /// 原版实例（无加载器）：要加载器的依赖判为不兼容，进 skipped。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn vanilla_instance_finds_no_loader_dependent_version() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &[],
                &[],
                r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
            )],
        )
        .await;
        mount_versions(
            &server,
            "fabric-api",
            &[modrinth_version(
                "ok1",
                "fabric-api",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("versions").join("1.20.1");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join("1.20.1.json"),
            r#"{"id":"1.20.1","type":"release","mainClass":"net.minecraft.client.main.Main"}"#,
        )
        .await
        .unwrap();
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.skipped,
            vec!["依赖 fabric-api 没有适配该实例的版本，未加入计划".to_owned()]
        );
    }

    /// 平台上没有请求的主版本：冒泡 ModVersionNotFound，不产出一份空计划。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unknown_mod_version_errors() {
        let server = MockServer::start().await;
        mount_versions(
            &server,
            "sodium",
            &[modrinth_version(
                "v1",
                "sodium",
                "2026-01-01T00:00:00Z",
                &["1.20.1"],
                &["fabric"],
                "",
            )],
        )
        .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let err = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "nope")
            .await
            .unwrap_err();
        match err {
            CoreError::ModVersionNotFound {
                project_id,
                version_id,
                ..
            } => {
                assert_eq!(project_id, "sodium");
                assert_eq!(version_id, "nope");
            }
            other => panic!("期望 ModVersionNotFound，得到 {other:?}"),
        }
    }

    /// 实例本地未安装：在触网之前就短路。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn uninstalled_instance_errors_before_network() {
        // 不挂载任何端点：一旦实现先去联网，请求会失败并给出别的错误。
        let server = MockServer::start().await;
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path(), server.uri());

        let err = aurora
            .plan_install("ghost", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap_err();
        assert!(matches!(err, CoreError::VersionNotInstalled { id } if id == "ghost"));
    }

    /// 依赖查询必须把实例的 MC 版本与加载器下推给平台，主项查询必须不带任何过滤。
    ///
    /// CurseForge 的文件端点只取首页，前置不带条件查会「明明有版本却找不到」；主项带条件查则会把
    /// 玩家刻意选中的版本筛掉。两条端点用互斥的匹配条件钉死，任一侧改错都会让 mock 落空而报错。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn instance_filters_are_pushed_down_for_dependencies_only() {
        let server = MockServer::start().await;
        // 主项端点只接受「不带任何过滤」的请求。
        Mock::given(method("GET"))
            .and(path("/project/sodium/version"))
            .and(query_param_is_missing("game_versions"))
            .and(query_param_is_missing("loaders"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "[{}]",
                modrinth_version(
                    "v1",
                    "sodium",
                    "2026-01-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    r#"{"project_id":"fabric-api","dependency_type":"required"}"#,
                )
            )))
            .mount(&server)
            .await;
        // 依赖端点只接受带上本实例事实的请求。
        Mock::given(method("GET"))
            .and(path("/project/fabric-api/version"))
            .and(query_param("game_versions", r#"["1.20.1"]"#))
            .and(query_param("loaders", r#"["fabric"]"#))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                "[{}]",
                modrinth_version(
                    "ok1",
                    "fabric-api",
                    "2026-01-01T00:00:00Z",
                    &["1.20.1"],
                    &["fabric"],
                    "",
                )
            )))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        put_fabric_instance(tmp.path(), "1.20.1-fabric").await;
        let aurora = aurora_at(tmp.path(), server.uri());

        let plan = aurora
            .plan_install("1.20.1-fabric", Platform::Modrinth, "sodium", "v1")
            .await
            .unwrap();
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.items[1].version.version_id, "ok1");
    }

    #[test]
    fn judge_ignores_mc_dimension_when_instance_version_is_unidentifiable() {
        // MC 版本识别不出的自定义实例：只按加载器判，不能因为「我们不知道」把候选全判死。
        let unknown_mc = InstanceFacts {
            mc_version: None,
            loaders: vec!["fabric".to_owned()],
        };
        let mut fabric_1_21 = version("fabric-api", "x");
        fabric_1_21.game_versions = vec!["1.21".to_owned()];
        fabric_1_21.loaders = vec![aurora_modplatform::ModLoader::Fabric];
        assert_eq!(judge(&fabric_1_21, &unknown_mc), Compatibility::Unknown);

        // 加载器这一维仍然照判。
        let mut forge_only = fabric_1_21.clone();
        forge_only.loaders = vec![aurora_modplatform::ModLoader::Forge];
        assert_eq!(
            judge(&forge_only, &unknown_mc),
            Compatibility::Mismatch {
                reason: "需要 Forge，该实例装的是 Fabric".to_owned()
            }
        );

        // MC 版本已知时恢复完整判定。
        let known_mc = InstanceFacts {
            mc_version: Some("1.20.1".to_owned()),
            loaders: vec!["fabric".to_owned()],
        };
        assert_eq!(
            judge(&fabric_1_21, &known_mc),
            Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            }
        );
    }

    #[test]
    fn pick_latest_compatible_orders_by_date_not_input_order() {
        let facts = InstanceFacts {
            mc_version: Some("1.20.1".to_owned()),
            loaders: vec!["fabric".to_owned()],
        };
        let mut older = version("fabric-api", "older");
        older.loaders = vec![aurora_modplatform::ModLoader::Fabric];
        older.date_published = "2025-01-01T00:00:00Z".to_owned();
        let mut newer = version("fabric-api", "newer");
        newer.loaders = vec![aurora_modplatform::ModLoader::Fabric];
        newer.date_published = "2026-01-01T00:00:00Z".to_owned();
        let mut incompatible = version("fabric-api", "incompatible");
        incompatible.loaders = vec![aurora_modplatform::ModLoader::Forge];
        incompatible.date_published = "2026-12-01T00:00:00Z".to_owned();

        // 输入顺序刻意打乱：择优不该依赖调用方给的顺序。
        let picked = pick_latest_compatible(
            vec![older.clone(), incompatible.clone(), newer.clone()],
            &facts,
        )
        .expect("应选出兼容版本");
        assert_eq!(picked.version_id, "newer");

        // 全部不兼容时给 None，由调用方写进 skipped。
        assert!(pick_latest_compatible(vec![incompatible], &facts).is_none());
        assert!(pick_latest_compatible(Vec::new(), &facts).is_none());
    }
}
