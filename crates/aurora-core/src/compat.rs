//! 兼容性判定与实例匹配。
//!
//! 下载页的落位层要回答一个问题：「这个 Mod 该装进哪个实例」。答案由两部分组成——纯判定
//! （[`classify`]：某个版本对某组实例事实是否可用）与编排（把判定结果铺成全部已装实例的矩阵）。
//! 判定被刻意做成不带 `self` 的自由函数，边界情形（平台没给加载器、实例是原版、MC 版本对不上）
//! 才有可能被单元测试逐条钉死。
//!
//! 三态而非布尔是关键决策：平台元数据本来就不完整（CurseForge 早期文件常常既没标加载器也没标
//! MC 版本），把「不知道」压成「不兼容」会让一大批本可正常使用的 Mod 在 UI 上凭空消失。
//! 因此 [`Compatibility::Unknown`] 在界面上是软提示、仍然放行，只有确凿对不上才判
//! [`Compatibility::Mismatch`]，并附一句给玩家看的中文原因——「装不了」必须同时说明「为什么」。

use std::collections::HashSet;

use aurora_instance::{DiscoveredVersion, discover_versions};
use aurora_modplatform::{ModLoader, ModVersionInfo, Platform, parse_loader_name, scan_mods_dir};
use serde::Serialize;

use crate::error::Result;
use crate::facade::Aurora;
use crate::modversions::sort_by_published_desc;

/// 实例工作目录下的模组目录名。与 [`Aurora::resolve_mods_dir`] 同一约定——那条路径每次都要重扫
/// `versions/`，矩阵要对每个实例算一遍，故这里用已扫到的事实自行拼装。
const MODS_DIR: &str = "mods";

/// 禁用态文件名后缀。上游 `aurora_modplatform` 的同名常量未公开，这里按同一约定复述：卷宗的 join 键
/// 是启用态文件名，磁盘上带该后缀的文件必须先剥掉后缀才对得上（禁用只是关掉，不是没装）。
const DISABLED_SUFFIX: &str = ".disabled";

/// 某版本对某实例的兼容判定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Compatibility {
    /// MC 版本与加载器都对上。
    Match,
    /// 明确对不上，`reason` 是给玩家看的一句中文说明。
    Mismatch { reason: String },
    /// 平台没给足够元数据，无法判断（不等于不兼容，UI 上作软提示放行）。
    Unknown,
}

/// 一个已装实例对某工程的匹配结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstanceMatch {
    /// 实例（已安装版本）id。
    pub version_id: String,
    /// 实例的 MC 版本号。
    pub mc_version: String,
    /// 实例已装的加载器种类（小写名）。
    pub loaders: Vec<String>,
    /// 该实例对本工程最佳版本的兼容判定。
    pub compatibility: Compatibility,
    /// 该实例下最合适的版本（无兼容版本时为 `None`）。
    pub best_version: Option<ModVersionInfo>,
    /// 该工程已装在此实例里时给出已装文件名（据卷宗与磁盘 join）。
    pub already_installed: Option<String>,
}

/// 加载器的展示名（专有名词，保留原文大小写）。
///
/// 判定结果里的实例加载器是小写技术名，直接塞进中文文案会读成「该实例装的是 neoforge」；
/// 玩家在其它地方看到的一律是 NeoForge，两处不一致会让人怀疑是不是两个东西。
pub(crate) fn loader_display_name(loader: ModLoader) -> &'static str {
    match loader {
        ModLoader::Fabric => "Fabric",
        ModLoader::Quilt => "Quilt",
        ModLoader::Forge => "Forge",
        ModLoader::NeoForge => "NeoForge",
        ModLoader::LiteLoader => "LiteLoader",
    }
}

/// 把一组加载器拼成文案里的一段，如 `Fabric/Quilt`。
fn join_loader_names(loaders: &[ModLoader]) -> String {
    loaders
        .iter()
        .map(|l| loader_display_name(*l))
        .collect::<Vec<_>>()
        .join("/")
}

