//! 内部 HTTP 发送助手：把「构建请求 -> 发送 -> 状态校验 -> 反序列化」封成一处，并叠加退避重试。
//!
//! 两个平台客户端共用它。`build` 是「每次尝试都重新构造一个 [`reqwest::RequestBuilder`]」的闭包——
//! 退避重试要求每次都是全新请求（query/header/body 重新装配），因此接收闭包而非现成 builder。
//! 仅可重试错误（5xx/429/408、超时/连接类）才会按 [`RetryPolicy`] 退避后再试；4xx 等确定性失败立即冒泡。

use aurora_base::retry::{RetryPolicy, retry_async};
use serde::de::DeserializeOwned;

use crate::error::{Error, Result};

/// 发送请求并把成功响应体反序列化为 `T`。非 2xx 归一到 [`Error::Status`]。
pub(crate) async fn send_json<T, F>(retry: &RetryPolicy, context: &str, build: F) -> Result<T>
where
    T: DeserializeOwned,
    F: Fn() -> reqwest::RequestBuilder,
{
    retry_async(retry, || async {
        let response = build().send().await.map_err(|source| Error::Http {
            context: context.to_string(),
            source,
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Status {
                url: response.url().to_string(),
                status: status.as_u16(),
            });
        }
        response.json::<T>().await.map_err(|source| Error::Http {
            context: context.to_string(),
            source,
        })
    })
    .await
}
