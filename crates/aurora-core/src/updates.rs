//! 存量身份反查与更新检查。
//!
//! 更新检查的前提是知道「磁盘上这个 jar 是谁」。经本启动器装的 Mod 在卷宗里有身份，直接精确查；
//! 手动丢进 `mods/` 的、老启动器留下的、整合包解出来的则没有——这类要靠文件哈希反查：Modrinth 用
//! SHA-1、CurseForge 用 MurmurHash2 指纹，两条通道各查一次，查到就把身份补回卷宗。
//!
//! 反查不到不算错误。本地自制 Mod、私有构建、改名过的 jar 本就没有平台来源，报错只会把「我有几个
//! 私货」变成「更新检查整个不能用」。这类文件安静跳过，下次也不必重试。
//!
//! 原版实例（未装任何 Mod 加载器）直接返回空：没有加载器就不会有 Mod 在跑，扫它纯属浪费一轮网络
//! 请求。这条规则抄自 Prism。

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use aurora_instance::discover_versions;
use aurora_modplatform::{
    CurseForgeClient, CurseForgeFile, CurseForgeFingerprintMatch, InstalledMod, ModHashes,
    ModLoader, ModVersionInfo, ModrinthClient, Platform, hash_mod_file, parse_loader_name,
};
use aurora_version::identify_mc_version;
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::facade::Aurora;
use crate::ledger::{Ledger, LedgerEntry};
use crate::modversions::sort_by_published_desc;

/// 磁盘上禁用态 Mod 的文件名后缀。aurora-modplatform 用它实现启禁切换但未导出常量，故在此重述。
const DISABLED_SUFFIX: &str = ".disabled";

/// 一个可更新项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCandidate {
    /// 启用态文件名，即与卷宗 join 的键（磁盘上处于禁用态时会多一个 `.disabled` 后缀）。
    pub file_name: String,
    /// 当前已装的版本标识。
    pub current_version_id: String,
    /// 该实例可用的最新版本。
    pub latest: ModVersionInfo,
}

impl Aurora {
    /// 对卷宗里没有身份的已装 Mod 做哈希反查补身份，返回补上的条数。
    ///
    /// 只处理「磁盘上有、卷宗里没有」的文件：已有身份的重算哈希再联网查一遍毫无收益。反查不到的
    /// 静默跳过，因此本方法可以反复调用，第二次起对同一批私货文件不再产生写入。
    pub async fn identify_installed_mods(&self, version_id: &str) -> Result<usize> {
        let installed = self.list_mods(version_id).await?;
        let store = self.ledger_store(version_id);
        let mut ledger = store.load().await?;

        let added = backfill_identities(self, &installed, &mut ledger).await?;
        // 一条都没补上就不落盘：不该为「什么都没查到」凭空生成一份空卷宗文件。
        if added > 0 {
            store.save(&ledger).await?;
        }
        Ok(added)
    }