/// 纯函数：判定某个版本能否用于给定的实例事实。
///
/// `instance_loaders` 取实例已装的加载器名（大小写不敏感，非 Mod 加载器如 OptiFine 会被忽略）。
/// 判定顺序按「越确凿越先判」排：先看加载器（对不上一定装不了），再看 MC 版本，最后才把
/// 元数据缺失归到 [`Compatibility::Unknown`]。
pub fn classify(
    version: &ModVersionInfo,
    instance_mc_version: &str,
    instance_loaders: &[String],
) -> Compatibility {
    // 实例侧加载器归一后再比对：两个平台与本地探测对同一加载器的写法并不统一。认不出的名字
    // （OptiFine 这类不是 Mod 加载器的东西）不参与交集——把它算作加载器会让原版+OptiFine 的实例
    // 被误判成「装了加载器」，进而给出一条毫无帮助的「需要 Forge，该实例装的是 OptiFine」。
    let instance_kinds: Vec<ModLoader> = instance_loaders
        .iter()
        .filter_map(|name| parse_loader_name(name))
        .collect();

    if !version.loaders.is_empty() {
        if instance_kinds.is_empty() {
            return Compatibility::Mismatch {
                reason: "该实例未安装 Mod 加载器".to_owned(),
            };
        }
        let intersects = version
            .loaders
            .iter()
            .any(|needed| instance_kinds.contains(needed));
        if !intersects {
            return Compatibility::Mismatch {
                reason: format!(
                    "需要 {}，该实例装的是 {}",
                    join_loader_names(&version.loaders),
                    join_loader_names(&instance_kinds)
                ),
            };
        }
    }

    if !version.game_versions.is_empty()
        && !version
            .game_versions
            .iter()
            .any(|v| v == instance_mc_version)
    {
        return Compatibility::Mismatch {
            reason: format!("不支持 MC {instance_mc_version}"),
        };
    }

    // 走到这里说明「已知的部分都对得上」。但只要有一维平台压根没给，就没有资格声称匹配。
    if version.game_versions.is_empty() || version.loaders.is_empty() {
        return Compatibility::Unknown;
    }

    Compatibility::Match
}

/// 排序档位：完美匹配 0 > 可能可行 1 > 不兼容 2。
///
/// 落位层默认选中第一项，所以档位序即「替玩家做的那个决定」：能确定装得上的排最前，
/// 拿不准的次之，确凿装不上的沉底但仍然列出（玩家有权知道它为什么不在候选里）。
fn tier(compatibility: &Compatibility) -> u8 {
    match compatibility {
        Compatibility::Match => 0,
        Compatibility::Unknown => 1,
        Compatibility::Mismatch { .. } => 2,
    }
}

/// 从版本列表里为一组实例事实挑最合适的版本，并给出该实例的整体判定。
///
/// `versions` 必须已按发布时间倒序：本函数取「第一个能用的」即认作最新可用版本，顺序错了择优就错了。
/// 判定优先级与 [`tier`] 一致——一旦命中确定匹配立刻返回，不再往下翻旧版本。
///
/// 全部版本都对不上时取「最新那个版本的原因」作为该实例的说明：玩家最可能去试的就是最新版，
/// 拿一个陈年旧版的失败原因去解释当前状况只会误导。
fn pick_best(
    versions: &[ModVersionInfo],
    instance_mc_version: &str,
    instance_loaders: &[String],
) -> (Compatibility, Option<ModVersionInfo>) {
    let mut newest_unknown: Option<&ModVersionInfo> = None;
    let mut newest_reason: Option<String> = None;

    for version in versions {
        match classify(version, instance_mc_version, instance_loaders) {
            Compatibility::Match => return (Compatibility::Match, Some(version.clone())),
            Compatibility::Unknown => {
                if newest_unknown.is_none() {
                    newest_unknown = Some(version);
                }
            }
            Compatibility::Mismatch { reason } => {
                if newest_reason.is_none() {
                    newest_reason = Some(reason);
                }
            }
        }
    }

    if let Some(version) = newest_unknown {
        return (Compatibility::Unknown, Some(version.clone()));
    }
    match newest_reason {
        Some(reason) => (Compatibility::Mismatch { reason }, None),
        // 工程一个版本都没有：没东西可装是确凿事实，如实说明，不含糊成「无法判断」。
        None => (
            Compatibility::Mismatch {
                reason: "该工程没有可用版本".to_owned(),
            },
            None,
        ),
    }
}

