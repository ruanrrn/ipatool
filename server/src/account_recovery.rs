//! Per-account bounded recovery admission. Never holds the registry over I/O.
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

pub fn requires_recovery(result: &HashMap<String, Value>) -> bool {
    if result.get("_state").and_then(Value::as_str) == Some("success") {
        return false;
    }
    matches!(
        result.get("failureType").and_then(Value::as_str),
        Some("2034" | "2042")
    ) || matches!(
        result.get("failureType").and_then(Value::as_i64),
        Some(2034 | 2042)
    )
}

#[derive(Default)]
pub struct RecoveryGate {
    state: Mutex<Option<(Instant, Result<(), String>)>>,
}
impl RecoveryGate {
    pub async fn run<F, Fut>(&self, cooldown: Duration, work: F) -> Result<(), String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(), String>>,
    {
        let mut state = self.state.lock().await;
        if let Some((finished, result)) = &*state {
            if finished.elapsed() < cooldown {
                return result.clone();
            }
        }
        // A cancelled leader must not let queued callers create a login storm.
        *state = Some((Instant::now(), Err("自动重新认证中断，请稍后重试".into())));
        let result = work().await;
        *state = Some((Instant::now(), result.clone()));
        result
    }
}

use super::{
    apple_auth_failure_details, canonical_purchase_email, load_saved_password,
    persist_account_session, save_pending_mfa, AccountStore, AppState, ACCOUNTS,
};
use actix_web::web;
async fn bounded_request<F, Fut, R, RFut>(mut operation: F, recover: R) -> StoreResult
where
    F: FnMut() -> Fut,
    Fut: Future<Output = StoreResult>,
    R: FnOnce() -> RFut,
    RFut: Future<Output = Result<(), String>>,
{
    let first = operation().await?;
    if !requires_recovery(&first) {
        return Ok(first);
    }
    recover().await?;
    operation().await
}