    /// 检查该实例可更新的 Mod。
    ///
    /// 有身份走精确查，无身份先做一轮哈希反查兜底（与 [`Aurora::identify_installed_mods`] 同一套逻辑，
    /// 共用一次目录扫描）。返回顺序按文件名字典序，保证 UI 列表稳定。
    pub async fn check_updates(&self, version_id: &str) -> Result<Vec<UpdateCandidate>> {
        let facts = instance_mod_facts(self, version_id).await?;
        // 原版实例（以及只装了 OptiFine 的实例）不会有 Mod 在跑，查更新纯属浪费网络请求。
        if facts.loaders.is_empty() {
            return Ok(Vec::new());
        }
        // 认不出实例的 MC 版本就没有兼容判定的基准，任何「最新版」都只是猜测（自造 id、无
        // inheritsFrom 又无版本线索的整合版本 JSON 会落到这里）。宁可不报，也不推一个可能启动不了的包。
        let Some(mc_version) = facts.mc_version else {
            return Ok(Vec::new());
        };

        let installed = self.list_mods(version_id).await?;
        let store = self.ledger_store(version_id);
        let mut ledger = store.load().await?;
        // 先补身份：手动丢进 mods/ 的 jar 没有卷宗记录，不补就永远查不到它的更新。
        if backfill_identities(self, &installed, &mut ledger).await? > 0 {
            store.save(&ledger).await?;
        }

        // 磁盘是权威：待查集合来自重扫结果，并按启用态文件名去重——同名的启用副本与 `.disabled`
        // 副本指向同一条卷宗身份，不该产出两条重复的更新项。
        let present: BTreeSet<String> = installed
            .iter()
            .map(|item| ledger_key(&item.file_name).to_owned())
            .collect();

        let game_versions = [mc_version];
        let mut candidates = Vec::new();

        // Modrinth 侧走批量端点：一次请求覆盖整个实例。此前是「每个 Mod 查一次工程版本列表」，
        // 几十个 Mod 的实例一进来就是几十次请求，几个实例连着查立刻被限流（HTTP 429）。
        let mut modrinth_by_hash: HashMap<String, (String, LedgerEntry)> = HashMap::new();
        // CurseForge 没有等价的「按指纹批量查更新」端点，仍逐个走工程文件列表。
        let mut curseforge_targets: Vec<(String, LedgerEntry)> = Vec::new();

        for file_name in present {
            // 卷宗里查不到身份：两条反查通道都没认出来的本地 Mod，无从判断它有没有新版。
            let Some(entry) = ledger.find(&file_name) else {
                continue;
            };
            match entry.platform {
                Platform::Modrinth => {
                    // 批量端点按 SHA-1 索引。卷宗里没记哈希的老记录就地补算（纯本地 IO，不触网）；
                    // 文件读不出来（被占用、刚被删）就跳过这一个，不牵连整批。
                    let sha1 = match &entry.sha1 {
                        Some(sha1) => sha1.clone(),
                        None => match sha1_of_installed(&installed, &file_name).await {
                            Some(sha1) => sha1,
                            None => continue,
                        },
                    };
                    modrinth_by_hash.insert(sha1, (file_name, entry.clone()));
                }
                Platform::CurseForge => curseforge_targets.push((file_name, entry.clone())),
            }
        }

        if !modrinth_by_hash.is_empty() {
            let hashes: Vec<String> = modrinth_by_hash.keys().cloned().collect();
            let client = ModrinthClient::new(self.http()).with_base_url(self.modrinth_base());
            let latest = client
                .latest_versions_by_hashes(&hashes, &facts.loaders, &[game_versions[0].as_str()])
                .await?;
            for (hash, version) in latest {
                let Some((file_name, entry)) = modrinth_by_hash.get(&hash) else {
                    continue;
                };
                // 端点返回的就是该文件在本实例加载器/版本下的最新兼容版本，可能正是当前这个。
                if version.id == entry.version_id {
                    continue;
                }
                let Some(info) = version.to_version_info() else {
                    continue;
                };
                candidates.push(UpdateCandidate {
                    current_version_id: entry.version_id.clone(),
                    latest: info,
                    file_name: file_name.clone(),
                });
            }
        }

        for (file_name, entry) in curseforge_targets {
            let mut versions = self
                .list_mod_versions(
                    entry.platform,
                    &entry.project_id,
                    &game_versions,
                    &facts.loaders,
                )
                .await?;
            sort_by_published_desc(&mut versions);
            let Some(latest) = versions.first() else {
                continue;
            };
            if latest.version_id == entry.version_id {
                continue;
            }
            // 新旧一律按发布时间比，绝不比版本号字符串：两个平台的版本号各家一套格式
            // （`mc1.20.1-0.5.3`、`0.5.3+1.20.1`，CurseForge 干脆拿文件名当版本号），
            // 字典序比出来的「更新」随时可能是一次降级。
            let Some(current) = versions.iter().find(|v| v.version_id == entry.version_id) else {
                // 当前装的版本不在该实例的兼容列表里（平台下架，或当初就装错了版本）：
                // 拿不到它的发布时间就无从证明「更新」，不猜。
                continue;
            };
            if latest.date_published.is_empty() || latest.date_published <= current.date_published {
                continue;
            }

            candidates.push(UpdateCandidate {
                current_version_id: entry.version_id.clone(),
                latest: latest.clone(),
                file_name,
            });
        }

        // 两条通道的结果合到一起，按文件名排序保证 UI 列表稳定。
        candidates.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        Ok(candidates)
    }
}

