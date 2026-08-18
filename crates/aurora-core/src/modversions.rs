//! 工程可用版本列表（跨平台统一视图）。
//!
//! Modrinth 的 `GET /project/{id}/version` 与 CurseForge 的 `GET /mods/{id}/files` 返回两套形状完全
//! 不同的 JSON，归一动作已经在 [`aurora_modplatform::version`] 里做完。本模块只负责门面这一层的编排：
//! 按平台选客户端、拿列表、按调用方给的 MC 版本与加载器过滤，最后统一成
//! [`ModVersionInfo`] 交给兼容判定、依赖解析与更新检查复用——上层不该再见到任何平台分支。
//!
//! 排序一律按发布时间倒序：两个平台都不保证返回顺序，而版本下拉、依赖择优、更新检查想要的都是
//! 「最新的那个」，排序规则若各处自定必然分叉。

use aurora_modplatform::{
    CurseForgeClient, CurseForgeFile, ModLoader, ModVersionInfo, ModrinthClient, ModrinthVersion,
    Platform,
};

use crate::error::{CoreError, Result};
use crate::facade::Aurora;

impl Aurora {
    /// 列出某工程的全部可用版本（跨平台统一模型），按发布时间倒序。
    ///
    /// `game_versions` / `loaders` 传空表示该维度不过滤。同一维度内多个取值之间是「或」，两个维度
    /// 之间是「与」。能下推给平台的过滤条件一律下推（Modrinth 支持多值数组，CurseForge 单次只能
    /// 表达一个值），但下推只当作优化：两个平台的过滤语义并不一致，最终口径由本地那一遍统一。
    ///
    /// 平台没给元数据的版本一律放行，与 [`crate::compat::classify`] 判 `Unknown` 是同一条口径。
    /// CurseForge 早期文件常常既没标加载器也没标 MC 版本，若把「不知道」当「不匹配」筛掉，落位层
    /// 判成「可能可行」的实例会在版本列表里一个版本都挑不出来，玩家看到的就是自相矛盾的两块 UI。
    ///
    /// 工程不存在时平台返回 404，经 [`CoreError::ModPlatform`] 原样冒泡而不吞成空列表——
    /// 「这个工程一个版本都没有」与「这个工程根本不存在」对 UI 是两件事。
    pub async fn list_mod_versions(
        &self,
        platform: Platform,
        project_id: &str,
        game_versions: &[String],
        loaders: &[ModLoader],
    ) -> Result<Vec<ModVersionInfo>> {
        let fetched = match platform {
            Platform::Modrinth => {
                let client = ModrinthClient::new(self.http()).with_base_url(self.modrinth_base());
                fetch_modrinth(&client, project_id, game_versions, loaders).await?
            }
            Platform::CurseForge => {
                let client =
                    CurseForgeClient::from_env(self.http())?.with_base_url(self.curseforge_base());
                fetch_curseforge(&client, project_id, game_versions, loaders).await?
            }
        };
        Ok(refine(fetched, game_versions, loaders))
    }
}

/// 拉取 Modrinth 工程版本并归一。
///
/// 过滤条件直接下推给 `/project/{id}/version` 的 `loaders` / `game_versions` 查询参数：Modrinth 两个
/// 参数都收 JSON 数组，语义与本地过滤一致，能少传一大截响应体。
///
/// `files` 为空的历史版本装不了，[`ModrinthVersion::to_version_info`] 会返回 `None`，这里显式跳过——
/// 塞一个空文件名进去会一路污染「卷宗与磁盘按文件名 join」这条地基。
async fn fetch_modrinth(
    client: &ModrinthClient,
    project_id: &str,
    game_versions: &[String],
    loaders: &[ModLoader],
) -> Result<Vec<ModVersionInfo>> {
    let wanted: Vec<&str> = game_versions.iter().map(String::as_str).collect();
    Ok(client
        .versions(project_id, loaders, &wanted)
        .await?
        .iter()
        .filter_map(ModrinthVersion::to_version_info)
        .collect())
}

