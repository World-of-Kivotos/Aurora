//! 账户操作：离线账户创建、微软设备码登录与多账户读取。
//!
//! 微软登录的编排（设备码 -> 轮询 -> 令牌链 -> 落库）抽成与平台无关的 [`perform_microsoft_login`]，
//! 便于用注入端点的 [`MicrosoftAuth`] 与任意 [`CredentialStore`] 做 mock 测试。凭据的加密落盘只有
//! Windows（DPAPI）实现，故登录本身用 `#[cfg(windows)]` 圈起。
//!
//! 账户分两个库：带凭据的（微软/外置）落 DPAPI 加密的 `credentials.bin`，离线账户落明文
//! `offline_accounts.json`（理由见 [`aurora_auth::offline_store`]）。对外的读取/切换/删除四个方法
//! 跨两库统一寻址，故它们本身是跨平台的——非 Windows 上凭据库那一半恒为空，离线那一半照常可用。

use aurora_auth::{
    Account, AccountCredentials, AccountManager, AuthError, CredentialStore, DeviceCodeResponse,
    GameProfile, MicrosoftAuth, MicrosoftCredentials, MicrosoftSession, OfflineAccountStore,
    UsernameCheck, YggdrasilClient, YggdrasilCredentials, offline_account, validate_username,
};

use crate::error::Result;
use crate::event::{CoreEvent, EventSink, emit};
use crate::facade::Aurora;

/// 环境变量名：微软登录 client_id 的回落来源。
pub const MSA_CLIENT_ID_ENV: &str = "AURORA_MSA_CLIENT_ID";

/// 内置默认微软登录 client_id：Aurora 自有的 Azure AD 公共客户端应用（受支持账户类型＝个人 Microsoft 账户）。
///
/// 注意两点：1) 该应用须在 Azure「身份验证」里开启「允许公共客户端流」，设备码流才成立；
/// 2) 走完设备码/XBL/XSTS 后，若最终 `login_with_xbox` 换 Minecraft 令牌那步返 403，说明该应用尚未通过
/// aka.ms/mce-reviewappid 的 Mojang 审批（审批通过后此步即放行）。
/// 旧的 login.live 调试 id `00000000402B5328` 不适用本项目的 Azure AD v2 端点（报 AADSTS700016），已弃用。
/// 用户在 config.json 填 msa_client_id 或设环境变量 AURORA_MSA_CLIENT_ID 均可覆盖此默认。
pub const DEFAULT_MSA_CLIENT_ID: &str = "bf8c139d-45e9-48c0-b469-175e8234e516";

/// 离线账户库文件名，与 `config.json` 同挂在数据目录下。明文 JSON，可随数据目录整包迁移。
pub const OFFLINE_ACCOUNTS_FILE: &str = "offline_accounts.json";

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