/// 就地补算某个已装文件的 SHA-1（卷宗里没记哈希的老记录会走到这）。
///
/// 读不出文件返回 `None`：文件可能刚被删或被占用，这只该让它一个查不了更新，不该让整批失败。
async fn sha1_of_installed(installed: &[InstalledMod], file_name: &str) -> Option<String> {
    let item = installed
        .iter()
        .find(|m| ledger_key(&m.file_name) == file_name)?;
    hash_mod_file(Path::new(&item.path)).await.ok().map(|h| h.sha1)
}

/// 更新检查所需的实例事实。
struct InstanceModFacts {
    /// 实例对应的原版 MC 版本；识别不出为 `None`。
    mc_version: Option<String>,
    /// 实例装的 Mod 加载器（OptiFine 这类非 Mod 加载器已剔除）。
    loaders: Vec<ModLoader>,
}

/// 读出某已装实例的 MC 版本与 Mod 加载器；版本本地未安装返回 [`CoreError::VersionNotInstalled`]。
///
/// 写成自由函数而不是 `impl Aurora` 的方法：这组事实只有本模块用得上，不必在门面上多开一个公开面。
async fn instance_mod_facts(aurora: &Aurora, version_id: &str) -> Result<InstanceModFacts> {
    let scan = discover_versions(aurora.game_dir()).await?;
    let target = scan
        .versions
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| CoreError::VersionNotInstalled {
            id: version_id.to_owned(),
        })?;

    Ok(InstanceModFacts {
        mc_version: identify_mc_version(&target.json).value,
        // 认不出的加载器名一律丢弃，口径与 compat::classify 一致：OptiFine 不是 Mod 加载器，
        // 把它算进来会让「原版 + OptiFine」的实例被误判成能装 Mod。
        loaders: target
            .loaders
            .iter()
            .filter_map(|info| parse_loader_name(info.kind.as_str()))
            .collect(),
    })
}

/// 给 `installed` 里尚无身份的文件做哈希反查并写进 `ledger`，返回补上的条数（不落盘）。
///
/// 两条通道有先后：Modrinth 按 SHA-1 逐个精确查，剩下认不出的才合成一次 CurseForge 指纹批量匹配。
/// 反过来先打 CurseForge 会让绝大多数 Modrinth 用户白白多发一个需要 API key 的请求。
async fn backfill_identities(
    aurora: &Aurora,
    installed: &[InstalledMod],
    ledger: &mut Ledger,
) -> Result<usize> {
    let unknown: Vec<&InstalledMod> = installed
        .iter()
        .filter(|item| ledger.find(ledger_key(&item.file_name)).is_none())
        .collect();
    if unknown.is_empty() {
        return Ok(0);
    }

    let modrinth = ModrinthClient::new(aurora.http()).with_base_url(aurora.modrinth_base());
    let mut probed: Vec<(&InstalledMod, ModHashes, Option<ModVersionInfo>)> =
        Vec::with_capacity(unknown.len());
    for item in unknown {
        let hashes = hash_mod_file(&item.path).await?;
        let hit = modrinth
            .version_by_hash(&hashes.sha1)
            .await?
            .and_then(|version| version.to_version_info());
        probed.push((item, hashes, hit));
    }

    let pending: Vec<u32> = probed
        .iter()
        .filter(|(_, _, hit)| hit.is_none())
        .map(|(_, hashes, _)| hashes.curseforge_fingerprint)
        .collect();
    let curseforge = curseforge_fingerprint_lookup(aurora, &pending).await?;

    let mut added = 0usize;
    for (item, hashes, hit) in probed {
        let info = match hit {
            Some(info) => info,
            None => match curseforge.get(&hashes.curseforge_fingerprint) {
                Some(file) => file.to_version_info(),
                // 两条通道都认不出：本地自制 Mod、私有构建、改过名的 jar 本就没有平台来源。
                None => continue,
            },
        };
        ledger.upsert(LedgerEntry {
            file_name: ledger_key(&item.file_name).to_owned(),
            platform: info.platform,
            project_id: info.project_id,
            version_id: info.version_id,
            // 记本地实测的 SHA-1 而非平台声明值：卷宗里的哈希用来校验「磁盘上这个文件」。
            sha1: Some(hashes.sha1),
            installed_at: file_mtime_unix(&item.path).await?,
            // 反查只能确定「这个文件是谁」，无从得知它当初是主动装的还是被依赖带进来的。
            installed_as_dependency_of: None,
        });
        added += 1;
    }
    Ok(added)
}