/// 拉取 CurseForge 工程文件并归一。
///
/// `/mods/{id}/files` 的 `gameVersion` 与 `modLoaderType` 都只收单值，因此只有该维度恰好请求一个取值
/// 时才下推。这不只是省流量：该端点是分页的（客户端只取首页），不下推游戏版本时，一个有上百个文件
/// 的大 Mod 首页可能压根不含目标 MC 版本的文件，列表会「明明有版本却是空的」。
async fn fetch_curseforge(
    client: &CurseForgeClient,
    project_id: &str,
    game_versions: &[String],
    loaders: &[ModLoader],
) -> Result<Vec<ModVersionInfo>> {
    // CurseForge 的工程 id 是十进制数字，非数字串不可能对应任何工程，在触网前就短路。
    let mod_id: u32 = project_id
        .parse()
        .map_err(|_| CoreError::ModVersionNotFound {
            platform: Platform::CurseForge.display_name(),
            project_id: project_id.to_owned(),
            // 错误枚举目前只有版本级的「未找到」，通配 * 表示「该工程的任何版本都取不到」。
            version_id: "*".to_owned(),
        })?;
    let single_game_version = match game_versions {
        [only] => Some(only.as_str()),
        _ => None,
    };
    let single_loader = match loaders {
        [only] => Some(*only),
        _ => None,
    };
    Ok(client
        .mod_files(mod_id, single_loader, single_game_version)
        .await?
        .iter()
        .map(CurseForgeFile::to_version_info)
        .collect())
}

/// 过滤 + 排序：两个平台拉回来的列表都过这一道，保证对外口径唯一。
fn refine(
    mut versions: Vec<ModVersionInfo>,
    game_versions: &[String],
    loaders: &[ModLoader],
) -> Vec<ModVersionInfo> {
    versions.retain(|version| matches_filters(version, game_versions, loaders));
    sort_by_published_desc(&mut versions);
    versions
}

/// 某版本是否通过调用方给的过滤条件。
///
/// 三条规则：该维度没提要求就放行；平台没给该维度的元数据也放行（缺失不是证据）；给了就必须有交集。
/// 第二条与 [`crate::compat::classify`] 把单维缺失判成 `Unknown` 完全对齐，两处若各行其是，
/// 落位层的判定和版本下拉就会互相打脸。
fn matches_filters(
    version: &ModVersionInfo,
    game_versions: &[String],
    loaders: &[ModLoader],
) -> bool {
    let game_ok = game_versions.is_empty()
        || version.game_versions.is_empty()
        || version
            .game_versions
            .iter()
            .any(|declared| game_versions.contains(declared));
    let loader_ok = loaders.is_empty()
        || version.loaders.is_empty()
        || version
            .loaders
            .iter()
            .any(|declared| loaders.contains(declared));
    game_ok && loader_ok
}