/// 实例的 MC 版本号：加载器版本的 `inheritsFrom` 即其基准原版版本；独立版本（原版、快照）的 id
/// 本身就是版本号。空串按缺失处理——写了个空 `inheritsFrom` 的版本 JSON 若照单全收，
/// 会让该实例的 MC 版本变成空串，之后每个版本都判「不支持 MC 」。
fn instance_mc_version(version: &DiscoveredVersion) -> String {
    version
        .json
        .inherits_from
        .clone()
        .filter(|parent| !parent.is_empty())
        .unwrap_or_else(|| version.id.clone())
}

/// 实例已装加载器的小写技术名。OptiFine / LiteLoader 这类也照实列出，由 [`classify`] 自行取舍——
/// 这里的职责是陈述实例事实，不是替判定做筛选。
fn instance_loaders(version: &DiscoveredVersion) -> Vec<String> {
    version
        .loaders
        .iter()
        .map(|loader| loader.kind.as_str().to_ascii_lowercase())
        .collect()
}

impl Aurora {
    /// 为某工程算出全部已装实例的匹配矩阵，按「完美匹配 > 可能可行 > 不兼容」排序，
    /// 同档内按实例 id 字典序，保证顺序稳定（UI 默认选中第一项）。
    ///
    /// 工程版本列表整场只拉一次，之后每个实例都在本地判定。逐实例发请求会让装了十来个实例的玩家
    /// 在点开安装弹层时等上十几个网络来回，还会撞上平台限流把后半截请求打成失败。
    pub async fn match_instances(
        &self,
        platform: Platform,
        project_id: &str,
    ) -> Result<Vec<InstanceMatch>> {
        // 先扫本地：一个实例都没有时连网络都不必打。
        let scan = discover_versions(self.game_dir()).await?;
        if scan.versions.is_empty() {
            return Ok(Vec::new());
        }

        let mut versions = self
            .list_mod_versions(platform, project_id, &[], &[])
            .await?;
        // 择优依赖倒序，就地保证一次，不把本函数的正确性押在上游的排序承诺上。
        sort_by_published_desc(&mut versions);

        let mut matches = Vec::with_capacity(scan.versions.len());
        for discovered in &scan.versions {
            let mc_version = instance_mc_version(discovered);
            let loaders = instance_loaders(discovered);
            let (compatibility, best_version) = pick_best(&versions, &mc_version, &loaders);
            let already_installed = self
                .installed_file_of(discovered, platform, project_id)
                .await?;
            matches.push(InstanceMatch {
                version_id: discovered.id.clone(),
                mc_version,
                loaders,
                compatibility,
                best_version,
                already_installed,
            });
        }

        matches.sort_by(|a, b| {
            tier(&a.compatibility)
                .cmp(&tier(&b.compatibility))
                .then_with(|| a.version_id.cmp(&b.version_id))
        });
        Ok(matches)
    }