/// 批量指纹反查，返回「指纹 -> 命中文件」。
///
/// 未配置 CurseForge API key 时该通道整体不可用，返回空表——沿用聚合搜索里「无 key 即禁用该源」
/// 的既有约定：缺一把可选的钥匙不该连带毁掉 Modrinth 通道。其它错误照常冒泡。
async fn curseforge_fingerprint_lookup(
    aurora: &Aurora,
    fingerprints: &[u32],
) -> Result<HashMap<u32, CurseForgeFile>> {
    if fingerprints.is_empty() {
        return Ok(HashMap::new());
    }
    let client = match CurseForgeClient::from_env(aurora.http()) {
        Ok(client) => client.with_base_url(aurora.curseforge_base()),
        Err(aurora_modplatform::Error::CurseForgeKeyMissing) => return Ok(HashMap::new()),
        Err(other) => return Err(other.into()),
    };
    Ok(index_by_fingerprint(
        client.fingerprint_matches(fingerprints).await?,
    ))
}

/// 把指纹匹配结果整理成「指纹 -> 命中文件」。
///
/// 只能按指纹 join：`exactMatches` 既不保证与请求同序，也可能比请求少（没收录的文件不会回）。
/// 指纹在请求侧是 32 位，装不进 `u32` 的返回值不可能来自本次请求，丢弃而不是截断——截断会把
/// 一个陌生文件的身份错安到别的 jar 上。
fn index_by_fingerprint(matches: Vec<CurseForgeFingerprintMatch>) -> HashMap<u32, CurseForgeFile> {
    matches
        .into_iter()
        .filter_map(|hit| {
            u32::try_from(hit.file.file_fingerprint)
                .ok()
                .map(|fingerprint| (fingerprint, hit.file))
        })
        .collect()
}

/// 取与卷宗 join 的键：卷宗记的是启用态文件名，磁盘上禁用态会多一个 `.disabled` 后缀。
fn ledger_key(file_name: &str) -> &str {
    file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name)
}

