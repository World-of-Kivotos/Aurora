//! 账户操作：离线账户创建、微软设备码登录与多账户读取。
//!
//! 微软登录的编排（设备码 -> 轮询 -> 令牌链 -> 落库）抽成与平台无关的 [`perform_microsoft_login`]，
//! 便于用注入端点的 [`MicrosoftAuth`] 与任意 [`CredentialStore`] 做 mock 测试。凭据的加密落盘只有
//! Windows（DPAPI）实现，故门面上「读写账户库」的方法用 `#[cfg(windows)]` 圈起；离线账户创建不依赖
//! 凭据库，跨平台可用。

use aurora_auth::{
    Account, AccountCredentials, AccountManager, CredentialStore, DeviceCodeResponse, MicrosoftAuth,
    MicrosoftCredentials, MicrosoftSession, UsernameCheck, offline_account, validate_username,
};

use crate::error::{CoreError, Result};
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::Aurora;

/// 环境变量名：微软登录 client_id 的调试回落来源。
pub const MSA_CLIENT_ID_ENV: &str = "AURORA_MSA_CLIENT_ID";

/// 走完微软设备码登录全链并把结果账户写入账户库。
///
/// 流程：请求设备码 -> `on_code` 回调（供 UI 展示 user_code 与验证网址）-> 轮询令牌 -> XBL/XSTS/
/// Minecraft/profile -> 组装账户 -> upsert 落库。返回落库后的账户。
pub async fn perform_microsoft_login<S: CredentialStore>(
    auth: &MicrosoftAuth,
    manager: &mut AccountManager<S>,
    on_code: impl FnOnce(&DeviceCodeResponse),
) -> Result<Account> {
    let device = auth.begin_device_code().await?;
    on_code(&device);
    let token = auth.poll_device_code(&device).await?;
    let session = auth.complete_login(&token).await?;
    let account = account_from_session(&session);
    manager.upsert(account.clone())?;
    Ok(account)
}

/// 把一次微软会话摊平成可持久化的账户记录（缓存 Minecraft 令牌与到期时间，供下次免握手启动）。
fn account_from_session(session: &MicrosoftSession) -> Account {
    Account::new(
        session.profile.id.clone(),
        session.profile.name.clone(),
        AccountCredentials::Microsoft(MicrosoftCredentials {
            refresh_token: session.refresh_token.clone(),
            minecraft_token: Some(session.minecraft_token.clone()),
            minecraft_expires_at: Some(session.minecraft_expires_at),
        }),
    )
}