type StoreResult = Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>;
static GATES: std::sync::LazyLock<Mutex<HashMap<String, Arc<RecoveryGate>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
pub struct RecoveryStore {
    pub account_email: String,
    token: String,
    data: web::Data<AppState>,
}
impl RecoveryStore {
    pub fn new(token: &str, account: &AccountStore, data: &web::Data<AppState>) -> Self {
        Self {
            account_email: account.account_email.clone(),
            token: token.into(),
            data: data.clone(),
        }
    }
    async fn snapshot(&self) -> Result<AccountStore, String> {
        ACCOUNTS
            .read()
            .await
            .get(&self.token)
            .cloned()
            .ok_or_else(|| "账号已删除或登录会话已替换".into())
    }
    async fn persist_current(&self, snapshot: &AccountStore) -> Result<(), String> {
        // Accounts -> DB is the existing delete/list ordering. No await while DB is held.
        let accounts = ACCOUNTS.read().await;
        let current = accounts
            .get(&self.token)
            .ok_or("账号已删除或登录会话已替换")?;
        // Never overwrite a newer authentication with an old in-flight request's auth_info.
        let store = if current
            .auth_info
            .as_ref()
            .and_then(|a| a.password_token.as_ref())
            != snapshot
                .auth_info
                .as_ref()
                .and_then(|a| a.password_token.as_ref())
        {
            current
        } else {
            snapshot
        };
        let db = self.data.db.lock().map_err(|_| "数据库锁错误")?;
        persist_account_session(&db, &self.token, store)
    }
    async fn recover(&self, stale: &AccountStore) -> Result<(), String> {
        let key = canonical_purchase_email(&self.account_email);
        let gate = { GATES.lock().await.entry(key).or_default().clone() };
        gate.run(Duration::from_secs(60), || async {
            let mut account = self.snapshot().await?;
            if account.auth_info.as_ref().and_then(|a| a.password_token.as_ref()) != stale.auth_info.as_ref().and_then(|a| a.password_token.as_ref()) { return Ok(()); }
            let password = {
                let db = self.data.db.lock().map_err(|_| "数据库锁错误")?;
                load_saved_password(&db, &account.account_email).ok_or("登录已过期且未保存可用密码，请重新登录")?
            };
            let result = tokio::time::timeout(Duration::from_secs(120), account.authenticate(&password, None))
                .await.map_err(|_| "自动重新认证超时，请稍后重试")?
                .map_err(|e| format!("自动重新认证失败: {}", e))?;
            if result.get("_state").and_then(Value::as_str) != Some("success") {
                let obj = result.into_iter().collect();
                let (_, message, needs_mfa) = apple_auth_failure_details(&obj, false);
                if needs_mfa {
                    save_pending_mfa(&account.account_email.clone(), &password, account).await;
                    return Err("自动重新认证需要验证码，请在账号登录处输入验证码完成登录（已保存验证会话）".into());
                }
                return Err(format!("自动重新认证失败: {}", message));
            }
            let mut accounts = ACCOUNTS.write().await;
            let current = accounts.get_mut(&self.token).ok_or("账号已删除或登录会话已替换")?;
            if current.auth_info.as_ref().and_then(|a| a.password_token.as_ref()) != stale.auth_info.as_ref().and_then(|a| a.password_token.as_ref()) { return Ok(()); }
            {
                let db = self.data.db.lock().map_err(|_| "数据库锁错误")?;
                persist_account_session(&db, &self.token, &account)?;
            }
            *current = account;
            drop(accounts);
            super::PURCHASE_CACHE.write().await.retain(|(email, _), _| email != &canonical_purchase_email(&self.account_email));
            Ok(())
        }).await
    }
    async fn request(&self, appid: &str, version: Option<&str>, license: bool) -> StoreResult {
        let stale = self.snapshot().await?;
        bounded_request(
            || async {
                let account = self.snapshot().await?;
                let result = if license {
                    account.ensure_license(appid, version).await
                } else {
                    account.download_product(appid, version).await
                };
                self.persist_current(&account).await?;
                result
            },
            || self.recover(&stale),
        )
        .await
    }
    pub async fn download_product(&self, appid: &str, version: Option<&str>) -> StoreResult {
        self.request(appid, version, false).await
    }
    pub async fn ensure_license(&self, appid: &str, version: Option<&str>) -> StoreResult {
        self.request(appid, version, true).await
    }
}
#[async_trait::async_trait]
impl ipa_webtool_services::ipa_handler::AppleAuthService for RecoveryStore {
    async fn download_product(
        &self,
        appid: &str,
        version: Option<&str>,
        _: &ipa_webtool_services::AuthInfo,
    ) -> StoreResult {
        self.download_product(appid, version).await
    }
    async fn ensure_license(
        &self,
        appid: &str,
        version: Option<&str>,
        _: &ipa_webtool_services::AuthInfo,
    ) -> StoreResult {
        self.ensure_license(appid, version).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[tokio::test]
    async fn retries_exact_session_failure_once_without_recursion() {
        let operations = AtomicUsize::new(0);
        let recoveries = AtomicUsize::new(0);
        let result = bounded_request(
            || async {
                operations.fetch_add(1, Ordering::SeqCst);
                Ok(HashMap::from([("failureType".into(), Value::from("2042"))]))
            },
            || async {
                recoveries.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap();
        assert_eq!(operations.load(Ordering::SeqCst), 2);
        assert_eq!(recoveries.load(Ordering::SeqCst), 1);
        assert!(requires_recovery(&result));
    }
    #[tokio::test]
    async fn ordinary_failures_and_transport_errors_never_reauthenticate() {
        for code in ["2035", "12042", "license not found"] {
            let operations = AtomicUsize::new(0);
            bounded_request(
                || async {
                    operations.fetch_add(1, Ordering::SeqCst);
                    Ok(HashMap::from([("failureType".into(), Value::from(code))]))
                },
                || async { panic!("ordinary failure") },
            )
            .await
            .unwrap();
            assert_eq!(operations.load(Ordering::SeqCst), 1);
        }
        assert!(bounded_request(
            || async { Err("transport".into()) },
            || async { panic!("transport") }
        )
        .await
        .is_err());
        let operations = AtomicUsize::new(0);
        let error = bounded_request(
            || async {
                operations.fetch_add(1, Ordering::SeqCst);
                Ok(HashMap::from([("failureType".into(), Value::from("2034"))]))
            },
            || async { Err("MFA required".into()) },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "MFA required");
        assert_eq!(operations.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn cancelled_leader_is_cooled_down() {
        let gate = Arc::new(RecoveryGate::default());
        let started = Arc::new(tokio::sync::Notify::new());
        let task = {
            let gate = gate.clone();
            let started = started.clone();
            tokio::spawn(async move {
                gate.run(Duration::from_secs(60), || async {
                    started.notify_one();
                    std::future::pending::<Result<(), String>>().await
                })
                .await
            })
        };
        started.notified().await;
        task.abort();
        let _ = task.await;
        let result = gate
            .run(Duration::from_secs(60), || async {
                panic!("must not retry cancelled authentication")
            })
            .await;
        assert!(result.unwrap_err().contains("中断"));
    }
    #[tokio::test]
    async fn wrapper_persists_cookies_and_propagates_deleted_row_failure() {
        let (data, token, email) =
            super::super::tests::purchase_test_state("recovery-persist").await;
        let account = AccountStore::restore(
            &email,
            "ABCDEF123456".into(),
            ipa_webtool_services::AuthInfo {
                ds_person_id: Some("123".into()),
                password_token: Some("fake-test-password-token".into()),
                display_name: None,
                email: Some(email.clone()),
                region: Some("US".into()),
            },
            123,
        );
        account
            .store
            .import_session_cookies(r#"{"pod":"71","cookies":[]}"#)
            .unwrap();
        ACCOUNTS
            .write()
            .await
            .insert(token.clone(), account.clone());
        let wrapper = RecoveryStore::new(&token, &account, &data);
        wrapper.persist_current(&account).await.unwrap();
        let rows = data.db.lock().unwrap().get_account_sessions().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].2.contains("fake-test-password-token"));
        ACCOUNTS.write().await.remove(&token);
        super::super::restore_account_sessions(&data.db).await;
        let restored = wrapper.snapshot().await.unwrap();
        assert_eq!(restored.store.guid, "ABCDEF123456");
        let cookies: Value =
            serde_json::from_str(&restored.store.export_session_cookies().unwrap()).unwrap();
        assert_eq!(cookies["pod"], "71");
        data.db.lock().unwrap().delete_account(&token).unwrap();
        assert!(wrapper.persist_current(&account).await.is_err());
        ACCOUNTS.write().await.remove(&token);
    }
    #[tokio::test]
    async fn missing_credentials_failure_is_shared_without_rotating_token() {
        let (data, token, _) = super::super::tests::purchase_test_state("recovery-missing").await;
        let account = ACCOUNTS.read().await.get(&token).unwrap().clone();
        let wrapper = RecoveryStore::new(&token, &account, &data);
        let first = wrapper.recover(&account).await.unwrap_err();
        let second = wrapper.recover(&account).await.unwrap_err();
        assert_eq!(first, second);
        assert!(first.contains("未保存"));
        assert!(ACCOUNTS.read().await.contains_key(&token));
        ACCOUNTS.write().await.remove(&token);
    }
    #[test]
    fn only_exact_failure_codes_recover() {
        for code in [
            Value::from("2034"),
            Value::from("2042"),
            Value::from(2034),
            Value::from(2042),
        ] {
            assert!(requires_recovery(&HashMap::from([
                ("_state".into(), Value::from("failure")),
                ("failureType".into(), code)
            ])));
        }
        for code in ["12042", "2042 extra", "2043", "sign in"] {
            assert!(!requires_recovery(&HashMap::from([(
                "failureType".into(),
                Value::from(code)
            )])));
        }
        assert!(!requires_recovery(&HashMap::from([(
            "customerMessage".into(),
            Value::from("2042 sign in")
        )])));
        assert!(!requires_recovery(&HashMap::from([
            ("_state".into(), Value::from("success")),
            ("failureType".into(), Value::from("2042"))
        ])));
    }
    #[tokio::test]
    async fn concurrent_failures_share_one_attempt_and_cooldown_result() {
        let gate = Arc::new(RecoveryGate::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let results = futures::future::join_all((0..16).map(|_| {
            let gate = gate.clone();
            let calls = calls.clone();
            async move {
                gate.run(Duration::from_secs(60), || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    Err("MFA required".into())
                })
                .await
            }
        }))
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(results
            .iter()
            .all(|r| r.as_ref().err().map(String::as_str) == Some("MFA required")));
    }
    #[tokio::test]
    async fn successful_recovery_is_shared_and_expiry_allows_new_attempt() {
        let gate = RecoveryGate::default();
        let calls = AtomicUsize::new(0);
        for _ in 0..2 {
            gate.run(Duration::from_secs(60), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await
            .unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        gate.run(Duration::ZERO, || async {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