/// 文件最后修改时刻（unix 秒）。
///
/// 反查补身份时的 `installed_at` 取这个值而不是「现在」：真实安装时刻早已不可考，落盘时间是唯一
/// 与它相关的事实。写「现在」会让卷宗声称一个两年前装的 Mod 是刚装的，UI 按安装时间排序即失真。
async fn file_mtime_unix(path: &Path) -> Result<u64> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|source| aurora_base::Error::Io {
            path: path.to_owned(),
            source,
        })?;
    let modified = metadata.modified().map_err(|source| aurora_base::Error::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(modified
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        // 早于 1970 的 mtime 只可能来自坏掉的文件元数据。该时间戳纯供展示排序，用 0 表示「不可考」，
        // 不值得为它让整轮身份补全失败。
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use aurora_instance::IsolationPolicy;
    use aurora_modplatform::{Platform, ReleaseChannel};
    use sha1::{Digest, Sha1};
    use std::path::PathBuf;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Fabric 实例：inheritsFrom 给出 MC 版本，fabric-loader 库触发加载器探测。
    const FABRIC_JSON: &str = r#"{"id":"1.20.1-fabric","inheritsFrom":"1.20.1","type":"release",
        "mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient",
        "libraries":[{"name":"net.fabricmc:fabric-loader:0.15.11"}]}"#;
    /// 原版实例：无任何加载器库。
    const VANILLA_JSON: &str =
        r#"{"id":"1.20.1","type":"release","mainClass":"m","libraries":[]}"#;

    async fn put_version(mc: &Path, id: &str, json: &str) {
        let dir = mc.join("versions").join(id);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(format!("{id}.json")), json)
            .await
            .unwrap();
    }

    /// 往实例的隔离 mods 目录放一个 jar，返回 (路径, sha1)。
    async fn put_mod(mc: &Path, version_id: &str, file_name: &str, bytes: &[u8]) -> (PathBuf, String) {
        let dir = mc.join("versions").join(version_id).join("mods");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(file_name);
        tokio::fs::write(&path, bytes).await.unwrap();

        let mut hasher = Sha1::new();
        hasher.update(bytes);
        let sha1 = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        (path, sha1)
    }

    fn aurora_at(mc: &Path, base: &str) -> Aurora {
        let mut aurora =
            Aurora::for_test(AuroraConfig::default(), mc.to_path_buf(), mc.to_path_buf());
        aurora.set_isolation_policy(IsolationPolicy::All);
        // CurseForge 端点也指向 mock：测试机若恰好配了 API key，也不会打到生产端点。
        aurora.with_modrinth_base(base).with_curseforge_base(base)
    }

    fn ledger_entry(file_name: &str, project_id: &str, version_id: &str) -> LedgerEntry {
        LedgerEntry {
            file_name: file_name.to_owned(),
            platform: Platform::Modrinth,
            project_id: project_id.to_owned(),
            version_id: version_id.to_owned(),
            sha1: None,
            installed_at: 1_700_000_000,
            installed_as_dependency_of: None,
        }
    }

    async fn save_ledger(aurora: &Aurora, version_id: &str, entries: Vec<LedgerEntry>) {
        let mut ledger = Ledger::default();
        for entry in entries {
            ledger.upsert(entry);
        }
        aurora
            .ledger_store(version_id)
            .save(&ledger)
            .await
            .unwrap();
    }

    /// 一条 Modrinth 版本 JSON（含单个主文件）。
    fn modrinth_version(
        id: &str,
        project_id: &str,
        version_number: &str,
        date: &str,
        file_name: &str,
        sha1: &str,
    ) -> String {
        format!(
            r#"{{"id":"{id}","project_id":"{project_id}","name":"{file_name}",
                "version_number":"{version_number}","version_type":"release",
                "date_published":"{date}","game_versions":["1.20.1"],"loaders":["fabric"],
                "files":[{{"hashes":{{"sha1":"{sha1}"}},"url":"https://example.invalid/{file_name}",
                    "filename":"{file_name}","primary":true,"size":1024}}]}}"#
        )
    }

    /// 把文件的 mtime 改成指定 unix 秒（验证 installed_at 取的是落盘时间而非「现在」）。
    fn set_mtime(path: &Path, unix_secs: u64) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(unix_secs);
        file.set_times(std::fs::FileTimes::new().set_modified(when))
            .unwrap();
    }

    /// 卷宗里已有身份的 Mod：命中更新时如实给出「从哪个版本到哪个版本」。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_reports_newer_version_for_known_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        let (_, sha1) = put_mod(mc, "1.20.1-fabric", "sodium-0.5.3.jar", b"sodium-payload").await;

        // Modrinth 侧走批量端点：一次 POST 带走整批哈希，响应按哈希索引。
        let server = MockServer::start().await;
        let body = format!(
            r#"{{"{sha1}":{}}}"#,
            modrinth_version(
                "new2",
                "AANobbMI",
                "0.6.0",
                "2026-06-01T00:00:00Z",
                "sodium-0.6.0.jar",
                "bb"
            ),
        );
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        save_ledger(
            &aurora,
            "1.20.1-fabric",
            vec![ledger_entry("sodium-0.5.3.jar", "AANobbMI", "old1")],
        )
        .await;

        let found = aurora.check_updates("1.20.1-fabric").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "sodium-0.5.3.jar");
        assert_eq!(found[0].current_version_id, "old1");
        assert_eq!(found[0].latest.version_id, "new2");
        assert_eq!(found[0].latest.file_name, "sodium-0.6.0.jar");
        assert_eq!(found[0].latest.release_channel, ReleaseChannel::Release);
        assert_eq!(found[0].latest.date_published, "2026-06-01T00:00:00Z");

        // 整个实例只该打一次更新查询，而不是每个 Mod 一次——这正是限流的成因。
        let posts = server
            .received_requests()
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.url.path() == "/version_files/update")
            .count();
        assert_eq!(posts, 1);
    }

    /// 批量端点返回的就是当前已装的那个版本：说明已是最新，不该报更新。
    ///
    /// 端点语义是「该文件在指定加载器与游戏版本下的最新兼容版本」，所以 Modrinth 侧不再自己比
    /// 发布时间，只比版本 id。按发布时间而非版本号串比较的逻辑仍用在 CurseForge 侧，
    /// 但那条路径要求 AURORA_CURSEFORGE_API_KEY，而环境变量是进程全局的、并行测试里会互相干扰，
    /// 故暂无自动化覆盖——改动 CurseForge 分支时需人工留意这条判定。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_skips_when_batch_returns_same_version() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        let (_, sha1) = put_mod(mc, "1.20.1-fabric", "lithium-0.9.0.jar", b"lithium-payload").await;

        let server = MockServer::start().await;
        let body = format!(
            r#"{{"{sha1}":{}}}"#,
            modrinth_version(
                "v090",
                "AANobbMI",
                "0.9.0",
                "2026-06-01T00:00:00Z",
                "lithium-0.9.0.jar",
                "aa"
            ),
        );
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        save_ledger(
            &aurora,
            "1.20.1-fabric",
            vec![ledger_entry("lithium-0.9.0.jar", "AANobbMI", "v090")],
        )
        .await;

        assert!(aurora.check_updates("1.20.1-fabric").await.unwrap().is_empty());
    }

    /// 响应里没有某个哈希：代表该文件无更新（或平台未收录），不是错误，也不该报更新。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_treats_missing_hash_in_response_as_no_update() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        put_mod(mc, "1.20.1-fabric", "sodium-0.5.3.jar", b"sodium-payload").await;

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        save_ledger(
            &aurora,
            "1.20.1-fabric",
            vec![ledger_entry("sodium-0.5.3.jar", "AANobbMI", "old1")],
        )
        .await;

        assert!(aurora.check_updates("1.20.1-fabric").await.unwrap().is_empty());
    }

    /// 原版实例：直接返回空，且一个请求都不发（没有加载器就不会有 Mod 在跑）。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_on_vanilla_instance_is_empty_without_any_request() {
        // 不挂任何 mock：真触网就是 404，会让断言以错误形式暴露。
        let server = MockServer::start().await;

        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1", VANILLA_JSON).await;
        put_mod(mc, "1.20.1", "sodium.jar", b"sodium-payload").await;
        let aurora = aurora_at(mc, &server.uri());
        save_ledger(
            &aurora,
            "1.20.1",
            vec![ledger_entry("sodium.jar", "AANobbMI", "old1")],
        )
        .await;

        assert!(aurora.check_updates("1.20.1").await.unwrap().is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    /// 手动丢进 mods/ 的 jar：先哈希反查补上身份，再据此查到更新，同时身份被写回卷宗。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_identifies_unknown_mod_by_hash_then_finds_update() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        let (jar, sha1) = put_mod(mc, "1.20.1-fabric", "mystery.jar", b"mystery-payload").await;
        set_mtime(&jar, 1_600_000_000);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/version_file/{sha1}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(modrinth_version(
                "old1",
                "AANobbMI",
                "0.5.3",
                "2025-01-01T00:00:00Z",
                "mystery.jar",
                &sha1,
            )))
            .mount(&server)
            .await;
        // 补上身份之后，更新查询同样走批量端点。
        let body = format!(
            r#"{{"{sha1}":{}}}"#,
            modrinth_version(
                "new2",
                "AANobbMI",
                "0.6.0",
                "2026-06-01T00:00:00Z",
                "mystery-0.6.0.jar",
                "bb"
            ),
        );
        Mock::given(method("POST"))
            .and(path("/version_files/update"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        let found = aurora.check_updates("1.20.1-fabric").await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name, "mystery.jar");
        assert_eq!(found[0].current_version_id, "old1");
        assert_eq!(found[0].latest.version_id, "new2");

        // 身份已落卷宗，下次不必再反查。
        let ledger = aurora.ledger_store("1.20.1-fabric").load().await.unwrap();
        let entry = ledger.find("mystery.jar").expect("反查到的身份应写入卷宗");
        assert_eq!(entry.platform, Platform::Modrinth);
        assert_eq!(entry.project_id, "AANobbMI");
        assert_eq!(entry.version_id, "old1");
        assert_eq!(entry.sha1.as_deref(), Some(sha1.as_str()));
        assert_eq!(entry.installed_at, 1_600_000_000);
        assert_eq!(entry.installed_as_dependency_of, None);
    }

    /// 反查命中：返回补上的条数，字段取自平台，安装时刻取文件 mtime；再跑一次为 0 且不重复联网。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identify_installed_mods_writes_identity_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        let (jar, sha1) = put_mod(mc, "1.20.1-fabric", "sodium.jar", b"sodium-payload").await;
        set_mtime(&jar, 1_612_345_678);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/version_file/{sha1}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(modrinth_version(
                "IZskiJmZ",
                "AANobbMI",
                "0.5.3",
                "2025-01-01T00:00:00Z",
                "sodium.jar",
                &sha1,
            )))
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        assert_eq!(
            aurora.identify_installed_mods("1.20.1-fabric").await.unwrap(),
            1
        );

        let entry = aurora
            .ledger_store("1.20.1-fabric")
            .load()
            .await
            .unwrap()
            .find("sodium.jar")
            .cloned()
            .expect("应写入一条身份");
        assert_eq!(entry.project_id, "AANobbMI");
        assert_eq!(entry.version_id, "IZskiJmZ");
        assert_eq!(entry.installed_at, 1_612_345_678);

        let before = server.received_requests().await.unwrap().len();
        // 已有身份不再反查：条数为 0，且没有新增任何请求。
        assert_eq!(
            aurora.identify_installed_mods("1.20.1-fabric").await.unwrap(),
            0
        );
        assert_eq!(server.received_requests().await.unwrap().len(), before);
    }

    /// 两条通道都认不出（本地自制 Mod）：返回 0，且不为此凭空生成卷宗文件。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn identify_installed_mods_skips_unidentifiable_files() {
        let tmp = tempfile::tempdir().unwrap();
        let mc = tmp.path();
        put_version(mc, "1.20.1-fabric", FABRIC_JSON).await;
        put_mod(mc, "1.20.1-fabric", "my-private-build.jar", b"homemade").await;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/version_file"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        // 测试机若配了 CurseForge key，指纹通道也会走到这里；给一份空匹配保持结果确定。
        Mock::given(method("POST"))
            .and(path("/v1/fingerprints"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"data":{"exactMatches":[]}}"#),
            )
            .mount(&server)
            .await;

        let aurora = aurora_at(mc, &server.uri());
        assert_eq!(
            aurora.identify_installed_mods("1.20.1-fabric").await.unwrap(),
            0
        );
        let ledger_path = aurora.ledger_store("1.20.1-fabric").path().to_path_buf();
        assert!(!tokio::fs::try_exists(&ledger_path).await.unwrap());
    }

    /// 未安装的版本：在触网前就冒泡 VersionNotInstalled。
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn check_updates_on_uninstalled_version_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path(), "http://127.0.0.1:1");

        let err = aurora.check_updates("ghost").await.unwrap_err();
        assert!(matches!(err, CoreError::VersionNotInstalled { id } if id == "ghost"));
    }

    #[test]
    fn ledger_key_strips_disabled_suffix_only_at_the_end() {
        assert_eq!(ledger_key("sodium.jar"), "sodium.jar");
        assert_eq!(ledger_key("sodium.jar.disabled"), "sodium.jar");
        // 中间出现的同名片段不是后缀，不能被剥掉。
        assert_eq!(ledger_key("a.disabled.jar"), "a.disabled.jar");
        assert_eq!(ledger_key(".disabled"), "");
    }

    #[test]
    fn fingerprint_index_joins_by_fingerprint_and_drops_out_of_range() {
        let matches: Vec<CurseForgeFingerprintMatch> = serde_json::from_str(
            r#"[
                {"id":238222,"file":{"id":4567,"modId":238222,"displayName":"JEI 15.2",
                    "fileName":"jei-15.2.jar","fileFingerprint":3608199863}},
                {"id":306612,"file":{"id":8901,"modId":306612,"displayName":"Fabric API",
                    "fileName":"fabric-api.jar","fileFingerprint":5000000000}}
            ]"#,
        )
        .unwrap();

        let index = index_by_fingerprint(matches);
        // 按指纹而非返回顺序 join。
        assert_eq!(index.len(), 1);
        assert_eq!(index.get(&3_608_199_863).unwrap().file_name, "jei-15.2.jar");
        assert_eq!(index.get(&3_608_199_863).unwrap().mod_id, 238_222);
        // 超出 u32 的指纹不可能来自本次请求：丢弃，绝不截断成 705_032_704 去错配。
        assert!(!index.contains_key(&705_032_704));
    }
}