impl Aurora {
    /// 解析微软登录 client_id：优先配置，其次环境变量；都缺失报 [`CoreError::MissingClientId`]。
    pub(crate) fn msa_client_id(&self) -> Result<String> {
        self.config()
            .msa_client_id
            .clone()
            .or_else(|| {
                std::env::var(MSA_CLIENT_ID_ENV)
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .ok_or(CoreError::MissingClientId)
    }

    /// 创建一个离线账户（不落库，供离线启动即用即弃）。
    ///
    /// 用户名先过硬性校验（空/引号/超长直接报错），软性告警（含非标准字符）经事件通道上抛。
    pub fn create_offline_account(
        &self,
        name: &str,
        events: Option<&EventSink>,
    ) -> Result<Account> {
        let UsernameCheck { warnings } = validate_username(name)?;
        for warning in warnings {
            emit(events, CoreEvent::warning(warning));
        }
        Ok(offline_account(name)?)
    }
}

#[cfg(windows)]
impl Aurora {
    /// 打开当前数据目录下的加密账户库（DPAPI）。
    fn open_accounts(&self) -> Result<AccountManager<aurora_auth::DpapiCredentialStore>> {
        let store = aurora_auth::DpapiCredentialStore::at(self.data_dir().join("credentials.bin"));
        Ok(AccountManager::load(store)?)
    }

    /// 走微软设备码登录并把账户写入加密账户库，返回登录到的账户。
    pub async fn microsoft_login(
        &self,
        on_code: impl FnOnce(&DeviceCodeResponse),
    ) -> Result<Account> {
        let client_id = self.msa_client_id()?;
        let auth = MicrosoftAuth::new(self.http(), client_id);
        let mut manager = self.open_accounts()?;
        perform_microsoft_login(&auth, &mut manager, on_code).await
    }

    /// 读取账户库中的全部账户。
    pub fn accounts(&self) -> Result<Vec<Account>> {
        Ok(self.open_accounts()?.accounts().to_vec())
    }

    /// 读取当前选中账户（若有）。
    pub fn current_account(&self) -> Result<Option<Account>> {
        Ok(self.open_accounts()?.current().cloned())
    }

    /// 切换当前账户。
    pub fn set_current_account(&self, uuid: &str) -> Result<()> {
        let mut manager = self.open_accounts()?;
        manager.set_current(uuid)?;
        Ok(())
    }

    /// 删除账户。
    pub fn remove_account(&self, uuid: &str) -> Result<()> {
        let mut manager = self.open_accounts()?;
        manager.remove(uuid)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_auth::Result as AuthResult;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 测试用不加密内存凭据库：验证登录编排的落库效果。
    #[derive(Default, Clone)]
    struct MemStore {
        bytes: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl CredentialStore for MemStore {
        fn load(&self) -> AuthResult<Option<Vec<u8>>> {
            Ok(self.bytes.lock().unwrap().clone())
        }
        fn save(&self, plaintext: &[u8]) -> AuthResult<()> {
            *self.bytes.lock().unwrap() = Some(plaintext.to_vec());
            Ok(())
        }
    }

    fn no_retry() -> aurora_base::retry::RetryPolicy {
        aurora_base::retry::RetryPolicy {
            max_attempts: 1,
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            multiplier: 1.0,
            jitter: false,
        }
    }

    fn auth_for(server: &MockServer) -> MicrosoftAuth {
        let client = aurora_base::http::build_client().unwrap();
        let endpoints = aurora_auth::MsaEndpoints {
            oauth_base: server.uri(),
            xbl_authenticate: format!("{}/xbl", server.uri()),
            xsts_authorize: format!("{}/xsts", server.uri()),
            minecraft_base: server.uri(),
        };
        MicrosoftAuth::new(client, "test-client-id")
            .with_endpoints(endpoints)
            .with_retry(no_retry())
    }

    async fn mount_full_login(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/devicecode"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"device_code":"DEV","user_code":"WXYZ-9999",
                    "verification_uri":"https://microsoft.com/link","expires_in":900,"interval":0,
                    "message":"输入 WXYZ-9999"}"#,
            ))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"AT","refresh_token":"rotated-refresh","expires_in":3600}"#,
            ))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/xbl"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"theuhs"}]}}"#,
            ))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/xsts"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"theuhs"}]}}"#,
            ))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/authentication/login_with_xbox"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"MC-TOKEN","expires_in":86400}"#,
            ))
            .mount(server)
            .await;
        Mock::given(method("GET"))
            .and(path("/minecraft/profile"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"id":"0123456789abcdef0123456789abcdef","name":"AuroraPlayer"}"#,
            ))
            .mount(server)
            .await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn microsoft_login_persists_account_and_caches_token() {
        let server = MockServer::start().await;
        mount_full_login(&server).await;

        let store = MemStore::default();
        let mut manager = AccountManager::load(store.clone()).unwrap();
        let auth = auth_for(&server);

        let mut shown = None;
        let account = perform_microsoft_login(&auth, &mut manager, |dc| {
            shown = Some(dc.user_code.clone());
        })
        .await
        .unwrap();

        // 回调拿到了 user_code。
        assert_eq!(shown.as_deref(), Some("WXYZ-9999"));
        // 账户字段正确。
        assert_eq!(account.uuid, "0123456789abcdef0123456789abcdef");
        assert_eq!(account.name, "AuroraPlayer");
        match &account.credentials {
            AccountCredentials::Microsoft(c) => {
                assert_eq!(c.refresh_token, "rotated-refresh");
                assert_eq!(c.minecraft_token.as_deref(), Some("MC-TOKEN"));
                assert!(c.minecraft_expires_at.unwrap() > 0);
            }
            other => panic!("期望 Microsoft 凭据，得到 {other:?}"),
        }

        // 从同一份底层字节重载，账户与「当前」应还原。
        let reloaded = AccountManager::load(store).unwrap();
        assert_eq!(reloaded.accounts().len(), 1);
        assert_eq!(
            reloaded.current().unwrap().uuid,
            "0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn offline_account_creation_validates_and_yields_stable_uuid() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = Aurora::for_test(
            crate::config::AuroraConfig::default(),
            tmp.path().to_path_buf(),
            tmp.path().to_path_buf(),
        );

        // 合法用户名 -> 稳定离线 UUID（与原版一致）。
        let account = aurora.create_offline_account("Steve", None).unwrap();
        assert_eq!(account.uuid, "5627dd98e6be3c21b8a8e92344183641");
        assert_eq!(account.account_type, aurora_auth::AccountType::Offline);

        // 含非标准字符 -> 通过但发出告警事件。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = aurora.create_offline_account("玩家一", Some(&tx)).unwrap();
        drop(tx);
        let mut warned = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, CoreEvent::Warning(_)) {
                warned = true;
            }
        }
        assert!(warned, "非标准字符用户名应发出告警事件");

        // 非法用户名（空）冒泡。
        assert!(matches!(
            aurora.create_offline_account("", None),
            Err(CoreError::Auth(_))
        ));
    }

    #[test]
    fn missing_client_id_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::config::AuroraConfig {
            msa_client_id: None,
            ..crate::config::AuroraConfig::default()
        };
        let aurora = Aurora::for_test(config, tmp.path().to_path_buf(), tmp.path().to_path_buf());
        // 未配置且（大概率）无环境变量时报 MissingClientId。
        if std::env::var(MSA_CLIENT_ID_ENV).is_err() {
            assert!(matches!(aurora.msa_client_id(), Err(CoreError::MissingClientId)));
        }
    }
}
