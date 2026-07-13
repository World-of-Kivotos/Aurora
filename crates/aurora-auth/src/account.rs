//! 账户模型与多账户管理。
//!
//! 一个 [`Account`] = 稳定 uuid + 名称 + 登录类型 + 令牌引用（[`AccountCredentials`]）。
//! [`AccountManager`] 在 [`CredentialStore`] 之上做增删改查与“当前账户”切换，整份
//! [`AccountDatabase`] 序列化为 JSON 后交给 store 加密落盘。

use serde::{Deserialize, Serialize};

use crate::credential::CredentialStore;
use crate::error::{AuthError, Result};

/// 登录方式（分派状态机的判据）。统一通行证（Nide8）依赖商业服务，不在此列。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    /// 微软正版。
    Microsoft,
    /// 离线（本地生成 UUID）。
    Offline,
    /// Authlib-Injector（Yggdrasil 第三方验证）。
    AuthlibInjector,
}

/// 游戏内档案：`id` 为 32 位无连字符 UUID（与 Mojang profile、Yggdrasil 一致），`name` 为角色名。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameProfile {
    pub id: String,
    pub name: String,
}

/// 微软账户随账户存储的令牌引用。
///
/// `refresh_token` 是唯一的持久密钥（每次刷新轮换回写）；`minecraft_token` 是短期缓存，
/// 过期后由 `refresh_token` 重走令牌链换取，避免每次启动都重新握手。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrosoftCredentials {
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_token: Option<String>,
    /// Minecraft 访问令牌到期的 Unix 秒（用于判断是否需要刷新）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_expires_at: Option<u64>,
}

impl MicrosoftCredentials {
    /// 在给定的 Unix 时刻，缓存的 Minecraft 令牌是否仍然可用。
    ///
    /// 预留 60 秒安全边际：临近到期即视为失效，避免拿着马上过期的令牌去启动。
    pub fn minecraft_token_valid_at(&self, unix_now: u64) -> bool {
        const SKEW_SECS: u64 = 60;
        match (&self.minecraft_token, self.minecraft_expires_at) {
            (Some(_), Some(exp)) => exp > unix_now.saturating_add(SKEW_SECS),
            _ => false,
        }
    }
}

/// Authlib-Injector 账户随账户存储的令牌引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YggdrasilCredentials {
    /// 认证服务器 API 根地址（以 `/` 结尾），供刷新/校验与 javaagent 拼装复用。
    pub api_root: String,
    pub access_token: String,
    pub client_token: String,
}

/// 按登录类型区分的令牌引用。离线账户无任何令牌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountCredentials {
    Microsoft(MicrosoftCredentials),
    Offline,
    AuthlibInjector(YggdrasilCredentials),
}

impl AccountCredentials {
    /// 该令牌引用对应的登录类型。
    pub fn account_type(&self) -> AccountType {
        match self {
            AccountCredentials::Microsoft(_) => AccountType::Microsoft,
            AccountCredentials::Offline => AccountType::Offline,
            AccountCredentials::AuthlibInjector(_) => AccountType::AuthlibInjector,
        }
    }
}

/// 单个账户记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// 稳定标识（增删改查、当前账户引用均以此为键）。32 位无连字符 UUID。
    pub uuid: String,
    pub name: String,
    pub account_type: AccountType,
    pub credentials: AccountCredentials,
}

impl Account {
    /// 构造账户；`account_type` 由 `credentials` 推导，保证二者一致。
    pub fn new(uuid: impl Into<String>, name: impl Into<String>, credentials: AccountCredentials) -> Self {
        Self {
            uuid: uuid.into(),
            name: name.into(),
            account_type: credentials.account_type(),
            credentials,
        }
    }
}

/// 可序列化的账户缓存根：账户列表 + 当前账户 uuid。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDatabase {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
}

/// 多账户管理：在 [`CredentialStore`] 之上维护账户列表与当前账户，任何变更立即持久化。
pub struct AccountManager<S: CredentialStore> {
    store: S,
    db: AccountDatabase,
}

