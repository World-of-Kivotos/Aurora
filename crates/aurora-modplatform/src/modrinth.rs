//! Modrinth v2 客户端（api.modrinth.com/v2）。
//!
//! 覆盖搜索（facets 过滤：类型/加载器/游戏版本）、工程详情、版本列表、依赖解析、以及按文件哈希查
//! 版本 / 查更新。所有远端地址经 `base_url` 注入，单测走本地 mock。响应体裸模型（`Modrinth*`）保持
//! 与 API 字段一致，向统一模型的翻译放在 [`From`] 实现里。

use std::path::PathBuf;

use aurora_base::retry::RetryPolicy;
use aurora_download::DownloadTask;
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::model::{DependencyKind, Platform, ResourceType, SearchHit, SearchQuery};
use crate::net::send_json;

/// Modrinth v2 API 根地址。
pub const MODRINTH_BASE: &str = "https://api.modrinth.com/v2";

/// Modrinth v2 客户端。
#[derive(Debug, Clone)]
pub struct ModrinthClient {
    http: reqwest::Client,
    base_url: String,
    retry: RetryPolicy,
}

impl ModrinthClient {
    /// 用共享 HTTP 客户端构造，指向官方地址。
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            base_url: MODRINTH_BASE.to_string(),
            retry: RetryPolicy::default(),
        }
    }

    /// 注入自定义根地址（末尾斜杠会被去除），供 mock 测试与镜像使用。
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into().trim_end_matches('/').to_string();
        self
    }

    /// 覆盖重试策略。
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// 搜索工程。
    pub async fn search(&self, query: &SearchQuery) -> Result<ModrinthSearchResponse> {
        let facets = build_facets(query)?;
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(text) = &query.query {
            params.push(("query", text.clone()));
        }
        params.push(("facets", facets));
        params.push(("index", query.sort.modrinth_index().to_string()));
        params.push(("offset", query.offset.to_string()));
        params.push(("limit", query.limit.to_string()));

        let url = format!("{}/search", self.base_url);
        send_json(&self.retry, "modrinth.search", || {
            self.http.get(url.as_str()).query(&params)
        })
        .await
    }

    /// 取工程详情（`id` 可为 project_id 或 slug）。
    pub async fn project(&self, id_or_slug: &str) -> Result<ModrinthProject> {
        let url = format!("{}/project/{}", self.base_url, id_or_slug);
        send_json(&self.retry, "modrinth.project", || self.http.get(url.as_str())).await
    }

    /// 列出工程的版本，可按加载器与游戏版本过滤。
    pub async fn versions(
        &self,
        id_or_slug: &str,
        loaders: &[crate::model::ModLoader],
        game_versions: &[&str],
    ) -> Result<Vec<ModrinthVersion>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if !loaders.is_empty() {
            let arr: Vec<&str> = loaders.iter().map(|l| l.modrinth_facet()).collect();
            params.push(("loaders", json_array(&arr, "modrinth loaders")?));
        }
        if !game_versions.is_empty() {
            params.push((
                "game_versions",
                json_array(game_versions, "modrinth game_versions")?,
            ));
        }
        let url = format!("{}/project/{}/version", self.base_url, id_or_slug);
        send_json(&self.retry, "modrinth.versions", || {
            self.http.get(url.as_str()).query(&params)
        })
        .await
    }

    /// 按 SHA-1 反查该文件对应的版本；未收录返回 `None`。
    pub async fn version_by_hash(&self, sha1: &str) -> Result<Option<ModrinthVersion>> {
        let url = format!("{}/version_file/{}", self.base_url, sha1);
        let result = send_json(&self.retry, "modrinth.version_file", || {
            self.http.get(url.as_str()).query(&[("algorithm", "sha1")])
        })
        .await;
        match result {
            Ok(version) => Ok(Some(version)),
            Err(Error::Status { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }

    /// 按 SHA-1 查该文件在指定加载器/游戏版本下的最新版本（更新检测）；无匹配返回 `None`。
    pub async fn latest_version_by_hash(
        &self,
        sha1: &str,
        loaders: &[crate::model::ModLoader],
        game_versions: &[&str],
    ) -> Result<Option<ModrinthVersion>> {
        #[derive(serde::Serialize)]
        struct UpdateBody<'a> {
            loaders: Vec<&'a str>,
            game_versions: Vec<&'a str>,
        }
        let body = UpdateBody {
            loaders: loaders.iter().map(|l| l.modrinth_facet()).collect(),
            game_versions: game_versions.to_vec(),
        };
        let url = format!("{}/version_file/{}/update", self.base_url, sha1);
        let result = send_json(&self.retry, "modrinth.version_file.update", || {
            self.http
                .post(url.as_str())
                .query(&[("algorithm", "sha1")])
                .json(&body)
        })
        .await;
        match result {
            Ok(version) => Ok(Some(version)),
            Err(Error::Status { status: 404, .. }) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

/// 构造 Modrinth `facets` 查询串：形如 `[["project_type:mod"],["categories:fabric"],["versions:1.20.1"]]`。
/// 组内为「或」、组间为「与」。加载器归入 `categories` facet（Modrinth 搜索约定）。
fn build_facets(query: &SearchQuery) -> Result<String> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    groups.push(vec![format!(
        "project_type:{}",
        query.resource_type.modrinth_project_type()
    )]);
    if !query.loaders.is_empty() {
        groups.push(
            query
                .loaders
                .iter()
                .map(|l| format!("categories:{}", l.modrinth_facet()))
                .collect(),
        );
    }
    if !query.game_versions.is_empty() {
        groups.push(
            query
                .game_versions
                .iter()
                .map(|v| format!("versions:{v}"))
                .collect(),
        );
    }
    serde_json::to_string(&groups).map_err(|source| Error::Json {
        context: "modrinth facets".to_string(),
        source,
    })
}

/// 把字符串切片序列化为 JSON 数组串（Modrinth 的 `loaders` / `game_versions` 查询参数格式）。
fn json_array(values: &[&str], context: &str) -> Result<String> {
    serde_json::to_string(values).map_err(|source| Error::Json {
        context: context.to_string(),
        source,
    })
}

/// `/search` 响应。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthSearchResponse {
    /// 命中列表。
    pub hits: Vec<ModrinthHit>,
    /// 本页偏移。
    pub offset: u32,
    /// 本页条数上限。
    pub limit: u32,
    /// 总命中数。
    pub total_hits: u32,
}

/// 单条搜索命中。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthHit {
    /// 工程 id。
    pub project_id: String,
    /// 工程 slug。
    #[serde(default)]
    pub slug: Option<String>,
    /// 标题。
    pub title: String,
    /// 简介。
    pub description: String,
    /// 分类/加载器标签。
    #[serde(default)]
    pub categories: Vec<String>,
    /// 工程类型（mod/modpack/...）。
    pub project_type: String,
    /// 下载量。
    pub downloads: u64,
    /// 关注数。
    #[serde(default)]
    pub follows: u64,
    /// 图标 URL。
    #[serde(default)]
    pub icon_url: Option<String>,
    /// 作者名。
    pub author: String,
    /// 更新时间（ISO-8601）。
    pub date_modified: String,
}

impl From<&ModrinthHit> for SearchHit {
    fn from(hit: &ModrinthHit) -> Self {
        let resource_type = ResourceType::from_modrinth_project_type(&hit.project_type);
        let page_url = hit
            .slug
            .as_ref()
            .map(|slug| format!("https://modrinth.com/{}/{}", hit.project_type, slug));
        SearchHit {
            platform: Platform::Modrinth,
            project_id: hit.project_id.clone(),
            slug: hit.slug.clone(),
            title: hit.title.clone(),
            description: hit.description.clone(),
            author: Some(hit.author.clone()),
            downloads: hit.downloads,
            follows: Some(hit.follows),
            icon_url: hit.icon_url.clone(),
            categories: hit.categories.clone(),
            resource_type,
            date_modified: Some(hit.date_modified.clone()),
            page_url,
        }
    }
}

/// 工程详情。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthProject {
    /// 工程 id。
    pub id: String,
    /// slug。
    #[serde(default)]
    pub slug: Option<String>,
    /// 标题。
    pub title: String,
    /// 简介。
    pub description: String,
    /// 分类/加载器标签。
    #[serde(default)]
    pub categories: Vec<String>,
    /// 工程类型。
    pub project_type: String,
    /// 下载量。
    pub downloads: u64,
    /// 关注数。
    #[serde(default)]
    pub followers: u64,
    /// 图标 URL。
    #[serde(default)]
    pub icon_url: Option<String>,
    /// 版本 id 列表。
    #[serde(default)]
    pub versions: Vec<String>,
    /// 支持的游戏版本。
    #[serde(default)]
    pub game_versions: Vec<String>,
    /// 支持的加载器。
    #[serde(default)]
    pub loaders: Vec<String>,
    /// 更新时间。
    #[serde(default)]
    pub updated: Option<String>,
    /// 发布时间。
    #[serde(default)]
    pub published: Option<String>,
}

