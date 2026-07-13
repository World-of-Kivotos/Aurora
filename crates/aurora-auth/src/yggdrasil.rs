//! Authlib-Injector（Yggdrasil）客户端与 ALI 元数据预取。
//!
//! - Yggdrasil 认证：`authserver/authenticate`、`refresh`、`validate`、`invalidate`，
//!   请求/响应字段与错误格式遵循 authlib-injector 的 Yggdrasil 服务端技术规范。
//! - ALI 元数据预取：GET 用户填写的服务器地址，按 `X-Authlib-Injector-API-Location` 头解析出
//!   真正的 API 根地址，再 GET 该根地址取元数据 JSON 并 Base64 编码（供 aurora-launch 拼装
//!   `-Dauthlibinjector.yggdrasil.prefetched`）。javaagent 参数本身的拼装归 aurora-launch。

use aurora_base::retry::{RetryPolicy, retry_async};
use base64::Engine as _;
use reqwest::StatusCode;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::account::GameProfile;
use crate::error::{AuthError, Result};

/// authlib-injector 用于指示真正 API 根地址的响应头。
const API_LOCATION_HEADER: &str = "x-authlib-injector-api-location";

/// Yggdrasil 认证客户端。`api_root` 以 `/` 结尾，其下拼接各 `authserver/*` 端点。
pub struct YggdrasilClient {
    client: reqwest::Client,
    api_root: String,
    retry: RetryPolicy,
}

/// `authenticate` 的结果：令牌对 + 可选角色列表 + 当前选中角色。
#[derive(Debug, Clone)]
pub struct AuthenticateResponse {
    pub access_token: String,
    pub client_token: String,
    /// 账号下可选的角色档案（多角色账号会有多个，供 UI 选择）。
    pub available_profiles: Vec<GameProfile>,
    /// 已绑定/选中的角色（无绑定时为 None）。
    pub selected_profile: Option<GameProfile>,
}

/// `refresh` 的结果：轮换后的令牌对 + 选中角色。
#[derive(Debug, Clone)]
pub struct RefreshResponse {
    pub access_token: String,
    pub client_token: String,
    pub selected_profile: Option<GameProfile>,
}

/// ALI 元数据预取结果。
#[derive(Debug, Clone)]
pub struct AliMetadata {
    /// 解析出的真正 API 根地址（以 `/` 结尾）。
    pub api_root: String,
    /// 元数据 JSON 的 Base64 编码（原样字节，供 `-Dauthlibinjector.yggdrasil.prefetched`）。
    pub prefetched: String,
    /// 解析后的元数据。
    pub metadata: YggdrasilMetadata,
}

/// Yggdrasil API 根元数据（GET API 根地址返回）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YggdrasilMetadata {
    /// 服务端自定义元信息（服务器名称、注册地址等），结构因服务器而异，原样保留。
    #[serde(default)]
    pub meta: serde_json::Value,
    /// 皮肤域名白名单。
    #[serde(default)]
    pub skin_domains: Vec<String>,
    /// 用于校验材质签名的 RSA 公钥（PEM）。部分服务器可能不提供。
    #[serde(default)]
    pub signature_publickey: String,
}

impl YggdrasilClient {
    /// 用 API 根地址构造（自动补足结尾 `/`）。
    pub fn new(client: reqwest::Client, api_root: impl AsRef<str>) -> Self {
        Self {
            client,
            api_root: ensure_trailing_slash(api_root.as_ref()),
            retry: RetryPolicy::default(),
        }
    }

    /// 覆盖重试策略（测试可关重试）。
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// 用户名 + 密码认证。多角色账号会在 `available_profiles` 返回全部角色供选择。
    pub async fn authenticate(
        &self,
        username: &str,
        password: &str,
        client_token: Option<&str>,
    ) -> Result<AuthenticateResponse> {
        let payload = AuthenticatePayload {
            agent: Agent {
                name: "Minecraft",
                version: 1,
            },
            username,
            password,
            client_token,
            request_user: false,
        };
        retry_async(&self.retry, || async {
            let (status, text) = self
                .post("authserver/authenticate", &payload, "Yggdrasil 认证")
                .await?;
            if !status.is_success() {
                return Err(yggdrasil_error(&text, "Yggdrasil 认证"));
            }
            let body: AuthenticateBody = parse(&text, "Yggdrasil 认证")?;
            Ok(body.into())
        })
        .await
    }

