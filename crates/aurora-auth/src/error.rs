//! crate 统一错误类型 [`AuthError`]。
//!
//! 每个变体携带足够定位现场的上下文（HTTP 上下文、OAuth/Yggdrasil 的 error 字段、XSTS 错误码）。
//! 令牌刷新失败明确区分“需重登”（[`AuthError::ReloginRequired`]）与瞬时“网络失败”
//! （[`AuthError::Http`]，实现 [`RetryableError`] 后可被上层 `retry_async` 重试）。

use aurora_base::retry::RetryableError;

/// aurora-auth 对外统一错误。
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// 下层 aurora-base 错误（文件、哈希、镜像改写等）冒泡。
    #[error(transparent)]
    Base(#[from] aurora_base::Error),

    /// HTTP 请求发送或读取响应体失败（瞬时网络故障，可重试）。
    #[error("HTTP 请求失败（{context}）")]
    Http {
        context: &'static str,
        #[source]
        source: reqwest::Error,
    },

    /// 响应体 JSON 解析失败（字段缺失或结构不符）。
    #[error("响应解析失败（{context}）")]
    Parse {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// 响应结构不符合协议约定（缺少必需字段、头部畸形等）。
    #[error("响应不符合协议约定（{context}）: {detail}")]
    Response { context: &'static str, detail: String },

    /// 设备码在用户完成授权前已过期，需重新发起登录。
    #[error("设备码已过期，请重新发起登录")]
    DeviceCodeExpired,

    /// 用户在浏览器中拒绝了本次登录授权。
    #[error("用户拒绝了本次登录授权")]
    AuthorizationDeclined,

    /// 微软 OAuth 端点返回的其它错误（非 pending/declined/expired）。
    #[error("微软登录失败: {error} - {description}")]
    OAuth { error: String, description: String },

    /// XSTS：该 Xbox 账户已被封禁。错误码 2148916227。
    #[error("该 Xbox 账户已被封禁，无法登录（XErr 2148916227）")]
    XstsBanned,

    /// XSTS：该微软账户未创建 Xbox 账户。错误码 2148916233。
    #[error("该微软账户尚未创建 Xbox 账户，请先在浏览器登录一次 Xbox（XErr 2148916233）")]
    XstsNoXboxAccount,

    /// XSTS：账户所在国家/地区不支持 Xbox Live。错误码 2148916235。
    #[error("当前账户所在国家或地区不支持 Xbox Live 服务（XErr 2148916235）")]
    XstsRegionUnavailable,

    /// XSTS：需要成人身份验证（韩国地区）。错误码 2148916236 / 2148916237。
    #[error("该账户需要完成成人身份验证后才能使用（XErr {0}）")]
    XstsAdultVerificationRequired(i64),

    /// XSTS：未成年账户，需由家庭组管理员将其加入家庭组。错误码 2148916238。
    #[error("该账户为未成年账户，需先由家庭组管理员将其加入家庭组（XErr 2148916238）")]
    XstsChildAccount,

    /// XSTS：未在已知表内的错误码。
    #[error("XSTS 授权失败，未识别的错误码 XErr {0}")]
    XstsUnknown(i64),

    /// 该账户未拥有 Minecraft（未购买正版或未设置游戏档案）。
    #[error("该账户未拥有 Minecraft（未购买正版或未创建游戏档案）")]
    MinecraftProfileNotFound,

    /// 令牌刷新判定为凭据失效，必须重新走完整登录流程（区别于网络故障）。
    #[error("登录凭据已失效，需要重新登录")]
    ReloginRequired,

    /// 离线用户名不合法（空、含引号、超长）。
    #[error("离线用户名不合法: {0}")]
    InvalidUsername(String),

    /// Yggdrasil（Authlib-Injector）认证服务器返回的业务错误。
    #[error("第三方认证失败: {error} - {error_message}")]
    Yggdrasil {
        error: String,
        error_message: String,
    },

    /// 目标账户不存在（多账户管理的选择/删除）。
    #[error("账户不存在: {0}")]
    AccountNotFound(String),

    /// 凭据缓存序列化失败。
    #[error("凭据序列化失败")]
    CredentialSerialize(#[source] serde_json::Error),

    /// 凭据缓存反序列化失败（文件损坏或格式不兼容）。
    #[error("凭据反序列化失败（凭据文件可能已损坏）")]
    CredentialDeserialize(#[source] serde_json::Error),

    /// Windows DPAPI 加解密失败。message 为格式化后的系统错误，避免 error 层引入平台类型。
    #[error("Windows DPAPI {operation}失败: {message}")]
    Dpapi {
        operation: &'static str,
        message: String,
    },
}

/// crate 级 `Result` 别名。
pub type Result<T> = std::result::Result<T, AuthError>;

impl RetryableError for AuthError {
    fn is_retryable(&self) -> bool {
        match self {
            // 下层错误的可重试性交给 aurora-base 判定（瞬时 IO、哈希不符等）。
            AuthError::Base(inner) => inner.is_retryable(),
            // 连接建立/读取超时属瞬时网络故障，值得重试；解析类 reqwest 错误不重试。
            AuthError::Http { source, .. } => {
                source.is_timeout() || source.is_connect() || source.is_request()
            }
            // 其余均为确定性的业务/协议/凭据错误，重试是同样结果。
            AuthError::Parse { .. }
            | AuthError::Response { .. }
            | AuthError::DeviceCodeExpired
            | AuthError::AuthorizationDeclined
            | AuthError::OAuth { .. }
            | AuthError::XstsBanned
            | AuthError::XstsNoXboxAccount
            | AuthError::XstsRegionUnavailable
            | AuthError::XstsAdultVerificationRequired(_)
            | AuthError::XstsChildAccount
            | AuthError::XstsUnknown(_)
            | AuthError::MinecraftProfileNotFound
            | AuthError::ReloginRequired
            | AuthError::InvalidUsername(_)
            | AuthError::Yggdrasil { .. }
            | AuthError::AccountNotFound(_)
            | AuthError::CredentialSerialize(_)
            | AuthError::CredentialDeserialize(_)
            | AuthError::Dpapi { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relogin_required_is_terminal_not_retryable() {
        // “需重登”是确定性失效，绝不能被当作瞬时网络故障重试。
        assert!(!AuthError::ReloginRequired.is_retryable());
    }

    #[test]
    fn base_error_delegates_retryability() {
        // 下层可重试错误（哈希不符）经包装后仍可重试。
        let base = aurora_base::Error::HashMismatch {
            algorithm: "SHA-1",
            expected: "aa".into(),
            actual: "bb".into(),
        };
        assert!(AuthError::Base(base).is_retryable());
    }

    #[test]
    fn xsts_child_account_message_carries_code() {
        assert!(
            AuthError::XstsChildAccount
                .to_string()
                .contains("2148916238")
        );
        assert!(
            AuthError::XstsAdultVerificationRequired(2148916236)
                .to_string()
                .contains("2148916236")
        );
    }

    #[test]
    fn yggdrasil_error_surfaces_server_fields() {
        let err = AuthError::Yggdrasil {
            error: "ForbiddenOperationException".into(),
            error_message: "Invalid credentials.".into(),
        };
        let text = err.to_string();
        assert!(text.contains("ForbiddenOperationException"));
        assert!(text.contains("Invalid credentials."));
    }
}
