//! 双源聚合搜索。
//!
//! 并行向 Modrinth 与 CurseForge 发起同一份查询，把两边结果归一到 [`SearchHit`] 后按 slug 去重
//! （同 slug 优先保留 Modrinth），再按下载量降序合并。单个平台失败不影响另一平台结果展示——失败被
//! 记录到 [`AggregateResult::errors`]，命中仍照常返回。CurseForge 未配置（`None`）时只走 Modrinth。

use std::collections::HashSet;

use crate::curseforge::CurseForgeClient;
use crate::error::Error;
use crate::model::{Platform, SearchHit, SearchQuery};
use crate::modrinth::ModrinthClient;

/// 某个平台在聚合搜索中的失败记录。
#[derive(Debug)]
pub struct PlatformError {
    /// 出错的平台。
    pub platform: Platform,
    /// 具体错误。
    pub error: Error,
}

/// 聚合搜索结果。
#[derive(Debug)]
pub struct AggregateResult {
    /// 去重合并并排序后的命中。
    pub hits: Vec<SearchHit>,
    /// 各平台的失败记录（为空表示两边都成功）。
    pub errors: Vec<PlatformError>,
}

impl AggregateResult {
    /// 是否两个平台都成功（无失败记录）。
    pub fn is_complete(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 并行聚合搜索。`curseforge` 为 `None` 表示该源未配置，仅用 Modrinth。
pub async fn aggregate_search(
    modrinth: &ModrinthClient,
    curseforge: Option<&CurseForgeClient>,
    query: &SearchQuery,
) -> AggregateResult {
    let modrinth_fut = async {
        modrinth
            .search(query)
            .await
            .map(|resp| resp.hits.iter().map(SearchHit::from).collect::<Vec<_>>())
    };
    let curseforge_fut = async {
        match curseforge {
            Some(client) => client
                .search(query)
                .await
                .map(|resp| resp.data.iter().map(SearchHit::from).collect::<Vec<_>>()),
            None => Ok(Vec::new()),
        }
    };

    let (modrinth_res, curseforge_res) = tokio::join!(modrinth_fut, curseforge_fut);

    let mut hits: Vec<SearchHit> = Vec::new();
    let mut errors: Vec<PlatformError> = Vec::new();
    let mut modrinth_slugs: HashSet<String> = HashSet::new();

    match modrinth_res {
        Ok(modrinth_hits) => {
            for hit in &modrinth_hits {
                if let Some(slug) = &hit.slug {
                    modrinth_slugs.insert(slug.to_lowercase());
                }
            }
            hits.extend(modrinth_hits);
        }
        Err(error) => errors.push(PlatformError {
            platform: Platform::Modrinth,
            error,
        }),
    }

    match curseforge_res {
        Ok(curseforge_hits) => {
            for hit in curseforge_hits {
                let is_duplicate = hit
                    .slug
                    .as_ref()
                    .is_some_and(|slug| modrinth_slugs.contains(&slug.to_lowercase()));
                if !is_duplicate {
                    hits.push(hit);
                }
            }
        }
        Err(error) => errors.push(PlatformError {
            platform: Platform::CurseForge,
            error,
        }),
    }

    sort_hits(&mut hits);
    AggregateResult { hits, errors }
}

/// 合并排序：下载量降序为主键；同下载量时 Modrinth 优先，再按标题字典序，保证结果稳定可预期。
fn sort_hits(hits: &mut [SearchHit]) {
    hits.sort_by(|a, b| {
        b.downloads
            .cmp(&a.downloads)
            .then_with(|| platform_rank(a.platform).cmp(&platform_rank(b.platform)))
            .then_with(|| a.title.cmp(&b.title))
    });
}

fn platform_rank(platform: Platform) -> u8 {
    match platform {
        Platform::Modrinth => 0,
        Platform::CurseForge => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_base::retry::RetryPolicy;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn no_retry() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 1,
            ..RetryPolicy::default()
        }
    }

    fn modrinth_body(slug: &str, downloads: u64) -> serde_json::Value {
        serde_json::json!({
            "hits": [{
                "project_id": format!("mr-{slug}"),
                "slug": slug,
                "title": slug,
                "description": "desc",
                "project_type": "mod",
                "downloads": downloads,
                "follows": 0,
                "author": "someone",
                "date_modified": "2026-01-01T00:00:00Z"
            }],
            "offset": 0, "limit": 20, "total_hits": 1
        })
    }

    fn curseforge_body(entries: &[(&str, u32, u64)]) -> serde_json::Value {
        let data: Vec<serde_json::Value> = entries
            .iter()
            .map(|(slug, id, downloads)| {
                serde_json::json!({
                    "id": id,
                    "name": slug,
                    "slug": slug,
                    "summary": "desc",
                    "downloadCount": downloads,
                    "classId": 6,
                    "authors": [{"id": 1, "name": "someone"}]
                })
            })
            .collect();
        serde_json::json!({"data": data, "pagination": {"index":0,"pageSize":20,"resultCount":data.len(),"totalCount":data.len()}})
    }

    async fn mount_modrinth(server: &MockServer, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    async fn mount_curseforge(server: &MockServer, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/v1/mods/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(server)
            .await;
    }

    fn clients(server: &MockServer) -> (ModrinthClient, CurseForgeClient) {
        let http = aurora_base::http::build_client().unwrap();
        let modrinth = ModrinthClient::new(http.clone())
            .with_base_url(server.uri())
            .with_retry(no_retry());
        let curseforge = CurseForgeClient::new(http, "test-key")
            .with_base_url(server.uri())
            .with_retry(no_retry());
        (modrinth, curseforge)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn dedups_by_slug_preferring_modrinth_and_sorts_by_downloads() {
        let server = MockServer::start().await;
        mount_modrinth(&server, modrinth_body("sodium", 100)).await;
        // CurseForge 同 slug 的 sodium 应被丢弃；embeddium 保留。
        mount_curseforge(
            &server,
            curseforge_body(&[("sodium", 111, 999), ("embeddium", 222, 50)]),
        )
        .await;

        let (modrinth, curseforge) = clients(&server);
        let result = aggregate_search(&modrinth, Some(&curseforge), &SearchQuery::new("sodium")).await;

        assert!(result.is_complete());
        assert_eq!(result.hits.len(), 2);
        // sodium(100, Modrinth) 排在 embeddium(50) 前；CurseForge 的 sodium 被去重。
        assert_eq!(result.hits[0].slug.as_deref(), Some("sodium"));
        assert_eq!(result.hits[0].platform, Platform::Modrinth);
        assert_eq!(result.hits[1].slug.as_deref(), Some("embeddium"));
        assert_eq!(result.hits[1].platform, Platform::CurseForge);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn single_platform_failure_still_returns_other() {
        let server = MockServer::start().await;
        // Modrinth 500，CurseForge 正常。聚合应仍返回 CurseForge 结果并记录 Modrinth 失败。
        Mock::given(method("GET"))
            .and(path("/search"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        mount_curseforge(&server, curseforge_body(&[("create", 328085, 42)])).await;

        let (modrinth, curseforge) = clients(&server);
        let result = aggregate_search(&modrinth, Some(&curseforge), &SearchQuery::new("create")).await;

        assert!(!result.is_complete());
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].platform, Platform::Modrinth);
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].slug.as_deref(), Some("create"));
        assert_eq!(result.hits[0].platform, Platform::CurseForge);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn none_curseforge_uses_only_modrinth() {
        let server = MockServer::start().await;
        mount_modrinth(&server, modrinth_body("lithium", 77)).await;

        let (modrinth, _curseforge) = clients(&server);
        let result = aggregate_search(&modrinth, None, &SearchQuery::new("lithium")).await;

        assert!(result.is_complete());
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].platform, Platform::Modrinth);
    }
}