    /// 该工程是否已装在这个实例里：卷宗给身份，磁盘给存在性，两者都成立才算装了。
    ///
    /// 卷宗有而磁盘没有一律当没装——玩家手动删 jar 是完全合法的操作，拿一条陈旧记录告诉他「已安装」
    /// 会让他找不到那个根本不存在的文件。卷宗里一条都对不上时直接短路，不去扫盘。
    async fn installed_file_of(
        &self,
        discovered: &DiscoveredVersion,
        platform: Platform,
        project_id: &str,
    ) -> Result<Option<String>> {
        let ledger = self.ledger_store(&discovered.id).load().await?;
        // project_id 的命名空间是按平台分的（Modrinth 是字符串 id，CurseForge 是数字 modId），
        // 只比 project_id 会让两个平台的 id 有机会撞车，故平台也一起比。
        //
        // 筛选就地收成一份文件名列表，而不是把惰性迭代器留到后面几个 await 之后再消费：借着卷宗的
        // 迭代器跨 await 会把那个闭包一并卷进本 future 的状态机，而 rustc 现有的高阶生命周期检查会把
        // 这类闭包的 `FnOnce` 实现判成「不够通用」（rust-lang/rust#102211 一族），导致本 future 在需要
        // `for<'r> Send` 的场合（`#[tauri::command]` 的包装）整个编译不过。行为与惰性版本完全一致。
        let candidates: Vec<&str> = ledger
            .entries
            .iter()
            .filter(|entry| entry.platform == platform && entry.project_id == project_id)
            .map(|entry| entry.file_name.as_str())
            .collect();
        if candidates.is_empty() {
            return Ok(None);
        }

        let working = self
            .resolve_working_dir_with(
                &discovered.id,
                discovered.has_mod_loader(),
                discovered.is_release(),
            )
            .await?;
        let mods_dir = working.working_dir.join(MODS_DIR);
        let exists =
            tokio::fs::try_exists(&mods_dir)
                .await
                .map_err(|source| aurora_base::Error::Io {
                    path: mods_dir.clone(),
                    source,
                })?;
        if !exists {
            return Ok(None);
        }

        let on_disk = scan_mods_dir(&mods_dir).await?;
        let present: HashSet<&str> = on_disk
            .iter()
            .map(|installed| {
                installed
                    .file_name
                    .strip_suffix(DISABLED_SUFFIX)
                    .unwrap_or(&installed.file_name)
            })
            .collect();
        Ok(candidates
            .into_iter()
            .find(|file_name| present.contains(file_name))
            .map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use crate::ledger::{Ledger, LedgerEntry, LedgerStore};
    use aurora_modplatform::ReleaseChannel;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn version(loaders: &[ModLoader], game_versions: &[&str]) -> ModVersionInfo {
        ModVersionInfo {
            version_id: "IZskiJmZ".to_owned(),
            project_id: "AANobbMI".to_owned(),
            platform: Platform::Modrinth,
            name: "Sodium 0.5.3".to_owned(),
            version_number: "mc1.20.1-0.5.3".to_owned(),
            release_channel: ReleaseChannel::Release,
            game_versions: game_versions.iter().map(|v| (*v).to_owned()).collect(),
            loaders: loaders.to_vec(),
            file_name: "sodium.jar".to_owned(),
            file_size: Some(204_800),
            sha1: Some("aabbcc".to_owned()),
            date_published: "2026-01-02T03:04:05Z".to_owned(),
            dependencies: Vec::new(),
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn full_metadata_hit_is_match() {
        let v = version(&[ModLoader::Fabric], &["1.20.1", "1.20.2"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["fabric"])),
            Compatibility::Match
        );
    }

    #[test]
    fn loader_name_comparison_is_case_insensitive() {
        let v = version(&[ModLoader::NeoForge], &["1.21"]);
        assert_eq!(
            classify(&v, "1.21", &names(&["NeoForge"])),
            Compatibility::Match
        );
        assert_eq!(
            classify(&v, "1.21", &names(&["neo-forge"])),
            Compatibility::Match
        );
    }

    #[test]
    fn multi_loader_version_matches_on_any_overlap() {
        // 版本同时支持 Fabric 与 Quilt，实例只有 Quilt：交集非空即通过。
        let v = version(&[ModLoader::Fabric, ModLoader::Quilt], &["1.20.1"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["quilt"])),
            Compatibility::Match
        );
    }

    #[test]
    fn vanilla_instance_rejects_loader_requiring_version() {
        let v = version(&[ModLoader::Forge], &["1.20.1"]);
        assert_eq!(
            classify(&v, "1.20.1", &[]),
            Compatibility::Mismatch {
                reason: "该实例未安装 Mod 加载器".to_owned()
            }
        );
    }

    #[test]
    fn optifine_only_instance_counts_as_no_mod_loader() {
        // OptiFine 不是 Mod 加载器：应给「未安装 Mod 加载器」，而不是列出它做对比。
        let v = version(&[ModLoader::Forge], &["1.20.1"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["optifine"])),
            Compatibility::Mismatch {
                reason: "该实例未安装 Mod 加载器".to_owned()
            }
        );
    }

    #[test]
    fn disjoint_loaders_name_both_sides_in_reason() {
        let v = version(&[ModLoader::Fabric, ModLoader::Quilt], &["1.20.1"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["forge"])),
            Compatibility::Mismatch {
                reason: "需要 Fabric/Quilt，该实例装的是 Forge".to_owned()
            }
        );
    }

    #[test]
    fn reason_uses_canonical_loader_spelling() {
        let v = version(&[ModLoader::NeoForge], &["1.21"]);
        let got = classify(&v, "1.21", &names(&["forge"]));
        // 文案里必须是 NeoForge / Forge 的规范写法，而非小写技术名。
        assert_eq!(
            got,
            Compatibility::Mismatch {
                reason: "需要 NeoForge，该实例装的是 Forge".to_owned()
            }
        );
    }