/// 版本。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthVersion {
    /// 版本 id。
    pub id: String,
    /// 所属工程 id。
    pub project_id: String,
    /// 版本显示名。
    pub name: String,
    /// 版本号串。
    pub version_number: String,
    /// 依赖。
    #[serde(default)]
    pub dependencies: Vec<ModrinthDependency>,
    /// 支持的游戏版本。
    #[serde(default)]
    pub game_versions: Vec<String>,
    /// 发布通道（release/beta/alpha）。
    pub version_type: String,
    /// 支持的加载器。
    #[serde(default)]
    pub loaders: Vec<String>,
    /// 是否精选。
    #[serde(default)]
    pub featured: bool,
    /// 发布时间。
    pub date_published: String,
    /// 下载量。
    #[serde(default)]
    pub downloads: u64,
    /// 文件列表。
    pub files: Vec<ModrinthFile>,
}

impl ModrinthVersion {
    /// 主文件：优先标记为 `primary` 者，否则取第一个。
    pub fn primary_file(&self) -> Option<&ModrinthFile> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}

/// 版本依赖。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthDependency {
    /// 指定的版本 id。
    #[serde(default)]
    pub version_id: Option<String>,
    /// 指定的工程 id。
    #[serde(default)]
    pub project_id: Option<String>,
    /// 指定的文件名。
    #[serde(default)]
    pub file_name: Option<String>,
    /// 依赖类型原始串。
    pub dependency_type: String,
}