    /// 刷新令牌（轮换）。传入 `selected_profile` 可在刷新时切换角色（更换角色）。
    pub async fn refresh(
        &self,
        access_token: &str,
        client_token: Option<&str>,
        selected_profile: Option<&GameProfile>,
    ) -> Result<RefreshResponse> {
        let payload = RefreshPayload {
            access_token,
            client_token,
            selected_profile: selected_profile.map(|p| ProfileRef {
                id: &p.id,
                name: &p.name,
            }),
            request_user: false,
        };
        retry_async(&self.retry, || async {
            let (status, text) = self
                .post("authserver/refresh", &payload, "Yggdrasil 刷新")
                .await?;
            if !status.is_success() {
                return Err(yggdrasil_error(&text, "Yggdrasil 刷新"));
            }
            let body: RefreshBody = parse(&text, "Yggdrasil 刷新")?;
            Ok(body.into())
        })
        .await
    }

    /// 校验令牌是否仍然有效。204 视为有效，403 视为失效，其它状态报错。
    pub async fn validate(&self, access_token: &str, client_token: Option<&str>) -> Result<bool> {
        let payload = ValidatePayload {
            access_token,
            client_token,
        };
        retry_async(&self.retry, || async {
            let (status, text) = self
                .post("authserver/validate", &payload, "Yggdrasil 校验")
                .await?;
            if status.is_success() {
                return Ok(true);
            }
            if status == StatusCode::FORBIDDEN {
                return Ok(false);
            }
            Err(yggdrasil_error(&text, "Yggdrasil 校验"))
        })
        .await
    }

    /// 使令牌失效（登出）。规范上无论令牌是否存在都返回 204。
    pub async fn invalidate(&self, access_token: &str, client_token: Option<&str>) -> Result<()> {
        let payload = ValidatePayload {
            access_token,
            client_token,
        };
        retry_async(&self.retry, || async {
            let (status, text) = self
                .post("authserver/invalidate", &payload, "Yggdrasil 登出")
                .await?;
            if status.is_success() {
                return Ok(());
            }
            Err(yggdrasil_error(&text, "Yggdrasil 登出"))
        })
        .await
    }

    async fn post<B: Serialize>(
        &self,
        path: &str,
        body: &B,
        context: &'static str,
    ) -> Result<(StatusCode, String)> {
        let url = format!("{}{}", self.api_root, path);
        let resp = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|source| AuthError::Http { context, source })?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|source| AuthError::Http { context, source })?;
        Ok((status, text))
    }
}

/// 解析用户填写的服务器地址，得到真正的 API 根地址（以 `/` 结尾）。
///
/// GET 该地址，若响应带 `X-Authlib-Injector-API-Location` 头，则以其为准（可为绝对或相对 URL，
/// 相对 URL 以响应 URL 为基准解析）；否则用（重定向后的）响应 URL 本身。
pub async fn resolve_api_root(client: &reqwest::Client, server_url: &str) -> Result<String> {
    let resp = client
        .get(server_url)
        .send()
        .await
        .map_err(|source| AuthError::Http {
            context: "解析 ALI 地址",
            source,
        })?;
    let final_url = resp.url().clone();
    if let Some(value) = resp.headers().get(API_LOCATION_HEADER) {
        let loc = value.to_str().map_err(|_| AuthError::Response {
            context: "解析 ALI 地址",
            detail: "X-Authlib-Injector-API-Location 头包含非法字符".into(),
        })?;
        let resolved = final_url.join(loc).map_err(|e| AuthError::Response {
            context: "解析 ALI 地址",
            detail: format!("无法解析 API 地址 {loc}: {e}"),
        })?;
        return Ok(ensure_trailing_slash(resolved.as_str()));
    }
    Ok(ensure_trailing_slash(final_url.as_str()))
}

