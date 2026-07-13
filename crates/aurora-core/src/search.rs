//! 资源搜索：Modrinth + CurseForge 双源聚合。
//!
//! Modrinth 无需鉴权直连；CurseForge 需 API key（环境变量 `AURORA_CURSEFORGE_API_KEY`），缺失时该源
//! 被明确禁用（仅用 Modrinth）而非静默出错。聚合、去重、排序逻辑在 aurora-modplatform，门面只负责按
//! 配置构建两个客户端并转调。

use aurora_modplatform::{
    AggregateResult, CurseForgeClient, ModrinthClient, SearchQuery, aggregate_search,
};

use crate::error::Result;
use crate::facade::Aurora;

impl Aurora {
    /// 聚合搜索 Modrinth 与 CurseForge。
    ///
    /// CurseForge 仅在配置了 API key 时参与；无 key 时结果只含 Modrinth，且不视为错误。单平台失败不影响
    /// 另一平台结果（失败记录在 [`AggregateResult::errors`]，调用方可据 [`AggregateResult::is_complete`] 判断）。
    pub async fn search(&self, query: &SearchQuery) -> Result<AggregateResult> {
        let modrinth = ModrinthClient::new(self.http()).with_base_url(self.modrinth_base());
        // 无 key -> None，仅用 Modrinth。
        let curseforge = CurseForgeClient::from_env(self.http())
            .ok()
            .map(|client| client.with_base_url(self.curseforge_base()));
        Ok(aggregate_search(&modrinth, curseforge.as_ref(), query).await)
    }
}

#[cfg(test)]
mod tests {
    use crate::config::AuroraConfig;
    use crate::facade::Aurora;
    use aurora_modplatform::{Platform, SearchQuery};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_returns_modrinth_hits_without_curseforge_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"hits":[{"project_id":"AABBCC","slug":"sodium","title":"Sodium",
                    "description":"性能优化","project_type":"mod","downloads":123456,"follows":10,
                    "author":"jellysquid","date_modified":"2026-01-01T00:00:00Z"}],
                    "offset":0,"limit":20,"total_hits":1}"#,
            ))
            .mount(&server)
            .await;
        // 把 CurseForge 端点也指向 mock：若测试环境恰好设置了 API key，也不会打到生产端点。
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"data":[],"pagination":{"index":0,"pageSize":20,"resultCount":0,"totalCount":0}}"#,
            ))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let aurora = Aurora::for_test(
            AuroraConfig::default(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        )
        .with_modrinth_base(server.uri())
        .with_curseforge_base(server.uri());

        let result = aurora.search(&SearchQuery::new("sodium")).await.unwrap();

        assert!(result.hits.iter().any(|h| h.slug.as_deref() == Some("sodium")));
        let hit = result.hits.iter().find(|h| h.slug.as_deref() == Some("sodium")).unwrap();
        assert_eq!(hit.platform, Platform::Modrinth);
        assert_eq!(hit.downloads, 123456);
        assert_eq!(hit.title, "Sodium");
    }
}
