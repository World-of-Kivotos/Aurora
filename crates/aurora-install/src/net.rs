//! 元数据抓取的薄封装。
//!
//! 安装流程里除了「下文件」（走 aurora-download 引擎），还要「读元数据」：版本清单、版本 JSON、
//! Fabric/Quilt meta、Forge maven-metadata。这些是小报文、需在内存里解析，故用注入的
//! `reqwest::Client` 直接 GET，并在退避重试下取回字节（5xx/超时自动重试）。报文一律交给
//! serde_json 自行解析（与 aurora-java 一致），不启用 reqwest 的 json 特性。

use aurora_base::retry::{RetryPolicy, retry_async};

use crate::error::{Error, Result};

/// 带退避重试地 GET 一个 URL 的响应体字节。
pub(crate) async fn get_bytes(
    client: &reqwest::Client,
    url: &str,
    policy: &RetryPolicy,
) -> Result<Vec<u8>> {
    retry_async(policy, || async {
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|source| Error::Http {
                url: url.to_owned(),
                source,
            })?
            .error_for_status()
            .map_err(|source| Error::Http {
                url: url.to_owned(),
                source,
            })?;
        let bytes = resp.bytes().await.map_err(|source| Error::Http {
            url: url.to_owned(),
            source,
        })?;
        Ok::<Vec<u8>, Error>(bytes.to_vec())
    })
    .await
}

/// GET 并用 serde_json 反序列化，失败时带上报文上下文。
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    policy: &RetryPolicy,
    context: &str,
) -> Result<T> {
    let bytes = get_bytes(client, url, policy).await?;
    serde_json::from_slice(&bytes).map_err(|source| Error::Json {
        context: context.to_owned(),
        source,
    })
}