/// 预取 ALI 元数据：解析 API 根地址后 GET 之，返回根地址、Base64 明文与解析后的元数据。
pub async fn prefetch_metadata(client: &reqwest::Client, server_url: &str) -> Result<AliMetadata> {
    let api_root = resolve_api_root(client, server_url).await?;
    let resp = client
        .get(&api_root)
        .send()
        .await
        .map_err(|source| AuthError::Http {
            context: "预取 ALI 元数据",
            source,
        })?;
    let status = resp.status();
    let text = resp.text().await.map_err(|source| AuthError::Http {
        context: "预取 ALI 元数据",
        source,
    })?;
    if !status.is_success() {
        return Err(AuthError::Response {
            context: "预取 ALI 元数据",
            detail: format!("HTTP {status}"),
        });
    }
    let metadata: YggdrasilMetadata = parse(&text, "预取 ALI 元数据")?;
    // 按规范对原样响应字节做 Base64；不重新序列化，避免字段顺序/空白变化。
    let prefetched = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    Ok(AliMetadata {
        api_root,
        prefetched,
        metadata,
    })
}

fn ensure_trailing_slash(url: &str) -> String {
    if url.ends_with('/') {
        url.to_owned()
    } else {
        format!("{url}/")
    }
}

fn parse<T: DeserializeOwned>(text: &str, context: &'static str) -> Result<T> {
    serde_json::from_str(text).map_err(|source| AuthError::Parse { context, source })
}

/// 把 Yggdrasil 错误响应体映射为错误变体；响应体不符合错误格式时归为解析错误。
fn yggdrasil_error(text: &str, context: &'static str) -> AuthError {
    match serde_json::from_str::<YggdrasilErrorBody>(text) {
        Ok(body) => AuthError::Yggdrasil {
            error: body.error,
            error_message: body.error_message,
        },
        Err(source) => AuthError::Parse { context, source },
    }
}

// --- 请求 serde 结构 ---

#[derive(Serialize)]
struct Agent {
    name: &'static str,
    version: u8,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticatePayload<'a> {
    agent: Agent,
    username: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_token: Option<&'a str>,
    request_user: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshPayload<'a> {
    access_token: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_token: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selected_profile: Option<ProfileRef<'a>>,
    request_user: bool,
}

#[derive(Serialize)]
struct ProfileRef<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidatePayload<'a> {
    access_token: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_token: Option<&'a str>,
}