    #[test]
    fn loader_mismatch_wins_over_game_version_mismatch() {
        // 两维都对不上时先报加载器：加载器是更硬的门槛，换个 Mod 版本也解决不了。
        let v = version(&[ModLoader::Forge], &["1.19.2"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["fabric"])),
            Compatibility::Mismatch {
                reason: "需要 Forge，该实例装的是 Fabric".to_owned()
            }
        );
    }

    #[test]
    fn game_version_miss_reports_instance_version() {
        let v = version(&[ModLoader::Fabric], &["1.19.2", "1.19.4"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["fabric"])),
            Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            }
        );
    }

    #[test]
    fn game_version_match_is_exact_not_prefix() {
        // "1.20" 与 "1.20.1" 是两个 MC 版本，前缀相同不等于兼容。
        let v = version(&[ModLoader::Fabric], &["1.20"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["fabric"])),
            Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            }
        );
        let snapshot = version(&[ModLoader::Fabric], &["24w14a"]);
        assert_eq!(
            classify(&snapshot, "24w14a", &names(&["fabric"])),
            Compatibility::Match
        );
    }

    #[test]
    fn missing_game_versions_is_unknown_not_mismatch() {
        let v = version(&[ModLoader::Fabric], &[]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["fabric"])),
            Compatibility::Unknown
        );
    }

    #[test]
    fn missing_loaders_is_unknown_even_on_vanilla_instance() {
        // 平台没标加载器时不能反推「这是原版 Mod」，也不能反推「装不了」，只能说不知道。
        let v = version(&[], &["1.20.1"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["forge"])),
            Compatibility::Unknown
        );
        assert_eq!(classify(&v, "1.20.1", &[]), Compatibility::Unknown);
    }

    #[test]
    fn missing_loaders_still_rejects_wrong_game_version() {
        // 加载器缺失不该让 MC 版本这一维也失效：能判的仍要判。
        let v = version(&[], &["1.19.2"]);
        assert_eq!(
            classify(&v, "1.20.1", &names(&["forge"])),
            Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            }
        );
    }

    #[test]
    fn both_dimensions_missing_is_unknown() {
        let v = version(&[], &[]);
        assert_eq!(classify(&v, "1.20.1", &[]), Compatibility::Unknown);
    }

    #[test]
    fn compatibility_serializes_with_kind_tag() {
        assert_eq!(
            serde_json::to_value(Compatibility::Match).unwrap(),
            serde_json::json!({ "kind": "match" })
        );
        assert_eq!(
            serde_json::to_value(Compatibility::Unknown).unwrap(),
            serde_json::json!({ "kind": "unknown" })
        );
        assert_eq!(
            serde_json::to_value(Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "kind": "mismatch", "reason": "不支持 MC 1.20.1" })
        );
    }

    #[test]
    fn instance_match_serializes_with_ipc_field_names() {
        let entry = InstanceMatch {
            version_id: "1.20.1-fabric".to_owned(),
            mc_version: "1.20.1".to_owned(),
            loaders: names(&["fabric"]),
            compatibility: Compatibility::Match,
            best_version: Some(version(&[ModLoader::Fabric], &["1.20.1"])),
            already_installed: Some("sodium.jar".to_owned()),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["version_id"], "1.20.1-fabric");
        assert_eq!(json["mc_version"], "1.20.1");
        assert_eq!(json["loaders"][0], "fabric");
        assert_eq!(json["compatibility"]["kind"], "match");
        assert_eq!(json["best_version"]["version_id"], "IZskiJmZ");
        assert_eq!(json["already_installed"], "sodium.jar");

        let not_installed = InstanceMatch {
            best_version: None,
            already_installed: None,
            ..entry
        };
        let json = serde_json::to_value(&not_installed).unwrap();
        assert_eq!(json["best_version"], serde_json::Value::Null);
        assert_eq!(json["already_installed"], serde_json::Value::Null);
    }

    // ---- 择优与实例事实抽取 ----

    /// 造一个指定发布年份的版本，其余维度由调用方给定。择优只关心相对新旧，年份足以区分。
    fn dated(id: &str, year: u16, loaders: &[ModLoader], mc: &[&str]) -> ModVersionInfo {
        ModVersionInfo {
            version_id: id.to_owned(),
            date_published: format!("{year}-06-01T00:00:00Z"),
            file_name: format!("{id}.jar"),
            ..version(loaders, mc)
        }
    }

    #[test]
    fn pick_best_takes_newest_matching_version_not_merely_first_compatible() {
        // 倒序列表：新版匹配就该选新版，即便后面还有同样匹配的旧版。
        let versions = vec![
            dated("newest", 2026, &[ModLoader::Fabric], &["1.20.1"]),
            dated("older", 2025, &[ModLoader::Fabric], &["1.20.1"]),
        ];
        let (compat, best) = pick_best(&versions, "1.20.1", &names(&["fabric"]));
        assert_eq!(compat, Compatibility::Match);
        assert_eq!(best.unwrap().version_id, "newest");
    }

    #[test]
    fn pick_best_skips_newer_mismatches_to_reach_a_match() {
        let versions = vec![
            dated("too-new", 2026, &[ModLoader::Fabric], &["1.21"]),
            dated("fits", 2025, &[ModLoader::Fabric], &["1.20.1"]),
        ];
        let (compat, best) = pick_best(&versions, "1.20.1", &names(&["fabric"]));
        assert_eq!(compat, Compatibility::Match);
        assert_eq!(best.unwrap().version_id, "fits");
    }

    #[test]
    fn pick_best_prefers_certain_match_over_newer_unknown() {
        // 元数据缺失的新版只是「可能可行」，确定能装的旧版才是更该默认选中的那个。
        let versions = vec![
            dated("no-metadata", 2026, &[], &[]),
            dated("certain", 2025, &[ModLoader::Fabric], &["1.20.1"]),
        ];
        let (compat, best) = pick_best(&versions, "1.20.1", &names(&["fabric"]));
        assert_eq!(compat, Compatibility::Match);
        assert_eq!(best.unwrap().version_id, "certain");
    }

    #[test]
    fn pick_best_falls_back_to_unknown_when_nothing_matches() {
        let versions = vec![
            dated("wrong-mc", 2026, &[ModLoader::Fabric], &["1.21"]),
            dated("no-loaders", 2025, &[], &["1.20.1"]),
            dated("older-unknown", 2024, &[], &["1.20.1"]),
        ];
        let (compat, best) = pick_best(&versions, "1.20.1", &names(&["fabric"]));
        assert_eq!(compat, Compatibility::Unknown);
        // 软提示也要给最新的那个候选。
        assert_eq!(best.unwrap().version_id, "no-loaders");
    }

    #[test]
    fn pick_best_reports_newest_versions_reason_when_all_mismatch() {
        let versions = vec![
            dated("newest", 2026, &[ModLoader::Fabric], &["1.21"]),
            dated("ancient", 2019, &[ModLoader::Forge], &["1.12.2"]),
        ];
        let (compat, best) = pick_best(&versions, "1.20.1", &names(&["fabric"]));
        assert_eq!(
            compat,
            Compatibility::Mismatch {
                reason: "不支持 MC 1.20.1".to_owned()
            }
        );
        // 不兼容时不给候选版本：让 UI 无从把一个装不上的版本渲染成可安装项。
        assert!(best.is_none());
    }

    #[test]
    fn pick_best_on_empty_version_list_states_the_project_has_none() {
        let (compat, best) = pick_best(&[], "1.20.1", &names(&["fabric"]));
        assert_eq!(
            compat,
            Compatibility::Mismatch {
                reason: "该工程没有可用版本".to_owned()
            }
        );
        assert!(best.is_none());
    }

    /// 用一段版本 JSON 造出发现结果（路径不参与本组断言，取占位值）。
    fn discovered(id: &str, json: &str) -> DiscoveredVersion {
        let parsed = aurora_version::VersionJson::from_json_str(json).expect("测试版本 JSON 合法");
        let loaders = aurora_version::detect_loaders(&parsed);
        let dir = std::path::PathBuf::from("versions").join(id);
        DiscoveredVersion {
            id: id.to_owned(),
            json_path: dir.join(format!("{id}.json")),
            jar_path: dir.join(format!("{id}.jar")),
            dir,
            json: parsed,
            loaders,
        }
    }

    #[test]
    fn instance_mc_version_prefers_inherits_from() {
        let v = discovered(
            "fabric-loader-0.15.11-1.20.1",
            r#"{"id":"fabric-loader-0.15.11-1.20.1","inheritsFrom":"1.20.1","type":"release",
                "libraries":[{"name":"net.fabricmc:fabric-loader:0.15.11"}]}"#,
        );
        assert_eq!(instance_mc_version(&v), "1.20.1");
    }

    #[test]
    fn instance_mc_version_falls_back_to_id_when_inherits_absent_or_blank() {
        let vanilla = discovered("1.21", r#"{"id":"1.21","type":"release"}"#);
        assert_eq!(instance_mc_version(&vanilla), "1.21");

        // 空串不能当成有效父版本，否则该实例的 MC 版本会变成空串。
        let blank = discovered(
            "24w14a",
            r#"{"id":"24w14a","inheritsFrom":"","type":"snapshot"}"#,
        );
        assert_eq!(instance_mc_version(&blank), "24w14a");
    }

    #[test]
    fn instance_loaders_are_lowercase_kind_names() {
        let v = discovered(
            "1.20.1-forge-optifine",
            r#"{"id":"1.20.1-forge-optifine","inheritsFrom":"1.20.1","type":"release",
                "libraries":[{"name":"net.minecraftforge:forge:1.20.1-47.2.0"},
                             {"name":"optifine:OptiFine:1.20.1_HD_U_I6"}]}"#,
        );
        // 探测顺序固定为 Forge 先于 OptiFine；非 Mod 加载器也如实列出，取舍交给 classify。
        assert_eq!(instance_loaders(&v), names(&["forge", "optifine"]));

        let vanilla = discovered("1.21", r#"{"id":"1.21","type":"release"}"#);
        assert!(instance_loaders(&vanilla).is_empty());
    }

    // ---- 实例匹配矩阵（端到端，平台走本地 mock）----

    /// 各加载器的特征库坐标，[`aurora_version::detect_loaders`] 据此识别实例装了什么。
    const FABRIC_LIB: &str = "net.fabricmc:fabric-loader:0.15.11";
    const QUILT_LIB: &str = "org.quiltmc:quilt-loader:0.26.0";
    const FORGE_LIB: &str = "net.minecraftforge:forge:1.20.1-47.2.0";

    /// 在 versions/<id>/<id>.json 落一份带指定父版本与加载器库的版本 JSON。
    async fn put_instance(mc: &std::path::Path, id: &str, inherits: &str, library: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(
            dir.join(format!("{id}.json")),
            format!(
                r#"{{"id":"{id}","inheritsFrom":"{inherits}","type":"release","mainClass":"m",
                     "libraries":[{{"name":"{library}"}}]}}"#
            ),
        )
        .await
        .unwrap();
    }

    /// 给某实例写一条 sodium 的安装记录，并在其 mods 目录里落下 `disk_files` 指定的文件。
    async fn put_ledger_and_disk(mc: &std::path::Path, id: &str, disk_files: &[&str]) {
        let version_dir = mc.join("versions").join(id);
        let mut ledger = Ledger::default();
        ledger.upsert(LedgerEntry {
            file_name: "sodium-0.5.jar".to_owned(),
            platform: Platform::Modrinth,
            project_id: "sodium".to_owned(),
            version_id: "new".to_owned(),
            sha1: None,
            installed_at: 1_754_000_000,
            installed_as_dependency_of: None,
        });
        LedgerStore::for_version_dir(&version_dir)
            .save(&ledger)
            .await
            .unwrap();

        let mods_dir = version_dir.join("mods");
        tokio::fs::create_dir_all(&mods_dir).await.unwrap();
        for name in disk_files {
            tokio::fs::write(mods_dir.join(name), b"jar").await.unwrap();
        }
    }

    /// 两个版本的工程：新版支持 Fabric/Quilt + 1.20.1，旧版没标加载器、只标 1.19.2。
    const SODIUM_VERSIONS: &str = r#"[
        {"id":"new","project_id":"sodium","name":"Sodium 0.5","version_number":"0.5",
         "version_type":"release","date_published":"2026-02-01T00:00:00Z",
         "game_versions":["1.20.1"],"loaders":["fabric","quilt"],
         "files":[{"hashes":{"sha1":"aa"},"url":"https://example.invalid/sodium-0.5.jar",
                   "filename":"sodium-0.5.jar","primary":true,"size":3}]},
        {"id":"old","project_id":"sodium","name":"Sodium 0.4","version_number":"0.4",
         "version_type":"release","date_published":"2025-01-01T00:00:00Z",
         "game_versions":["1.19.2"],
         "files":[{"hashes":{"sha1":"bb"},"url":"https://example.invalid/sodium-0.4.jar",
                   "filename":"sodium-0.4.jar","primary":true,"size":3}]}
    ]"#;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn match_instances_ranks_three_tiers_and_joins_ledger_against_disk() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/sodium/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SODIUM_VERSIONS))
            // 版本列表整场只能拉一次：改成逐实例发请求，这条期望即挂（四个实例会打四次）。
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        // id 刻意让「不兼容/可能可行」的字典序排在「完美匹配」之前，逼出档位优先于字典序。
        put_instance(&mc, "a-fabric-1.19.2", "1.19.2", FABRIC_LIB).await;
        put_instance(&mc, "a-quilt-1.20.1", "1.20.1", QUILT_LIB).await;
        put_instance(&mc, "b-fabric-1.20.1", "1.20.1", FABRIC_LIB).await;
        put_instance(&mc, "c-forge-1.20.1", "1.20.1", FORGE_LIB).await;

        // 卷宗有、磁盘没有：磁盘是权威，判为未安装。
        put_ledger_and_disk(&mc, "a-quilt-1.20.1", &["other.jar"]).await;
        // 卷宗与磁盘都有：已安装。
        put_ledger_and_disk(&mc, "b-fabric-1.20.1", &["sodium-0.5.jar"]).await;
        // 禁用态仍算已装，只是被关掉了。
        put_ledger_and_disk(&mc, "c-forge-1.20.1", &["sodium-0.5.jar.disabled"]).await;

        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc.clone())
            .with_modrinth_base(server.uri());
        let matches = aurora
            .match_instances(Platform::Modrinth, "sodium")
            .await
            .unwrap();

        let ids: Vec<&str> = matches.iter().map(|m| m.version_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "a-quilt-1.20.1",
                "b-fabric-1.20.1",
                "a-fabric-1.19.2",
                "c-forge-1.20.1"
            ]
        );

        // 第一档：Quilt 实例命中新版（工程同时支持 Fabric/Quilt）。
        assert_eq!(matches[0].compatibility, Compatibility::Match);
        assert_eq!(matches[0].mc_version, "1.20.1");
        assert_eq!(matches[0].loaders, names(&["quilt"]));
        assert_eq!(matches[0].best_version.as_ref().unwrap().version_id, "new");
        assert_eq!(matches[0].already_installed, None);

        assert_eq!(matches[1].compatibility, Compatibility::Match);
        assert_eq!(
            matches[1].best_version.as_ref().unwrap().file_name,
            "sodium-0.5.jar"
        );
        assert_eq!(
            matches[1].already_installed.as_deref(),
            Some("sodium-0.5.jar")
        );

        // 第二档：1.19.2 实例只有旧版能沾边，而旧版没标加载器 -> 软提示放行。
        assert_eq!(matches[2].compatibility, Compatibility::Unknown);
        assert_eq!(matches[2].mc_version, "1.19.2");
        assert_eq!(matches[2].best_version.as_ref().unwrap().version_id, "old");
        assert_eq!(matches[2].already_installed, None);

        // 第三档：Forge 实例两个版本都对不上，说明取自最新版的原因。
        assert_eq!(
            matches[3].compatibility,
            Compatibility::Mismatch {
                reason: "需要 Fabric/Quilt，该实例装的是 Forge".to_owned()
            }
        );
        assert!(matches[3].best_version.is_none());
        assert_eq!(
            matches[3].already_installed.as_deref(),
            Some("sodium-0.5.jar")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn match_instances_without_any_instance_skips_the_network_entirely() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/sodium/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string(SODIUM_VERSIONS))
            // 没有实例就没有判定对象，一次请求都不该发。
            .expect(0)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path().to_path_buf();
        let aurora = Aurora::for_test(AuroraConfig::default(), mc.clone(), mc)
            .with_modrinth_base(server.uri());

        assert!(
            aurora
                .match_instances(Platform::Modrinth, "sodium")
                .await
                .unwrap()
                .is_empty()
        );
    }
}