/// 按发布时间倒序排序，日期缺失的沉底。
///
/// 两个平台的 `date_published` 都是 UTC 的 ISO 8601 定宽串，字典序即时间序，不必解析成时间戳再比
/// （启动器没有必要为排序引入一个日期库）。若将来出现带时区偏移的取值，这里必须改成真正的时间解析。
///
/// 日期为空串的版本排在最后：平台没给发布时间时无从判断新旧，把它顶到「最新」位置会让默认选中落到
/// 一个来历不明的版本上。
pub fn sort_by_published_desc(versions: &mut [ModVersionInfo]) {
    versions.sort_by(|a, b| {
        // 第一维升序（有日期 false < 无日期 true，即有日期的在前），第二维降序（新的在前）。
        a.date_published
            .is_empty()
            .cmp(&b.date_published.is_empty())
            .then_with(|| b.date_published.cmp(&a.date_published))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuroraConfig;
    use aurora_modplatform::{
        DependencyKind, Error as PlatformError, ModDependency, ReleaseChannel,
    };
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn version(id: &str, date: &str) -> ModVersionInfo {
        ModVersionInfo {
            version_id: id.to_owned(),
            project_id: "AANobbMI".to_owned(),
            platform: Platform::Modrinth,
            name: format!("Sodium {id}"),
            version_number: id.to_owned(),
            release_channel: ReleaseChannel::Release,
            game_versions: vec!["1.20.1".to_owned()],
            loaders: Vec::new(),
            file_name: format!("{id}.jar"),
            file_size: None,
            sha1: None,
            date_published: date.to_owned(),
            dependencies: Vec::new(),
        }
    }

    /// 造一个可精确控制两维元数据的版本，用于过滤边界。
    fn tagged(
        id: &str,
        date: &str,
        game_versions: &[&str],
        loaders: &[ModLoader],
    ) -> ModVersionInfo {
        ModVersionInfo {
            game_versions: game_versions.iter().map(|v| (*v).to_owned()).collect(),
            loaders: loaders.to_vec(),
            ..version(id, date)
        }
    }

    fn ids(versions: &[ModVersionInfo]) -> Vec<&str> {
        versions.iter().map(|v| v.version_id.as_str()).collect()
    }

    fn test_aurora(base: &str) -> Aurora {
        let tmp = std::env::temp_dir().join("aurora-modversions-tests");
        Aurora::for_test(AuroraConfig::default(), tmp.clone(), tmp)
            .with_modrinth_base(base)
            .with_curseforge_base(base)
    }

    /// 直接建一个带 key 的 CurseForge 客户端：`from_env` 依赖进程级环境变量，测试里改全局环境
    /// （edition 2024 下还是 unsafe）会污染同进程的其它测试，故只覆盖到客户端之后的逻辑。
    fn curseforge_client(base: &str) -> CurseForgeClient {
        let http = aurora_base::http::build_client().expect("构建测试 HTTP 客户端");
        CurseForgeClient::new(http, "test-key").with_base_url(base)
    }

    // ---- 排序 ----

    #[test]
    fn newest_first() {
        let mut list = vec![
            version("old", "2025-03-01T00:00:00Z"),
            version("newest", "2026-07-30T12:00:00Z"),
            version("mid", "2026-01-02T03:04:05Z"),
        ];
        sort_by_published_desc(&mut list);
        assert_eq!(ids(&list), vec!["newest", "mid", "old"]);
    }

    #[test]
    fn same_day_orders_by_time_not_just_date() {
        let mut list = vec![
            version("morning", "2026-05-05T08:00:00Z"),
            version("evening", "2026-05-05T21:30:00Z"),
        ];
        sort_by_published_desc(&mut list);
        assert_eq!(ids(&list), vec!["evening", "morning"]);
    }

    #[test]
    fn undated_versions_sink_below_dated_ones() {
        let mut list = vec![
            version("nodate-a", ""),
            version("oldest", "2019-01-01T00:00:00Z"),
            version("nodate-b", ""),
            version("newer", "2026-01-01T00:00:00Z"),
        ];
        sort_by_published_desc(&mut list);
        // 即便日期最老，有日期的也排在无日期的前面。
        assert_eq!(ids(&list), vec!["newer", "oldest", "nodate-a", "nodate-b"]);
    }

    #[test]
    fn empty_and_single_are_untouched() {
        let mut empty: Vec<ModVersionInfo> = Vec::new();
        sort_by_published_desc(&mut empty);
        assert!(empty.is_empty());

        let mut one = vec![version("only", "2026-01-01T00:00:00Z")];
        sort_by_published_desc(&mut one);
        assert_eq!(ids(&one), vec!["only"]);
    }

    // ---- 本地过滤 ----

    #[test]
    fn filter_drops_declared_mismatches_and_keeps_metadata_less_versions() {
        let list = vec![
            tagged(
                "hit",
                "2026-01-05T00:00:00Z",
                &["1.20.1"],
                &[ModLoader::Fabric],
            ),
            // 加载器对不上：确凿不兼容，筛掉。
            tagged(
                "wrong-loader",
                "2026-01-04T00:00:00Z",
                &["1.20.1"],
                &[ModLoader::Forge],
            ),
            // MC 版本对不上：确凿不兼容，筛掉。
            tagged(
                "wrong-mc",
                "2026-01-03T00:00:00Z",
                &["1.19.4"],
                &[ModLoader::Fabric],
            ),
            // 两维都没标：平台没给元数据不等于不兼容，放行。
            tagged("bare", "2026-01-02T00:00:00Z", &[], &[]),
            // 只标了加载器且对得上，MC 版本缺失：单维缺失同样放行。
            tagged(
                "loader-only",
                "2026-01-01T00:00:00Z",
                &[],
                &[ModLoader::Fabric],
            ),
        ];
        let kept = refine(list, &["1.20.1".to_owned()], &[ModLoader::Fabric]);
        assert_eq!(ids(&kept), vec!["hit", "bare", "loader-only"]);
    }

    #[test]
    fn filter_treats_multiple_values_inside_one_dimension_as_or() {
        let list = vec![
            tagged(
                "mc-1201",
                "2026-01-03T00:00:00Z",
                &["1.20.1"],
                &[ModLoader::Fabric],
            ),
            tagged(
                "mc-1211",
                "2026-01-02T00:00:00Z",
                &["1.21.1"],
                &[ModLoader::Quilt],
            ),
            tagged(
                "mc-1194",
                "2026-01-01T00:00:00Z",
                &["1.19.4"],
                &[ModLoader::Fabric],
            ),
        ];
        let kept = refine(
            list,
            &["1.20.1".to_owned(), "1.21.1".to_owned()],
            &[ModLoader::Fabric, ModLoader::Quilt],
        );
        assert_eq!(ids(&kept), vec!["mc-1201", "mc-1211"]);
    }

    #[test]
    fn filter_requires_both_dimensions_to_pass() {
        // MC 版本命中但加载器不命中，仍然筛掉：两个维度之间是「与」。
        let list = vec![tagged(
            "half",
            "2026-01-01T00:00:00Z",
            &["1.20.1"],
            &[ModLoader::Forge],
        )];
        let kept = refine(list, &["1.20.1".to_owned()], &[ModLoader::Fabric]);
        assert!(kept.is_empty());
    }

    #[test]
    fn empty_filters_keep_everything() {
        let list = vec![
            tagged(
                "a",
                "2026-01-01T00:00:00Z",
                &["1.7.10"],
                &[ModLoader::LiteLoader],
            ),
            tagged(
                "b",
                "2026-02-01T00:00:00Z",
                &["1.21"],
                &[ModLoader::NeoForge],
            ),
        ];
        let kept = refine(list, &[], &[]);
        assert_eq!(ids(&kept), vec!["b", "a"]);
    }

    // ---- Modrinth ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn modrinth_versions_are_listed_newest_first_with_fields_mapped() {
        let server = MockServer::start().await;
        // 顺序刻意打乱，且最新的那条没有文件——必须被跳过，否则它会排在第一位。
        let body = r#"[
            {"id":"v-old","project_id":"AANobbMI","name":"Sodium 0.4.10",
             "version_number":"mc1.19.4-0.4.10","version_type":"release",
             "date_published":"2025-03-01T00:00:00Z","game_versions":["1.19.4"],
             "loaders":["fabric"],
             "files":[{"hashes":{"sha1":"aaa111"},"url":"https://cdn/sodium-old.jar",
                       "filename":"sodium-old.jar","primary":true,"size":100}]},
            {"id":"v-nofile","project_id":"AANobbMI","name":"Sodium 撤包版",
             "version_number":"mc1.20.1-0.5.9","version_type":"release",
             "date_published":"2026-08-01T00:00:00Z","game_versions":["1.20.1"],
             "loaders":["fabric"],"files":[]},
            {"id":"v-new","project_id":"AANobbMI","name":"Sodium 0.5.3",
             "version_number":"mc1.20.1-0.5.3","version_type":"beta",
             "date_published":"2026-07-30T12:00:00Z","game_versions":["1.20.1","1.20.4"],
             "loaders":["fabric","quilt","rift"],
             "dependencies":[{"project_id":"P7dR8mSH","version_id":null,
                              "dependency_type":"required"}],
             "files":[{"hashes":{"sha1":"bbb222"},"url":"https://cdn/sodium-new.jar",
                       "filename":"sodium-new.jar","primary":true,"size":204800}]}
        ]"#;
        Mock::given(method("GET"))
            .and(path("/project/sodium/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;

        let aurora = test_aurora(&server.uri());
        let listed = aurora
            .list_mod_versions(Platform::Modrinth, "sodium", &[], &[])
            .await
            .unwrap();

        assert_eq!(ids(&listed), vec!["v-new", "v-old"]);
        let newest = &listed[0];
        assert_eq!(newest.project_id, "AANobbMI");
        assert_eq!(newest.platform, Platform::Modrinth);
        assert_eq!(newest.name, "Sodium 0.5.3");
        assert_eq!(newest.version_number, "mc1.20.1-0.5.3");
        assert_eq!(newest.release_channel, ReleaseChannel::Beta);
        assert_eq!(newest.game_versions, vec!["1.20.1", "1.20.4"]);
        // "rift" 认不出，丢弃而不是报错。
        assert_eq!(newest.loaders, vec![ModLoader::Fabric, ModLoader::Quilt]);
        assert_eq!(newest.file_name, "sodium-new.jar");
        assert_eq!(newest.file_size, Some(204_800));
        assert_eq!(newest.sha1.as_deref(), Some("bbb222"));
        assert_eq!(newest.date_published, "2026-07-30T12:00:00Z");
        assert_eq!(
            newest.dependencies,
            vec![ModDependency {
                project_id: Some("P7dR8mSH".to_owned()),
                version_id: None,
                kind: DependencyKind::Required,
            }]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn modrinth_filters_are_pushed_into_the_platform_query() {
        let server = MockServer::start().await;
        // 只有带上这两个查询参数的请求才有响应；参数没下推就撞不到 mock，wiremock 回 404 变成错误。
        Mock::given(method("GET"))
            .and(path("/project/sodium/version"))
            .and(query_param("loaders", r#"["fabric"]"#))
            .and(query_param("game_versions", r#"["1.20.1"]"#))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"[{"id":"v-filtered","project_id":"AANobbMI","name":"Sodium",
                     "version_number":"0.5.3","version_type":"release",
                     "date_published":"2026-07-30T12:00:00Z","game_versions":["1.20.1"],
                     "loaders":["fabric"],
                     "files":[{"hashes":{"sha1":"ccc333"},"url":"https://cdn/s.jar",
                               "filename":"s.jar","primary":true,"size":10}]}]"#,
            ))
            .mount(&server)
            .await;

        let aurora = test_aurora(&server.uri());
        let listed = aurora
            .list_mod_versions(
                Platform::Modrinth,
                "sodium",
                &["1.20.1".to_owned()],
                &[ModLoader::Fabric],
            )
            .await
            .unwrap();
        assert_eq!(ids(&listed), vec!["v-filtered"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn modrinth_missing_project_bubbles_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/project/ghost/version"))
            .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"error":"not_found"}"#))
            .mount(&server)
            .await;

        let aurora = test_aurora(&server.uri());
        let err = aurora
            .list_mod_versions(Platform::Modrinth, "ghost", &[], &[])
            .await
            .unwrap_err();
        match err {
            CoreError::ModPlatform(PlatformError::Status { status, .. }) => {
                assert_eq!(status, 404);
            }
            other => panic!("期望 404 状态错误冒泡，得到 {other:?}"),
        }
    }

    // ---- CurseForge ----

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn curseforge_files_are_normalized_and_listed_newest_first() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [
                {
                    "id": 3901,
                    "modId": 238222,
                    "displayName": "JEI 15.2.0.27",
                    "fileName": "jei-1.20.1-forge-15.2.0.27.jar",
                    "releaseType": 1,
                    "hashes": [
                        {"value": "0123456789abcdef", "algo": 1},
                        {"value": "ffee", "algo": 2}
                    ],
                    "fileDate": "2026-03-04T05:06:07Z",
                    "fileLength": 1_234_567u64,
                    "downloadUrl": "https://cf/jei.jar",
                    "gameVersions": ["1.20.1", "Forge", "Client", "Java 17"],
                    "dependencies": [{"modId": 306612, "relationType": 3}]
                },
                {
                    "id": 3800,
                    "modId": 238222,
                    "displayName": "JEI 15.1.0.0",
                    "fileName": "jei-1.20.1-forge-15.1.0.0.jar",
                    "releaseType": 2,
                    "hashes": [],
                    "fileDate": "2025-11-01T00:00:00Z",
                    "fileLength": 1_000_000u64,
                    "downloadUrl": "https://cf/jei-old.jar",
                    "gameVersions": ["1.20.1", "Forge"],
                    "dependencies": []
                }
            ],
            "pagination": {"index": 0, "pageSize": 50, "resultCount": 2, "totalCount": 2}
        });
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let client = curseforge_client(&server.uri());
        let fetched = fetch_curseforge(&client, "238222", &[], &[]).await.unwrap();
        let listed = refine(fetched, &[], &[]);

        assert_eq!(ids(&listed), vec!["3901", "3800"]);
        let newest = &listed[0];
        assert_eq!(newest.project_id, "238222");
        assert_eq!(newest.platform, Platform::CurseForge);
        assert_eq!(newest.name, "JEI 15.2.0.27");
        assert_eq!(newest.version_number, "jei-1.20.1-forge-15.2.0.27.jar");
        assert_eq!(newest.release_channel, ReleaseChannel::Release);
        // gameVersions 里混着加载器名与杂物：数字开头的进 game_versions，加载器名进 loaders，其余丢弃。
        assert_eq!(newest.game_versions, vec!["1.20.1"]);
        assert_eq!(newest.loaders, vec![ModLoader::Forge]);
        assert_eq!(newest.file_name, "jei-1.20.1-forge-15.2.0.27.jar");
        assert_eq!(newest.file_size, Some(1_234_567));
        // 只取 algo=1 的 SHA-1，不能误取 MD5。
        assert_eq!(newest.sha1.as_deref(), Some("0123456789abcdef"));
        assert_eq!(newest.date_published, "2026-03-04T05:06:07Z");
        assert_eq!(
            newest.dependencies,
            vec![ModDependency {
                project_id: Some("306612".to_owned()),
                version_id: None,
                kind: DependencyKind::Required,
            }]
        );
        assert_eq!(listed[1].release_channel, ReleaseChannel::Beta);
        assert_eq!(listed[1].sha1, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn curseforge_single_value_filters_are_pushed_into_the_platform_query() {
        let server = MockServer::start().await;
        // modLoaderType=4 是 Fabric 的 CurseForge 编码；两个参数都必须出现在请求里。
        Mock::given(method("GET"))
            .and(path("/v1/mods/394468/files"))
            .and(query_param("gameVersion", "1.20.1"))
            .and(query_param("modLoaderType", "4"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{
                    "id": 5001,
                    "modId": 394468,
                    "displayName": "Sodium",
                    "fileName": "sodium-fabric-0.5.3.jar",
                    "releaseType": 1,
                    "hashes": [],
                    "fileDate": "2026-02-02T00:00:00Z",
                    "fileLength": 500u64,
                    "gameVersions": ["1.20.1", "Fabric"],
                    "dependencies": []
                }],
                "pagination": {"index": 0, "pageSize": 50, "resultCount": 1, "totalCount": 1}
            })))
            .mount(&server)
            .await;

        let client = curseforge_client(&server.uri());
        let fetched = fetch_curseforge(
            &client,
            "394468",
            &["1.20.1".to_owned()],
            &[ModLoader::Fabric],
        )
        .await
        .unwrap();
        assert_eq!(ids(&fetched), vec!["5001"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn curseforge_multi_value_filters_stay_local() {
        let server = MockServer::start().await;
        // 多值无法下推：请求里必须一个过滤参数都没有（下推了单值就是错的），服务端把三个 MC
        // 版本的文件全给回来，筛选责任落到本地那一遍。
        Mock::given(method("GET"))
            .and(path("/v1/mods/238222/files"))
            .and(query_param_is_missing("gameVersion"))
            .and(query_param_is_missing("modLoaderType"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": 1, "modId": 238222, "displayName": "a", "fileName": "a.jar",
                     "releaseType": 1, "hashes": [], "fileDate": "2026-01-03T00:00:00Z",
                     "fileLength": 1u64, "gameVersions": ["1.20.1", "Forge"], "dependencies": []},
                    {"id": 2, "modId": 238222, "displayName": "b", "fileName": "b.jar",
                     "releaseType": 1, "hashes": [], "fileDate": "2026-01-02T00:00:00Z",
                     "fileLength": 1u64, "gameVersions": ["1.21.1", "Forge"], "dependencies": []},
                    {"id": 3, "modId": 238222, "displayName": "c", "fileName": "c.jar",
                     "releaseType": 1, "hashes": [], "fileDate": "2026-01-01T00:00:00Z",
                     "fileLength": 1u64, "gameVersions": ["1.16.5", "Forge"], "dependencies": []}
                ],
                "pagination": {"index": 0, "pageSize": 50, "resultCount": 3, "totalCount": 3}
            })))
            .mount(&server)
            .await;

        let client = curseforge_client(&server.uri());
        let wanted = vec!["1.20.1".to_owned(), "1.21.1".to_owned()];
        let fetched = fetch_curseforge(&client, "238222", &wanted, &[])
            .await
            .unwrap();
        assert_eq!(ids(&fetched), vec!["1", "2", "3"]);
        // 本地这一遍把 1.16.5 筛掉，并按发布时间倒序。
        let listed = refine(fetched, &wanted, &[]);
        assert_eq!(ids(&listed), vec!["1", "2"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn curseforge_missing_project_bubbles_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/mods/999999/files"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = curseforge_client(&server.uri());
        let err = fetch_curseforge(&client, "999999", &[], &[])
            .await
            .unwrap_err();
        match err {
            CoreError::ModPlatform(PlatformError::Status { status, .. }) => {
                assert_eq!(status, 404);
            }
            other => panic!("期望 404 状态错误冒泡，得到 {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn curseforge_non_numeric_project_id_errors_before_touching_network() {
        // 基址指向一个必然连不上的端口：只要发生了网络请求，报的就会是传输层错误而不是这条。
        let client = curseforge_client("http://127.0.0.1:1");
        let err = fetch_curseforge(&client, "sodium", &[], &[])
            .await
            .unwrap_err();
        match err {
            CoreError::ModVersionNotFound {
                platform,
                project_id,
                version_id,
            } => {
                assert_eq!(platform, "CurseForge");
                assert_eq!(project_id, "sodium");
                assert_eq!(version_id, "*");
            }
            other => panic!("期望 ModVersionNotFound，得到 {other:?}"),
        }
    }
}