// --- 响应 serde 结构 ---

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticateBody {
    access_token: String,
    client_token: String,
    #[serde(default)]
    available_profiles: Vec<ProfileBody>,
    selected_profile: Option<ProfileBody>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefreshBody {
    access_token: String,
    client_token: String,
    selected_profile: Option<ProfileBody>,
}

#[derive(serde::Deserialize)]
struct ProfileBody {
    id: String,
    name: String,
}

impl From<ProfileBody> for GameProfile {
    fn from(p: ProfileBody) -> Self {
        GameProfile {
            id: p.id,
            name: p.name,
        }
    }
}

impl From<AuthenticateBody> for AuthenticateResponse {
    fn from(b: AuthenticateBody) -> Self {
        Self {
            access_token: b.access_token,
            client_token: b.client_token,
            available_profiles: b.available_profiles.into_iter().map(Into::into).collect(),
            selected_profile: b.selected_profile.map(Into::into),
        }
    }
}

impl From<RefreshBody> for RefreshResponse {
    fn from(b: RefreshBody) -> Self {
        Self {
            access_token: b.access_token,
            client_token: b.client_token,
            selected_profile: b.selected_profile.map(Into::into),
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct YggdrasilErrorBody {
    error: String,
    #[serde(default)]
    error_message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn no_retry() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 1,
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
            multiplier: 1.0,
            jitter: false,
        }
    }

    fn client_for(server: &MockServer) -> YggdrasilClient {
        let http = aurora_base::http::build_client().expect("构建客户端");
        YggdrasilClient::new(http, format!("{}/", server.uri())).with_retry(no_retry())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticate_parses_profiles() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/authenticate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "accessToken": "acc-tok",
                "clientToken": "cli-tok",
                "availableProfiles": [
                    { "id": "1111111111111111aaaaaaaaaaaaaaaa", "name": "Hero" },
                    { "id": "2222222222222222bbbbbbbbbbbbbbbb", "name": "Alt" }
                ],
                "selectedProfile": { "id": "1111111111111111aaaaaaaaaaaaaaaa", "name": "Hero" }
            })))
            .mount(&server)
            .await;

        let resp = client_for(&server)
            .authenticate("user@example.com", "pw", None)
            .await
            .unwrap();
        assert_eq!(resp.access_token, "acc-tok");
        assert_eq!(resp.client_token, "cli-tok");
        assert_eq!(resp.available_profiles.len(), 2);
        assert_eq!(resp.available_profiles[1].name, "Alt");
        assert_eq!(resp.selected_profile.unwrap().name, "Hero");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authenticate_403_surfaces_yggdrasil_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/authenticate"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": "ForbiddenOperationException",
                "errorMessage": "Invalid credentials. Invalid username or password."
            })))
            .mount(&server)
            .await;

        let err = client_for(&server)
            .authenticate("user", "wrong", None)
            .await
            .unwrap_err();
        match err {
            AuthError::Yggdrasil {
                error,
                error_message,
            } => {
                assert_eq!(error, "ForbiddenOperationException");
                assert!(error_message.contains("Invalid credentials"));
            }
            other => panic!("期望 Yggdrasil 错误，得到 {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn refresh_switches_profile() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "accessToken": "new-acc",
                "clientToken": "cli-tok",
                "selectedProfile": { "id": "2222222222222222bbbbbbbbbbbbbbbb", "name": "Alt" }
            })))
            .mount(&server)
            .await;

        let target = GameProfile {
            id: "2222222222222222bbbbbbbbbbbbbbbb".into(),
            name: "Alt".into(),
        };
        let resp = client_for(&server)
            .refresh("old-acc", Some("cli-tok"), Some(&target))
            .await
            .unwrap();
        assert_eq!(resp.access_token, "new-acc");
        assert_eq!(resp.selected_profile.unwrap().name, "Alt");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn validate_true_on_204_false_on_403() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/validate"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        assert!(
            client_for(&server)
                .validate("tok", None)
                .await
                .unwrap()
        );

        let server2 = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/validate"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "error": "ForbiddenOperationException",
                "errorMessage": "Invalid token."
            })))
            .mount(&server2)
            .await;
        assert!(
            !client_for(&server2)
                .validate("tok", None)
                .await
                .unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prefetch_follows_relative_api_location_header() {
        let server = MockServer::start().await;
        let metadata_json = json!({
            "meta": { "serverName": "Aurora Test" },
            "skinDomains": ["example.com", ".littleskin.cn"],
            "signaturePublickey": "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----"
        });
        // 根地址返回相对定位头，不含元数据主体。
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-Authlib-Injector-API-Location", "/api/yggdrasil"),
            )
            .mount(&server)
            .await;
        // 解析出的 API 根地址（带结尾斜杠）返回元数据。
        Mock::given(method("GET"))
            .and(path("/api/yggdrasil/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(metadata_json.clone()))
            .mount(&server)
            .await;

        let http = aurora_base::http::build_client().unwrap();
        let ali = prefetch_metadata(&http, &server.uri()).await.unwrap();

        assert_eq!(ali.api_root, format!("{}/api/yggdrasil/", server.uri()));
        assert_eq!(
            ali.metadata.skin_domains,
            vec!["example.com".to_string(), ".littleskin.cn".to_string()]
        );
        // 预取串 Base64 解码后应是可解析回同一元数据的 JSON。
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(ali.prefetched.as_bytes())
            .unwrap();
        let roundtrip: serde_json::Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(roundtrip["skinDomains"][1], ".littleskin.cn");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prefetch_without_header_uses_response_url() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "skinDomains": ["direct.example"]
            })))
            .mount(&server)
            .await;

        let http = aurora_base::http::build_client().unwrap();
        let ali = prefetch_metadata(&http, &server.uri()).await.unwrap();
        assert_eq!(ali.api_root, format!("{}/", server.uri()));
        assert_eq!(ali.metadata.skin_domains, vec!["direct.example".to_string()]);
    }
}