/// 用持久化的 refresh_token 静默续期微软账户：重跑 MSA 刷新 -> XBL/XSTS/Minecraft/profile 换新令牌，
/// 按 uuid upsert 回写账户库（轮换后的 refresh_token 与新 Minecraft 令牌一并落盘），返回续好的账户。
///
/// 不含「是否需要续期」的判断——由调用方（[`Aurora::ensure_microsoft_fresh`]）先查缓存有效期再决定是否调用。
pub async fn perform_microsoft_refresh<S: CredentialStore>(
    auth: &MicrosoftAuth,
    manager: &mut AccountManager<S>,
    refresh_token: &str,
) -> Result<Account> {
    let session = auth.refresh_session(refresh_token).await?;
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

/// 走完 Authlib-Injector（Yggdrasil）用户名密码登录并把结果账户写入账户库。
///
/// 流程：`authenticate` -> 选定角色 -> 组装账户 -> upsert 落库。返回落库后的账户。
/// `api_root`（已由 [`resolve_api_root`](aurora_auth::yggdrasil::resolve_api_root) 解析）连同已
/// 构造的 `client` 一并传入，既复用同一 HTTP 客户端，也便于用 mock 认证端点做落库测试。
pub async fn perform_authlib_login<S: CredentialStore>(
    client: &YggdrasilClient,
    api_root: &str,
    manager: &mut AccountManager<S>,
    username: &str,
    password: &str,
) -> Result<Account> {
    let resp = client.authenticate(username, password, None).await?;
    let profile = select_profile(resp.available_profiles, resp.selected_profile)?;
    // api_root 单独持有：YggdrasilClient 内部的根地址不对外暴露，凭据需自带以供刷新/校验复用。
    let account = Account::new(
        profile.id,
        profile.name,
        AccountCredentials::AuthlibInjector(YggdrasilCredentials {
            api_root: api_root.to_owned(),
            access_token: resp.access_token,
            client_token: resp.client_token,
        }),
    );
    manager.upsert(account.clone())?;
    Ok(account)
}

/// 选定登录角色：优先服务端已选中角色，否则取可用角色列表首个。
///
/// 多角色账号的交互式选择（列出全部角色供用户点选）留待后续 UI；当前按「首个可用」自动定角色。
/// 账户下无任何角色时认证虽成功却无从组装档案，按协议不符冒泡为 [`AuthError::Response`]。
fn select_profile(
    available: Vec<GameProfile>,
    selected: Option<GameProfile>,
) -> Result<GameProfile> {
    selected
        .or_else(|| available.into_iter().next())
        .ok_or_else(|| {
            AuthError::Response {
                context: "Yggdrasil 认证",
                detail: "认证成功但账户下无可用角色，请先在验证服务器创建游戏角色".into(),
            }
            .into()
        })
}

impl Aurora {
    /// 解析微软登录 client_id：优先配置，其次环境变量，最后回落到内置默认（[`DEFAULT_MSA_CLIENT_ID`]），
    /// 保证正版登录开箱可用。返回 Result 仅为与调用点的 `?` 保持一致，实际恒为 Ok。
    pub(crate) fn msa_client_id(&self) -> Result<String> {
        Ok(self
            .config()
            .msa_client_id
            .clone()
            .or_else(|| {
                std::env::var(MSA_CLIENT_ID_ENV)
                    .ok()
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or_else(|| DEFAULT_MSA_CLIENT_ID.to_owned()))
    }

    /// 就地造一个离线账户（不落库，供 `launch_offline` 那种「给个名字直接开」的即用即弃场景）。
    ///
    /// 用户名先过硬性校验（空/引号/超长直接报错），软性告警（含非标准字符）经事件通道上抛。
    /// 要让账户在下次开启动器时还在，用 [`Aurora::add_offline_account`]。
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

    /// 打开离线账户库（明文 JSON，跨平台可用）。
    fn open_offline_accounts(&self) -> Result<OfflineAccountStore> {
        Ok(OfflineAccountStore::load(
            self.data_dir().join(OFFLINE_ACCOUNTS_FILE),
        )?)
    }

    /// 保存一个离线账户并把它设为当前账户，返回落库后的账户。
    ///
    /// 校验与告警走与 [`Aurora::create_offline_account`] 同一条路（硬性错误冒泡、软性告警经事件通道），
    /// 保证「即用即弃」与「持久保存」两条入口对用户名的判定完全一致。同名重复保存是幂等的。
    pub fn add_offline_account(&self, name: &str, events: Option<&EventSink>) -> Result<Account> {
        let UsernameCheck { warnings } = validate_username(name)?;
        for warning in warnings {
            emit(events, CoreEvent::warning(warning));
        }
        Ok(self.open_offline_accounts()?.add(name)?)
    }

    /// 已保存的离线账户列表。
    pub fn offline_accounts(&self) -> Result<Vec<Account>> {
        Ok(self.open_offline_accounts()?.accounts())
    }

    /// 全部账户：凭据库（微软/外置）在前，离线账户在后。
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let mut accounts = self.credential_accounts()?;
        accounts.extend(self.open_offline_accounts()?.accounts());
        Ok(accounts)
    }

    /// 按 uuid 跨两库查账户（含令牌，供启动路径取用）。
    pub fn find_account(&self, uuid: &str) -> Result<Option<Account>> {
        if let Some(account) = self.open_offline_accounts()?.find(uuid) {
            return Ok(Some(account));
        }
        Ok(self
            .credential_accounts()?
            .into_iter()
            .find(|a| a.uuid == uuid))
    }

    /// 当前账户（若有）。
    ///
    /// 两个库各自记着自己的「当前」，仲裁规则只有一条：离线库里存着选中项时它就是全局当前账户。
    /// 这条规则成立的前提是 [`Aurora::set_current_account`] 切到微软/外置账户时会清空离线选中项，
    /// 二者必须一起看。
    pub fn current_account(&self) -> Result<Option<Account>> {
        if let Some(account) = self.open_offline_accounts()?.current() {
            return Ok(Some(account));
        }
        self.credential_current()
    }

    /// 切换当前账户（uuid 属哪个库自动判定）。
    pub fn set_current_account(&self, uuid: &str) -> Result<()> {
        let mut offline = self.open_offline_accounts()?;
        if offline.find(uuid).is_some() {
            return Ok(offline.set_current(uuid)?);
        }
        // 目标是微软/外置账户：先把凭据库那边的选择落定，成功后才清掉离线选中项——
        // 顺序反过来的话，凭据库那步失败就会留下「两个库都没选中」的空当。
        self.credential_set_current(uuid)?;
        Ok(offline.clear_current()?)
    }

    /// 删除账户（uuid 属哪个库自动判定）。
    ///
    /// 删掉凭据账户后必须补一次选中权回落：切到凭据账户的那一刻离线选中项已被清空（见
    /// [`Aurora::set_current_account`]），若删掉的又正是最后一个凭据账户，两个库就会同时没有选中项——
    /// 账户列表里明明还留着离线号，当前账户却是空的，用户得手动再点一次「设为当前」才能开游戏。
    /// 回落到剩余离线账户的第一个，与 [`OfflineAccountStore::remove`] 自身的回落语义一致。
    pub fn remove_account(&self, uuid: &str) -> Result<()> {
        let mut offline = self.open_offline_accounts()?;
        if offline.find(uuid).is_some() {
            return Ok(offline.remove(uuid)?);
        }
        self.credential_remove(uuid)?;
        if self.credential_current()?.is_none()
            && offline.current().is_none()
            && let Some(first) = offline.accounts().into_iter().next()
        {
            offline.set_current(&first.uuid)?;
        }
        Ok(())
    }
}

// ---- 凭据库（微软/外置）一侧的跨平台缝 ----
//
// 凭据加密只有 Windows(DPAPI) 实现。非 Windows 上「读」返回空、「写」报账户不存在——那里确实没有
// 任何凭据账户，这是如实回答而非兜底掩盖（离线账户走另一个库，不受影响）。

#[cfg(windows)]
impl Aurora {
    fn credential_accounts(&self) -> Result<Vec<Account>> {
        Ok(self.open_accounts()?.accounts().to_vec())
    }

    fn credential_current(&self) -> Result<Option<Account>> {
        Ok(self.open_accounts()?.current().cloned())
    }

    fn credential_set_current(&self, uuid: &str) -> Result<()> {
        let mut manager = self.open_accounts()?;
        manager.set_current(uuid)?;
        Ok(())
    }

    fn credential_remove(&self, uuid: &str) -> Result<()> {
        let mut manager = self.open_accounts()?;
        manager.remove(uuid)?;
        Ok(())
    }
}

#[cfg(not(windows))]
impl Aurora {
    fn credential_accounts(&self) -> Result<Vec<Account>> {
        Ok(Vec::new())
    }

    fn credential_current(&self) -> Result<Option<Account>> {
        Ok(None)
    }

    fn credential_set_current(&self, uuid: &str) -> Result<()> {
        Err(AuthError::AccountNotFound(uuid.to_owned()).into())
    }

    fn credential_remove(&self, uuid: &str) -> Result<()> {
        Err(AuthError::AccountNotFound(uuid.to_owned()).into())
    }
}

#[cfg(windows)]
impl Aurora {
    /// 打开当前数据目录下的加密账户库（DPAPI）。
    fn open_accounts(&self) -> Result<AccountManager<aurora_auth::DpapiCredentialStore>> {
        let store = aurora_auth::DpapiCredentialStore::at(self.data_dir().join("credentials.bin"));
        Ok(AccountManager::load(store)?)
    }

    /// 把刚登录成功的账户落定为当前账户。
    ///
    /// 登录本身只往凭据库里 upsert，而 [`Aurora::current_account`] 的仲裁是「离线库存着选中项就赢」；
    /// 不走这一步的话，玩家在选着某个离线账户时登录微软/外置，登完界面上的当前账户还是那个离线号，
    /// 点开始游戏进的也仍是离线号。落定当前账户同时清掉离线选中项，两个库的选择因此永远只有一处。
    fn adopt_as_current(&self, account: Account) -> Result<Account> {
        self.set_current_account(&account.uuid)?;
        Ok(account)
    }

    /// 走微软设备码登录并把账户写入加密账户库，返回登录到的账户（登录后即为当前账户）。
    pub async fn microsoft_login(
        &self,
        on_code: impl FnOnce(&DeviceCodeResponse),
    ) -> Result<Account> {
        let client_id = self.msa_client_id()?;
        let auth = MicrosoftAuth::new(self.http(), client_id);
        let mut manager = self.open_accounts()?;
        let account = perform_microsoft_login(&auth, &mut manager, on_code).await?;
        drop(manager);
        self.adopt_as_current(account)
    }

    /// 确保要启动的微软账户持有有效 Minecraft 令牌：缓存令牌在当前时刻仍有效则原样返回（不联网、不开库）；
    /// 已过期或缺失则用 refresh_token 静默续期并回写账户库，返回续好的账户。非微软账户原样返回。
    ///
    /// refresh_token 也失效（满 90 天 / 被吊销）时，续期冒泡 [`aurora_auth::AuthError`]，由上层提示重新登录。
    pub async fn ensure_microsoft_fresh(&self, account: &Account) -> Result<Account> {
        let AccountCredentials::Microsoft(creds) = &account.credentials else {
            return Ok(account.clone());
        };
        // 现实时刻若早于 1970（时钟异常）取 0，令缓存判定为过期而走续期——宁可多刷新，不拿废令牌启动。
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if creds.minecraft_token_valid_at(now) {
            return Ok(account.clone());
        }
        let client_id = self.msa_client_id()?;
        let auth = MicrosoftAuth::new(self.http(), client_id);
        let mut manager = self.open_accounts()?;
        perform_microsoft_refresh(&auth, &mut manager, &creds.refresh_token).await
    }

    /// 走 Authlib-Injector 用户名密码登录并把账户写入加密账户库，返回登录到的账户。
    ///
    /// `server_url` 为用户填写的第三方验证服务器地址：先经 `resolve_api_root` 解析出真正的 API
    /// 根地址（跟随重定向与 `X-Authlib-Injector-API-Location` 头），再据此构造 Yggdrasil 客户端
    /// 完成认证与落库。解析出的根地址随凭据一并存储，供后续刷新/校验与 javaagent 拼装复用。
    /// 登录后该账户即为当前账户。
    pub async fn authlib_login(
        &self,
        server_url: &str,
        username: &str,
        password: &str,
    ) -> Result<Account> {
        let api_root = aurora_auth::yggdrasil::resolve_api_root(&self.http(), server_url).await?;
        let client = YggdrasilClient::new(self.http(), &api_root);
        let mut manager = self.open_accounts()?;
        let account =
            perform_authlib_login(&client, &api_root, &mut manager, username, password).await?;
        drop(manager);
        self.adopt_as_current(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
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
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    r#"{"Token":"xbl","DisplayClaims":{"xui":[{"uhs":"theuhs"}]}}"#,
                ),
            )
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/xsts"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    r#"{"Token":"xsts","DisplayClaims":{"xui":[{"uhs":"theuhs"}]}}"#,
                ),
            )
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path("/authentication/login_with_xbox"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"access_token":"MC-TOKEN","expires_in":86400}"#),
            )
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn microsoft_refresh_swaps_token_and_persists() {
        let server = MockServer::start().await;
        mount_full_login(&server).await; // 复用全链 mock；刷新走 /token(refresh) -> xbl/xsts/mc/profile

        let store = MemStore::default();
        let mut manager = AccountManager::load(store.clone()).unwrap();
        // 先塞一个同 uuid 的旧账户，Minecraft 令牌已过期（到期时刻=1）。
        manager
            .upsert(Account::new(
                "0123456789abcdef0123456789abcdef",
                "OldName",
                AccountCredentials::Microsoft(MicrosoftCredentials {
                    refresh_token: "old-refresh".into(),
                    minecraft_token: Some("OLD-MC".into()),
                    minecraft_expires_at: Some(1),
                }),
            ))
            .unwrap();
        let auth = auth_for(&server);

        let refreshed = perform_microsoft_refresh(&auth, &mut manager, "old-refresh")
            .await
            .unwrap();

        // 换到新 Minecraft 令牌与轮换后的 refresh_token；到期时刻推到未来。
        match &refreshed.credentials {
            AccountCredentials::Microsoft(c) => {
                assert_eq!(c.refresh_token, "rotated-refresh");
                assert_eq!(c.minecraft_token.as_deref(), Some("MC-TOKEN"));
                assert!(c.minecraft_expires_at.unwrap() > 1);
            }
            other => panic!("期望 Microsoft 凭据，得到 {other:?}"),
        }
        // 同 uuid 原地替换，未新增账户；重载后新令牌确已落盘（删掉 upsert 的落盘即挂）。
        assert_eq!(manager.accounts().len(), 1);
        let reloaded = AccountManager::load(store).unwrap();
        match &reloaded
            .find("0123456789abcdef0123456789abcdef")
            .unwrap()
            .credentials
        {
            AccountCredentials::Microsoft(c) => {
                assert_eq!(c.refresh_token, "rotated-refresh");
                assert_eq!(c.minecraft_token.as_deref(), Some("MC-TOKEN"));
            }
            other => panic!("期望 Microsoft 凭据，得到 {other:?}"),
        }
    }

    /// 构造指向 mock 服务器、关闭重试的 Yggdrasil 客户端。
    fn yggdrasil_client_for(api_root: &str) -> YggdrasilClient {
        let client = aurora_base::http::build_client().unwrap();
        YggdrasilClient::new(client, api_root).with_retry(no_retry())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authlib_login_persists_authlib_account() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/authenticate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"accessToken":"acc-tok","clientToken":"cli-tok",
                    "availableProfiles":[{"id":"aaaa1111aaaa1111aaaa1111aaaa1111","name":"Hero"}],
                    "selectedProfile":{"id":"aaaa1111aaaa1111aaaa1111aaaa1111","name":"Hero"}}"#,
            ))
            .mount(&server)
            .await;

        let api_root = format!("{}/", server.uri());
        let client = yggdrasil_client_for(&api_root);
        let store = MemStore::default();
        let mut manager = AccountManager::load(store.clone()).unwrap();

        let account =
            perform_authlib_login(&client, &api_root, &mut manager, "user@example.com", "pw")
                .await
                .unwrap();

        assert_eq!(account.uuid, "aaaa1111aaaa1111aaaa1111aaaa1111");
        assert_eq!(account.name, "Hero");
        assert_eq!(
            account.account_type,
            aurora_auth::AccountType::AuthlibInjector
        );
        match &account.credentials {
            AccountCredentials::AuthlibInjector(c) => {
                assert_eq!(c.api_root, api_root);
                assert_eq!(c.access_token, "acc-tok");
                assert_eq!(c.client_token, "cli-tok");
            }
            other => panic!("期望 AuthlibInjector 凭据，得到 {other:?}"),
        }

        // 从同一份底层字节重载，账户与「当前」应还原（首个账户自动成为当前）。
        let reloaded = AccountManager::load(store).unwrap();
        assert_eq!(reloaded.accounts().len(), 1);
        assert_eq!(
            reloaded.current().unwrap().uuid,
            "aaaa1111aaaa1111aaaa1111aaaa1111"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authlib_login_falls_back_to_first_available_profile() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/authserver/authenticate"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"accessToken":"acc","clientToken":"cli",
                    "availableProfiles":[
                        {"id":"1111111111111111aaaaaaaaaaaaaaaa","name":"First"},
                        {"id":"2222222222222222bbbbbbbbbbbbbbbb","name":"Second"}],
                    "selectedProfile":null}"#,
            ))
            .mount(&server)
            .await;

        let api_root = format!("{}/", server.uri());
        let client = yggdrasil_client_for(&api_root);
        let mut manager = AccountManager::load(MemStore::default()).unwrap();

        let account = perform_authlib_login(&client, &api_root, &mut manager, "u", "p")
            .await
            .unwrap();

        // 无选中角色时定为第一个可用角色。
        assert_eq!(account.uuid, "1111111111111111aaaaaaaaaaaaaaaa");
        assert_eq!(account.name, "First");
    }

    #[test]
    fn select_profile_prefers_selected_over_available() {
        let available = vec![GameProfile {
            id: "aaaa".into(),
            name: "Available".into(),
        }];
        let selected = GameProfile {
            id: "zzzz".into(),
            name: "Selected".into(),
        };
        let chosen = select_profile(available, Some(selected)).unwrap();
        assert_eq!(chosen.id, "zzzz");
        assert_eq!(chosen.name, "Selected");
    }

    #[test]
    fn select_profile_errors_when_account_has_no_profile() {
        let err = select_profile(Vec::new(), None).unwrap_err();
        assert!(matches!(
            err,
            CoreError::Auth(AuthError::Response { context, .. }) if context == "Yggdrasil 认证"
        ));
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

    /// 用同一个数据目录重开一个门面，等价于「关掉启动器再打开」。
    fn aurora_at(data_dir: &std::path::Path) -> Aurora {
        Aurora::for_test(
            crate::config::AuroraConfig::default(),
            data_dir.to_path_buf(),
            data_dir.to_path_buf(),
        )
    }

    #[test]
    fn added_offline_accounts_survive_restart_with_selection() {
        let tmp = tempfile::tempdir().unwrap();
        {
            let aurora = aurora_at(tmp.path());
            aurora.add_offline_account("Steve", None).unwrap();
            aurora.add_offline_account("Alex", None).unwrap();
        }

        // 换一个门面实例重新读盘：账户与当前选中都必须还在（删掉 add 的落盘此断言即挂）。
        let restarted = aurora_at(tmp.path());
        let names: Vec<_> = restarted
            .offline_accounts()
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, ["Steve", "Alex"]);
        assert_eq!(restarted.current_account().unwrap().unwrap().name, "Alex");
        // 统一列表里也看得到（此处凭据库为空，故全部来自离线库）。
        assert_eq!(restarted.accounts().unwrap().len(), 2);

        // 明文落点就在数据目录下，与 config.json 同级。
        assert!(tmp.path().join(OFFLINE_ACCOUNTS_FILE).is_file());
    }

    #[test]
    fn offline_uuid_stays_stable_across_restart_and_readd() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        let created = aurora.add_offline_account("Steve", None).unwrap();
        // 与原版 UUID.nameUUIDFromBytes 一致的固定值——换了它，老存档里的玩家数据就对不上了。
        assert_eq!(created.uuid, "5627dd98e6be3c21b8a8e92344183641");

        aurora.remove_account(&created.uuid).unwrap();
        let again = aurora_at(tmp.path())
            .add_offline_account("Steve", None)
            .unwrap();
        assert_eq!(again.uuid, created.uuid);
    }

    #[test]
    fn switching_and_removing_offline_accounts_moves_the_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        let steve = aurora.add_offline_account("Steve", None).unwrap();
        let alex = aurora.add_offline_account("Alex", None).unwrap();

        aurora.set_current_account(&steve.uuid).unwrap();
        assert_eq!(
            aurora_at(tmp.path())
                .current_account()
                .unwrap()
                .unwrap()
                .uuid,
            steve.uuid
        );

        // 删掉当前选中项 -> 回落到剩下的第一个，且这一回落也要落盘。
        aurora.remove_account(&steve.uuid).unwrap();
        let restarted = aurora_at(tmp.path());
        assert_eq!(
            restarted.current_account().unwrap().unwrap().uuid,
            alex.uuid
        );
        assert!(restarted.find_account(&steve.uuid).unwrap().is_none());

        // 删光之后不再有当前账户。
        restarted.remove_account(&alex.uuid).unwrap();
        assert!(restarted.current_account().unwrap().is_none());
        assert!(restarted.offline_accounts().unwrap().is_empty());
    }

    #[test]
    fn find_account_resolves_persisted_offline_uuid() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        let created = aurora.add_offline_account("Steve", None).unwrap();

        // 启动路径按 uuid 取账户：离线账户也必须查得到，否则「当前账户」点了启动会报账户不存在。
        let found = aurora_at(tmp.path())
            .find_account(&created.uuid)
            .unwrap()
            .expect("已保存的离线账户应能按 uuid 查到");
        assert_eq!(found.name, "Steve");
        assert_eq!(found.account_type, aurora_auth::AccountType::Offline);
        assert!(
            aurora
                .find_account("ffffffffffffffffffffffffffffffff")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn add_offline_account_validates_like_the_ephemeral_path() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());

        // 非法用户名冒泡，且一个字节都不落盘。
        assert!(matches!(
            aurora.add_offline_account("bad\"name", None),
            Err(CoreError::Auth(_))
        ));
        assert!(!tmp.path().join(OFFLINE_ACCOUNTS_FILE).exists());

        // 含非标准字符：保存成功，同时经事件通道发出告警。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let saved = aurora.add_offline_account("玩家一", Some(&tx)).unwrap();
        drop(tx);
        assert_eq!(saved.name, "玩家一");
        let mut warned = false;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, CoreEvent::Warning(_)) {
                warned = true;
            }
        }
        assert!(warned, "非标准字符用户名应发出告警事件");
        assert_eq!(aurora_at(tmp.path()).offline_accounts().unwrap().len(), 1);
    }

    /// 登录成功必须把「当前账户」从离线号手里接过来。
    ///
    /// 这是两个库共存后最容易漏的一处：登录只往凭据库 upsert，而 current_account 的仲裁让离线库先赢，
    /// 于是玩家选着离线号去登微软，登完当前账户还是离线号，点开始游戏进的也是离线号。
    /// 走 adopt_as_current（登录方法末尾调的正是它）才成立——把那一句删掉，或把
    /// set_current_account 里的 clear_current 删掉，本用例即挂。
    ///
    /// 只在 Windows 跑：凭据库是 DPAPI，别的平台上根本没有「凭据账户」这一半。
    #[cfg(windows)]
    #[test]
    fn login_takes_over_the_current_account_from_the_offline_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());

        let offline = aurora.add_offline_account("Steve", None).unwrap();
        assert_eq!(
            aurora.current_account().unwrap().unwrap().uuid,
            offline.uuid,
            "刚保存的离线账户应当是当前账户"
        );

        // 登录落库那一步：真登录只是多走一趟网络，最终动作就是把账户 upsert 进凭据库。
        let logged_in = Account::new(
            "0123456789abcdef0123456789abcdef",
            "AuroraPlayer",
            AccountCredentials::Microsoft(MicrosoftCredentials {
                refresh_token: "rotated-refresh".into(),
                minecraft_token: Some("MC-TOKEN".into()),
                minecraft_expires_at: Some(u64::MAX),
            }),
        );
        aurora
            .open_accounts()
            .unwrap()
            .upsert(logged_in.clone())
            .unwrap();

        let adopted = aurora.adopt_as_current(logged_in.clone()).unwrap();
        assert_eq!(adopted.uuid, logged_in.uuid);

        // 重开门面读盘：当前账户已换成刚登录的那个，离线号还在列表里但不再被选中。
        let restarted = aurora_at(tmp.path());
        let current = restarted.current_account().unwrap().unwrap();
        assert_eq!(current.uuid, logged_in.uuid);
        assert_eq!(current.name, "AuroraPlayer");
        assert_eq!(
            current.account_type,
            aurora_auth::AccountType::Microsoft,
            "接管当前账户的必须是凭据账户本身，而不是同名回退"
        );
        let offline_names: Vec<_> = restarted
            .offline_accounts()
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(offline_names, ["Steve"], "登录不该删掉已保存的离线账户");

        // 反向再切回离线号仍然成立，两个库的选择始终只有一处。
        restarted.set_current_account(&offline.uuid).unwrap();
        assert_eq!(
            aurora_at(tmp.path())
                .current_account()
                .unwrap()
                .unwrap()
                .uuid,
            offline.uuid
        );
    }

    /// 删掉最后一个凭据账户后，当前账户必须回落到还留着的离线号。
    ///
    /// 这是两个库共存的另一半（上一条守的是登录方向）：登录时离线选中项被清空，此后再把那个凭据
    /// 账户删掉，若不补一次回落，界面上就会出现「列表里明明还有账户，却没有一个是当前账户」，
    /// 玩家必须手动再点一次「设为当前」。把 remove_account 里那段回落删掉，本用例即挂。
    ///
    /// 同时守住反向边界：凭据库里还剩别的账户时，选中权必须留在凭据库，不许被离线号抢走。
    #[cfg(windows)]
    #[test]
    fn removing_the_last_credential_account_hands_the_selection_back_to_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let aurora = aurora_at(tmp.path());
        let offline = aurora.add_offline_account("Alex", None).unwrap();

        let credential_account = |uuid: &str, name: &str| {
            let account = Account::new(
                uuid,
                name,
                AccountCredentials::Microsoft(MicrosoftCredentials {
                    refresh_token: "rotated-refresh".into(),
                    minecraft_token: Some("MC-TOKEN".into()),
                    minecraft_expires_at: Some(u64::MAX),
                }),
            );
            aurora
                .open_accounts()
                .unwrap()
                .upsert(account.clone())
                .unwrap();
            aurora.adopt_as_current(account.clone()).unwrap();
            account
        };
        let first = credential_account("0123456789abcdef0123456789abcdef", "AuroraPlayer");
        let second = credential_account("fedcba9876543210fedcba9876543210", "AuroraBackup");

        // 凭据库里还有别的账户：删完当前项由凭据库自己回落，离线号不该被拉上来。
        aurora.remove_account(&second.uuid).unwrap();
        let after_first_removal = aurora.current_account().unwrap().unwrap();
        assert_eq!(after_first_removal.uuid, first.uuid);
        assert_eq!(
            after_first_removal.account_type,
            aurora_auth::AccountType::Microsoft
        );

        // 删掉最后一个凭据账户：账户列表里只剩离线号，它就必须接手当前账户。
        aurora.remove_account(&first.uuid).unwrap();
        let restarted = aurora_at(tmp.path());
        assert_eq!(restarted.accounts().unwrap().len(), 1);
        let current = restarted
            .current_account()
            .unwrap()
            .expect("删掉最后一个凭据账户后，剩下的离线账户必须接手当前账户");
        assert_eq!(current.uuid, offline.uuid);
        assert_eq!(current.name, "Alex");
        assert_eq!(current.account_type, aurora_auth::AccountType::Offline);
    }

    #[test]
    fn client_id_falls_back_to_builtin_default() {
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::config::AuroraConfig {
            msa_client_id: None,
            ..crate::config::AuroraConfig::default()
        };
        let aurora = Aurora::for_test(config, tmp.path().to_path_buf(), tmp.path().to_path_buf());
        // 未配置且无环境变量时回落到内置调试 client_id，保证正版登录开箱可用；删掉回落此断言即挂。
        if std::env::var(MSA_CLIENT_ID_ENV).is_err() {
            assert_eq!(aurora.msa_client_id().unwrap(), DEFAULT_MSA_CLIENT_ID);
        }
    }
}