impl<S: CredentialStore> AccountManager<S> {
    /// 从 store 载入既有账户缓存；store 为空（首次运行）则以空库开始。
    pub fn load(store: S) -> Result<Self> {
        let db = match store.load()? {
            Some(bytes) => {
                serde_json::from_slice(&bytes).map_err(AuthError::CredentialDeserialize)?
            }
            None => AccountDatabase::default(),
        };
        Ok(Self { store, db })
    }

    /// 全部账户（只读）。
    pub fn accounts(&self) -> &[Account] {
        &self.db.accounts
    }

    /// 当前账户（若已选择且仍存在）。
    pub fn current(&self) -> Option<&Account> {
        let id = self.db.current.as_deref()?;
        self.find(id)
    }

    /// 按 uuid 查找账户。
    pub fn find(&self, uuid: &str) -> Option<&Account> {
        self.db.accounts.iter().find(|a| a.uuid == uuid)
    }

    /// 新增或更新账户（按 uuid 判重：已存在则整条替换，用于重新登录后回写新令牌）。
    ///
    /// 首个被加入的账户自动成为当前账户；已有当前账户时不改变选择。
    pub fn upsert(&mut self, account: Account) -> Result<()> {
        match self.db.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
            Some(existing) => *existing = account,
            None => {
                if self.db.current.is_none() {
                    self.db.current = Some(account.uuid.clone());
                }
                self.db.accounts.push(account);
            }
        }
        self.persist()
    }

    /// 删除账户；若删除的是当前账户，则回落到剩余的第一个（无剩余则清空当前）。
    pub fn remove(&mut self, uuid: &str) -> Result<()> {
        let before = self.db.accounts.len();
        self.db.accounts.retain(|a| a.uuid != uuid);
        if self.db.accounts.len() == before {
            return Err(AuthError::AccountNotFound(uuid.to_owned()));
        }
        if self.db.current.as_deref() == Some(uuid) {
            self.db.current = self.db.accounts.first().map(|a| a.uuid.clone());
        }
        self.persist()
    }

    /// 切换当前账户；目标不存在则报错。
    pub fn set_current(&mut self, uuid: &str) -> Result<()> {
        if self.find(uuid).is_none() {
            return Err(AuthError::AccountNotFound(uuid.to_owned()));
        }
        self.db.current = Some(uuid.to_owned());
        self.persist()
    }

    fn persist(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.db).map_err(AuthError::CredentialSerialize)?;
        self.store.save(&bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential::testing::InMemoryStore;

    fn offline(uuid: &str, name: &str) -> Account {
        Account::new(uuid, name, AccountCredentials::Offline)
    }

    fn microsoft(uuid: &str, name: &str, refresh: &str) -> Account {
        Account::new(
            uuid,
            name,
            AccountCredentials::Microsoft(MicrosoftCredentials {
                refresh_token: refresh.into(),
                minecraft_token: None,
                minecraft_expires_at: None,
            }),
        )
    }

    #[test]
    fn account_type_is_derived_from_credentials() {
        assert_eq!(offline("u", "n").account_type, AccountType::Offline);
        assert_eq!(
            microsoft("u", "n", "r").account_type,
            AccountType::Microsoft
        );
    }

    #[test]
    fn first_account_becomes_current_second_does_not_change_it() {
        let mut mgr = AccountManager::load(InMemoryStore::new()).unwrap();
        mgr.upsert(offline("aaa", "Alpha")).unwrap();
        assert_eq!(mgr.current().unwrap().uuid, "aaa");

        mgr.upsert(offline("bbb", "Beta")).unwrap();
        assert_eq!(mgr.accounts().len(), 2);
        // 当前账户仍是第一个。
        assert_eq!(mgr.current().unwrap().uuid, "aaa");
    }

    #[test]
    fn upsert_same_uuid_replaces_in_place() {
        let mut mgr = AccountManager::load(InMemoryStore::new()).unwrap();
        mgr.upsert(microsoft("uuid1", "OldName", "old-refresh"))
            .unwrap();
        mgr.upsert(microsoft("uuid1", "NewName", "new-refresh"))
            .unwrap();

        assert_eq!(mgr.accounts().len(), 1);
        let acc = mgr.find("uuid1").unwrap();
        assert_eq!(acc.name, "NewName");
        match &acc.credentials {
            AccountCredentials::Microsoft(c) => assert_eq!(c.refresh_token, "new-refresh"),
            other => panic!("期望 Microsoft 凭据，得到 {other:?}"),
        }
    }

    #[test]
    fn set_current_switches_and_unknown_errors() {
        let mut mgr = AccountManager::load(InMemoryStore::new()).unwrap();
        mgr.upsert(offline("aaa", "Alpha")).unwrap();
        mgr.upsert(offline("bbb", "Beta")).unwrap();

        mgr.set_current("bbb").unwrap();
        assert_eq!(mgr.current().unwrap().uuid, "bbb");

        let err = mgr.set_current("zzz").unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(id) if id == "zzz"));
    }

    #[test]
    fn remove_current_falls_back_to_first_remaining() {
        let mut mgr = AccountManager::load(InMemoryStore::new()).unwrap();
        mgr.upsert(offline("aaa", "Alpha")).unwrap();
        mgr.upsert(offline("bbb", "Beta")).unwrap();
        mgr.set_current("bbb").unwrap();

        mgr.remove("bbb").unwrap();
        assert_eq!(mgr.accounts().len(), 1);
        // 当前账户回落到剩余的第一个。
        assert_eq!(mgr.current().unwrap().uuid, "aaa");

        // 删除不存在的账户报错。
        let err = mgr.remove("bbb").unwrap_err();
        assert!(matches!(err, AuthError::AccountNotFound(_)));
    }

    #[test]
    fn removing_last_account_clears_current() {
        let mut mgr = AccountManager::load(InMemoryStore::new()).unwrap();
        mgr.upsert(offline("aaa", "Alpha")).unwrap();
        mgr.remove("aaa").unwrap();
        assert!(mgr.current().is_none());
        assert!(mgr.accounts().is_empty());
    }

    #[test]
    fn state_survives_reload_through_store() {
        // 共享同一份底层字节的两个 store 句柄，模拟“写入后重启进程重载”。
        let store = InMemoryStore::new();
        {
            let mut mgr = AccountManager::load(store.clone()).unwrap();
            mgr.upsert(microsoft("uuid1", "Alpha", "refresh-a")).unwrap();
            mgr.upsert(offline("uuid2", "Beta")).unwrap();
            mgr.set_current("uuid2").unwrap();
        }

        // 新 manager 从同一份持久化字节重建，应完全还原账户列表与当前选择。
        let reloaded = AccountManager::load(store).unwrap();
        assert_eq!(reloaded.current().unwrap().uuid, "uuid2");
        assert_eq!(reloaded.accounts().len(), 2);
        assert_eq!(reloaded.accounts()[0].name, "Alpha");
        assert_eq!(reloaded.accounts()[1].account_type, AccountType::Offline);
        match &reloaded.find("uuid1").unwrap().credentials {
            AccountCredentials::Microsoft(c) => assert_eq!(c.refresh_token, "refresh-a"),
            other => panic!("期望 Microsoft 凭据，得到 {other:?}"),
        }
    }

    #[test]
    fn microsoft_token_validity_respects_skew() {
        let creds = MicrosoftCredentials {
            refresh_token: "r".into(),
            minecraft_token: Some("mc".into()),
            minecraft_expires_at: Some(1_000),
        };
        // 距到期 >60s 才算有效。
        assert!(creds.minecraft_token_valid_at(900));
        assert!(!creds.minecraft_token_valid_at(950));
        assert!(!creds.minecraft_token_valid_at(1_000));

        // 无缓存令牌一律无效。
        let empty = MicrosoftCredentials {
            refresh_token: "r".into(),
            minecraft_token: None,
            minecraft_expires_at: None,
        };
        assert!(!empty.minecraft_token_valid_at(0));
    }
}