impl ModrinthDependency {
    /// 依赖关系（统一模型）；未知类型返回 `None`。
    pub fn kind(&self) -> Option<DependencyKind> {
        DependencyKind::from_modrinth(&self.dependency_type)
    }
}

/// 版本文件。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthFile {
    /// 各算法哈希。
    pub hashes: ModrinthHashes,
    /// 下载 URL。
    pub url: String,
    /// 文件名。
    pub filename: String,
    /// 是否主文件。
    #[serde(default)]
    pub primary: bool,
    /// 文件大小（字节）。
    pub size: u64,
}

impl ModrinthFile {
    /// 转成下载引擎可执行的任务：带上 sha1 与大小以便下载后强制校验。
    pub fn to_download_task(&self, dest: impl Into<PathBuf>) -> DownloadTask {
        let mut task = DownloadTask::new(self.url.clone(), dest).with_size(self.size);
        if let Some(sha1) = &self.hashes.sha1 {
            task = task.with_sha1(sha1.clone());
        }
        task
    }
}

/// 文件哈希集合。
#[derive(Debug, Clone, Deserialize)]
pub struct ModrinthHashes {
    /// SHA-1（小写十六进制）。
    #[serde(default)]
    pub sha1: Option<String>,
    /// SHA-512。
    #[serde(default)]
    pub sha512: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ModLoader, SortField};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> ModrinthClient {
        let http = aurora_base::http::build_client().unwrap();
        ModrinthClient::new(http).with_base_url(server.uri())
    }

    #[test]
    fn facets_encodes_type_loader_and_version() {
        let query = SearchQuery::new("sodium")
            .with_loader(ModLoader::Fabric)
            .with_game_version("1.20.1");
        let facets = build_facets(&query).unwrap();
        assert_eq!(
            facets,
            r#"[["project_type:mod"],["categories:fabric"],["versions:1.20.1"]]"#
        );
    }

    #[test]
    fn facets_multi_loader_is_or_group() {
        let query = SearchQuery::default()
            .with_loader(ModLoader::Fabric)
            .with_loader(ModLoader::Quilt);
        let facets = build_facets(&query).unwrap();
        assert_eq!(
            facets,
            r#"[["project_type:mod"],["categories:fabric","categories:quilt"]]"#
        );
    }

    #[test]
    fn facets_without_filters_only_has_project_type() {
        let facets = build_facets(&SearchQuery::default()).unwrap();
        assert_eq!(facets, r#"[["project_type:mod"]]"#);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn search_parses_hits_and_sends_params() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "hits": [{
                "project_id": "AANobbMI",
                "slug": "sodium",
                "title": "Sodium",
                "description": "现代化渲染优化",
                "categories": ["optimization", "fabric"],
                "project_type": "mod",
                "downloads": 12345678,
                "follows": 4321,
                "icon_url": "https://cdn.modrinth.com/sodium.png",
                "author": "jellysquid3",
                "date_modified": "2026-01-02T03:04:05Z"
            }],
            "offset": 0,
            "limit": 20,
            "total_hits": 1
        });
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("index", "downloads"))
            .and(query_param("query", "sodium"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let query = SearchQuery::new("sodium").with_sort(SortField::Downloads);
        let resp = client(&server).search(&query).await.unwrap();
        assert_eq!(resp.total_hits, 1);
        let hit = &resp.hits[0];
        assert_eq!(hit.project_id, "AANobbMI");
        assert_eq!(hit.downloads, 12_345_678);
        assert_eq!(hit.follows, 4321);

        // 统一模型转换。
        let unified: SearchHit = hit.into();
        assert_eq!(unified.platform, Platform::Modrinth);
        assert_eq!(unified.resource_type, ResourceType::Mod);
        assert_eq!(unified.follows, Some(4321));
        assert_eq!(
            unified.page_url.as_deref(),
            Some("https://modrinth.com/mod/sodium")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn version_by_hash_maps_404_to_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/version_file/deadbeef"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let got = client(&server).version_by_hash("deadbeef").await.unwrap();
        assert!(got.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn version_by_hash_parses_version_and_primary_file() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "id": "IZskiJmZ",
            "project_id": "AANobbMI",
            "name": "Sodium 0.5.3",
            "version_number": "mc1.20.1-0.5.3",
            "dependencies": [{
                "project_id": "P7dR8mSH",
                "dependency_type": "required"
            }],
            "game_versions": ["1.20.1"],
            "version_type": "release",
            "loaders": ["fabric"],
            "featured": true,
            "date_published": "2026-01-02T03:04:05Z",
            "downloads": 999,
            "files": [
                { "hashes": {"sha1": "aabbcc"}, "url": "https://cdn/other.jar", "filename": "other.jar", "primary": false, "size": 10 },
                { "hashes": {"sha1": "ddeeff", "sha512": "long"}, "url": "https://cdn/sodium.jar", "filename": "sodium.jar", "primary": true, "size": 204800 }
            ]
        });
        Mock::given(method("GET"))
            .and(path("/version_file/abc123"))
            .and(query_param("algorithm", "sha1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let version = client(&server)
            .version_by_hash("abc123")
            .await
            .unwrap()
            .expect("应命中版本");
        assert_eq!(version.version_number, "mc1.20.1-0.5.3");
        assert_eq!(version.dependencies[0].kind(), Some(DependencyKind::Required));

        // 主文件选取「primary=true」那个，转下载任务时带上 sha1 与大小。
        let primary = version.primary_file().unwrap();
        assert_eq!(primary.filename, "sodium.jar");
        let task = primary.to_download_task("C:/mods/sodium.jar");
        assert_eq!(task.url, "https://cdn/sodium.jar");
        assert_eq!(task.sha1.as_deref(), Some("ddeeff"));
        assert_eq!(task.size, Some(204800));
    }
}
