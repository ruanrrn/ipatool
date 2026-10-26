use account_recovery::RecoveryStore;
mod account_recovery;
use actix_files as fs;
use actix_multipart::Multipart;
use actix_web::{
    body::{EitherBody, MessageBody},
    dev::{ServiceRequest, ServiceResponse},
    error::{ErrorInternalServerError, ErrorNotFound, ErrorUnauthorized},
    http::header::{ContentDisposition, DispositionParam, DispositionType},
    middleware::{from_fn, Next},
    web, App, Error, FromRequest, HttpRequest, HttpResponse, HttpServer, Responder,
};
use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use futures_util::{
    future::{ready, Ready},
    stream, StreamExt, TryStreamExt,
};
use ipa_webtool_services::models::{
    build_session_cookie, clear_session_cookie, is_pending_mfa_expired, normalize_mfa_code,
    normalize_region_code, session_expires_at, should_use_secure_cookie, unauthorized_response,
    AdminLoginRequest, ApiResponse, AppMetaQuery, AppleLoginRequest, AuthUserPayload,
    ChangePasswordRequest, ClaimRequest, ConfirmPurchaseRequest, DeliveryDecision,
    DownloadArtifact, DownloadRecordView, DownloadUrlQuery, ExistingDownloadResponse,
    IpaArtifactView, JobIdQuery, ManifestQuery, PendingMfaSession, PurchaseCacheEntry,
    PurchaseStatusBatchRequest, PurchaseStatusQuery, StartDownloadDirectRequest, VersionQuery,
    ADMIN_SESSION_COOKIE, PENDING_MFA_TTL_MINUTES,
};
use ipa_webtool_services::DownloadRecord;
use ipa_webtool_services::{
    canonical_ipa_filename, download_ipa_with_account, generate_plist, get_license_error_message,
    inspect_ipa_path, read_bundle_identifier_from_ipa, sanitize_ipa_filename, AccountStore,
    BatchItem, Database, DownloadManager, DownloadParams, InstallQuery, IpaInspection, JobEndEvent,
    JobEvent, JobLogEvent, JobProgressEvent, JobProgressPayload, JobState, JobStore,
    NewSubscription,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct AuthenticatedAdmin {
    pub username: String,
    pub is_default: bool,
    #[allow(dead_code)]
    pub session_token: String,
}

/// Per-IP and per-username login rate limiter.
/// After LOGIN_RATE_LIMIT_MAX_FAILURES failed attempts within LOGIN_RATE_LIMIT_WINDOW,
/// further attempts are rejected until the window elapses.
struct LoginRateLimiter {
    /// (attempt_count, last_attempt_time)
    ip_attempts: HashMap<String, (u32, Instant)>,
    user_attempts: HashMap<String, (u32, Instant)>,
}

impl LoginRateLimiter {
    fn new() -> Self {
        Self {
            ip_attempts: HashMap::new(),
            user_attempts: HashMap::new(),
        }
    }

    fn cleanup_expired(&mut self, window: Duration) {
        let now = Instant::now();
        self.ip_attempts
            .retain(|_, (_, last)| now.duration_since(*last) < window);
        self.user_attempts
            .retain(|_, (_, last)| now.duration_since(*last) < window);
    }

    fn check_and_record(&mut self, ip: &str, username: &str) -> Result<(), String> {
        const MAX_FAILURES: u32 = 5;
        let window = Duration::from_secs(900); // 15 minutes
        let lockout = Duration::from_secs(900);

        let now = Instant::now();
        self.cleanup_expired(window);

        // Check IP-based rate limit
        if let Some((count, last)) = self.ip_attempts.get(ip) {
            if *count >= MAX_FAILURES {
                let elapsed = now.duration_since(*last);
                if elapsed < lockout {
                    let remaining = lockout.as_secs() - elapsed.as_secs();
                    return Err(format!("请求过于频繁，请 {} 秒后再试", remaining));
                }
                // Lockout expired, remove entry (will be re-added on next failure)
            }
        }

        // Check username-based rate limit (prevents distributed attacks)
        if let Some((count, last)) = self.user_attempts.get(username) {
            if *count >= MAX_FAILURES {
                let elapsed = now.duration_since(*last);
                if elapsed < lockout {
                    let remaining = lockout.as_secs() - elapsed.as_secs();
                    return Err(format!("该账号登录尝试过于频繁，请 {} 秒后再试", remaining));
                }
            }
        }

        Ok(())
    }

    fn record_failure(&mut self, ip: &str, username: &str) {
        let now = Instant::now();
        let entry = self.ip_attempts.entry(ip.to_string()).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;

        let entry = self
            .user_attempts
            .entry(username.to_string())
            .or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    fn clear_success(&mut self, ip: &str, username: &str) {
        self.ip_attempts.remove(ip);
        self.user_attempts.remove(username);
    }
}

// 应用状态
struct AppState {
    db: Arc<Mutex<Database>>,
    download_manager: Arc<DownloadManager>,
    job_store: JobStore,
    downloads_dir: PathBuf,
    login_rate_limiter: Arc<Mutex<LoginRateLimiter>>,
}

// 模拟的账号存储（生产环境应该使用数据库）
// Replaced lazy_static with std::sync::LazyLock
static ACCOUNTS: std::sync::LazyLock<RwLock<HashMap<String, AccountStore>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
static PENDING_MFA: std::sync::LazyLock<RwLock<HashMap<String, PendingMfaSession>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

// Replaced lazy_static with std::sync::LazyLock
static PURCHASE_CACHE: std::sync::LazyLock<
    RwLock<std::collections::HashMap<(String, String), PurchaseCacheEntry>>,
> = std::sync::LazyLock::new(|| RwLock::new(std::collections::HashMap::new()));
static COMMUNITY_DELISTED_INDEX_CACHE: std::sync::LazyLock<
    RwLock<Option<(Instant, CommunityDelistedLiteIndex)>>,
> = std::sync::LazyLock::new(|| RwLock::new(None));
static COMMUNITY_DELISTED_APP_CACHE: std::sync::LazyLock<
    RwLock<HashMap<String, (Instant, CommunityDelistedAppDetail)>>,
> = std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
static STORE_STATUS_CACHE: std::sync::LazyLock<RwLock<HashMap<String, (Instant, bool)>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));
type LocalDelistedCandidatesCache = RwLock<Option<(Instant, Vec<LocalDelistedCandidate>)>>;
static LOCAL_DELISTED_CANDIDATES_CACHE: std::sync::LazyLock<LocalDelistedCandidatesCache> =
    std::sync::LazyLock::new(|| RwLock::new(None));

const PURCHASE_CACHE_TTL_SECS: u64 = 300; // 5 minutes
const COMMUNITY_ARCHIVE_CACHE_TTL_SECS: u64 = 300; // 5 minutes
const STORE_STATUS_CACHE_TTL_SECS: u64 = 3600; // 1 hour
const LOCAL_DELISTED_CANDIDATES_CACHE_TTL_SECS: u64 = 60; // 1 minute
const STORE_STATUS_CHECK_CONCURRENCY: usize = 8;

fn canonical_purchase_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn purchase_requires_login(error: &str) -> bool {
    let lowered = error.to_lowercase();
    lowered.contains("2042")
        || lowered.contains("sign in to the itunes store")
        || error.contains("登录到 iTunes Store")
}

fn purchase_required(error: &str) -> bool {
    let lowered = error.to_lowercase();
    if lowered.contains("storefront")
        || lowered.contains("region")
        || lowered.contains("sign in")
        || lowered.contains("2042")
        || error.contains("地区")
        || error.contains("登录")
    {
        return false;
    }
    lowered.contains("license not found")
        || lowered.contains("not purchased")
        || lowered.contains("not owned")
        || error.contains("未购买")
        || error.contains("未领取")
}

fn purchase_failure_error(error: &str) -> Option<String> {
    (!purchase_required(error)).then(|| error.to_string())
}

fn purchase_database_error(error: impl std::fmt::Display) -> HttpResponse {
    HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
        "购买记录数据库错误: {}",
        error
    )))
}

fn persist_purchase(data: &AppState, email: &str, appid: &str, source: &str) -> Result<(), String> {
    let db = data
        .db
        .lock()
        .map_err(|error| format!("数据库锁错误: {}", error))?;
    db.record_purchase(&canonical_purchase_email(email), appid, source)
        .map_err(|error| error.to_string())
}

fn evict_expired_purchase_cache_locked(
    cache: &mut std::collections::HashMap<(String, String), PurchaseCacheEntry>,
) {
    let ttl = Duration::from_secs(PURCHASE_CACHE_TTL_SECS);
    cache.retain(|_, entry| entry.cached_at.elapsed() < ttl);
}

fn build_http_client() -> Client {
    thread_local! {
        // Reuse one connection pool per actix worker thread instead of building a client (and
        // TLS context) per call. Per-thread keeps pooled connections on the runtime using them.
        static HTTP_CLIENT: Client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent("ipatool-server")
            .build()
            .unwrap_or_else(|error| {
                log::error!("failed to build HTTP client with timeouts: {}", error);
                Client::new()
            });
    }
    HTTP_CLIENT.with(Client::clone)
}

fn github_contents_api_path(file_path: &str) -> String {
    file_path
        .split('/')
        .map(urlencoding::encode)
        .collect::<Vec<_>>()
        .join("/")
}

/// App Store app ids ("adamId") are numeric. Validating them before use keeps request input out
/// of archive file paths (`data/archive/{id}.json`) and upstream URLs.
fn is_valid_app_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
}

fn invalid_app_id_response() -> HttpResponse {
    HttpResponse::BadRequest().json(ApiResponse::<()>::error(
        "app_id 必须是数字 Apple ID".to_string(),
    ))
}

/// Normalise a storefront country code (2–3 ASCII letters), falling back to `default`.
fn store_region_or(value: Option<&str>, default: &str) -> String {
    value
        .map(str::trim)
        .filter(|value| {
            (2..=3).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_alphabetic())
        })
        .map(|value| value.to_ascii_uppercase())
        .unwrap_or_else(|| default.to_string())
}

/// Mask a secret for display without slicing inside a multi-byte UTF-8 character.
fn mask_secret(secret: &str, visible_prefix: usize, visible_suffix: usize) -> String {
    let chars = secret.chars().collect::<Vec<_>>();
    if chars.len() <= visible_prefix + visible_suffix + 4 {
        return "*".repeat(chars.len().clamp(1, 4));
    }
    let prefix = chars[..visible_prefix].iter().collect::<String>();
    let suffix = chars[chars.len() - visible_suffix..]
        .iter()
        .collect::<String>();
    format!("{}****{}", prefix, suffix)
}

/// 持久化到 accounts.auth_session 的 Apple 登录态（整体 AES-GCM 加密）。
#[derive(Serialize, Deserialize)]
struct PersistedAccountSession {
    guid: String,
    auth_info: ipa_webtool_services::AuthInfo,
    #[serde(default)]
    authenticated_at: i64,
    #[serde(default)]
    session_cookies: String,
}

#[derive(Serialize, Deserialize)]
struct EncryptedAccountSession {
    ct: String,
    iv: String,
    tag: String,
}

/// 将账号当前登录态加密写入 DB，保证服务重启后无需重新登录。
fn persist_account_session(db: &Database, token: &str, store: &AccountStore) -> Result<(), String> {
    let auth_info = store.auth_info.clone().ok_or("账号尚未认证")?;
    let payload = serde_json::to_string(&PersistedAccountSession {
        guid: store.store.guid.clone(),
        auth_info,
        authenticated_at: store.authenticated_at,
        session_cookies: store.store.export_session_cookies()?,
    })
    .map_err(|e| e.to_string())?;
    let enc_key = ipa_webtool_services::crypto::ensure_encryption_key(db)?;
    let (ct, iv, tag) = ipa_webtool_services::crypto::encrypt(&payload, &enc_key)?;
    let blob = serde_json::to_string(&EncryptedAccountSession { ct, iv, tag })
        .map_err(|e| e.to_string())?;
    // update_account_session currently accepts zero affected rows; reject stale/deleted tokens.
    if db
        .get_account_by_token(token)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        return Err("账号已删除或登录会话已替换".into());
    }
    db.update_account_session(token, &blob)
        .map_err(|e| e.to_string())
}

/// 启动时从 DB 恢复已登录的 Apple 账号到内存。
async fn restore_account_sessions(db_arc: &Arc<Mutex<Database>>) {
    let restored: Vec<(String, AccountStore)> = {
        let db = match db_arc.lock() {
            Ok(db) => db,
            Err(_) => return,
        };
        let rows = match db.get_account_sessions() {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("[account-session] load failed: {}", e);
                return;
            }
        };
        if rows.is_empty() {
            return;
        }
        let enc_key = match ipa_webtool_services::crypto::ensure_encryption_key(&db) {
            Ok(k) => k,
            Err(e) => {
                log::error!("[account-session] encryption key error: {}", e);
                return;
            }
        };
        rows.into_iter()
            .filter_map(|(token, email, blob)| {
                let session = serde_json::from_str::<EncryptedAccountSession>(&blob)
                    .map_err(|e| e.to_string())
                    .and_then(|enc| {
                        ipa_webtool_services::crypto::decrypt(&enc.ct, &enc.iv, &enc.tag, &enc_key)
                    })
                    .and_then(|plain| {
                        serde_json::from_str::<PersistedAccountSession>(&plain)
                            .map_err(|e| e.to_string())
                    });
                match session {
                    Ok(session) => {
                        let store = AccountStore::restore(
                            &email,
                            session.guid,
                            session.auth_info,
                            session.authenticated_at,
                        );
                        match store.store.import_session_cookies(&session.session_cookies) {
                            Ok(()) => Some((token, store)),
                            Err(error) => {
                                log::warn!(
                                    "[account-session] invalid cookie state for {}: {}",
                                    email,
                                    error
                                );
                                None
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("[account-session] skip {}: {}", email, e);
                        None
                    }
                }
            })
            .collect()
    };

    let count = restored.len();
    let mut accounts = ACCOUNTS.write().await;
    for (token, store) in restored {
        accounts.insert(token, store);
    }
    log::info!("[account-session] restored {} Apple account(s)", count);
}

/// 解密已保存的 Apple ID 密码。
fn load_saved_password(db: &Database, email: &str) -> Option<String> {
    let cred = db.get_credentials(email).ok().flatten()?;
    let enc_key = ipa_webtool_services::crypto::ensure_encryption_key(db).ok()?;
    ipa_webtool_services::crypto::decrypt(
        &cred.password_encrypted,
        &cred.iv,
        &cred.auth_tag,
        &enc_key,
    )
    .ok()
}

fn hash_password(password: &str) -> String {
    Database::hash_password(password)
}

fn is_legacy_password_hash(hash: &str) -> bool {
    hash.len() != 60
}

fn verify_password(password: &str, hash: &str) -> bool {
    if !is_legacy_password_hash(hash) {
        bcrypt::verify(password, hash).unwrap_or(false)
    } else {
        let old_hash = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(password.as_bytes());
            hex::encode(hasher.finalize())
        };
        old_hash.len() == hash.len() && openssl::memcmp::eq(old_hash.as_bytes(), hash.as_bytes())
    }
}

/// bcrypt hash checked when the username does not exist, so response timing does not reveal
/// which admin usernames are valid.
static DUMMY_PASSWORD_HASH: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| hash_password("ipatool-timing-equaliser"));

/// Returns the form of `password` that matches `hash`: as typed, or whitespace-trimmed (older
/// clients and the previous server trimmed before verifying). Runs bcrypt: call off the async
/// worker.
fn matching_login_password(password: &str, hash: &str) -> Option<String> {
    if verify_password(password, hash) {
        return Some(password.to_string());
    }
    let trimmed = password.trim();
    (trimmed != password && !trimmed.is_empty() && verify_password(trimmed, hash))
        .then(|| trimmed.to_string())
}

fn request_uses_https(req: &HttpRequest) -> bool {
    should_use_secure_cookie(req.connection_info().scheme().eq_ignore_ascii_case("https"))
}

const MIN_ADMIN_PASSWORD_CHARS: usize = 8;

/// Whether `username` still has `expected_hash`. Call with the DB lock held, after verifying a
/// password against a hash snapshot taken before the lock was released for bcrypt.
fn admin_password_hash_unchanged(db: &Database, username: &str, expected_hash: &str) -> bool {
    db.get_admin_user(username)
        .ok()
        .flatten()
        .is_some_and(|user| user.password_hash == expected_hash)
}

fn resolved_account_region(result: &HashMap<String, Value>, fallback: Option<String>) -> String {
    result
        .get("region")
        .and_then(|value| value.as_str())
        .and_then(normalize_region_code)
        .or_else(|| fallback.and_then(|value| normalize_region_code(&value)))
        .unwrap_or_else(|| "US".to_string())
}

async fn save_pending_mfa(email: &str, password: &str, account_store: AccountStore) {
    let mut pending = PENDING_MFA.write().await;
    pending.insert(
        email.to_string(),
        PendingMfaSession {
            account_store,
            password_hash: hash_password(password),
            created_at: Utc::now(),
        },
    );
}

async fn clear_pending_mfa(email: &str) {
    let mut pending = PENDING_MFA.write().await;
    pending.remove(email);
}

async fn take_pending_mfa(email: &str, password: &str) -> Result<AccountStore, String> {
    let pending_session = {
        let mut pending = PENDING_MFA.write().await;
        pending.remove(email)
    };

    let pending_session = pending_session.ok_or_else(|| {
        "验证码会话不存在或已丢失，请重新输入账号密码并再次登录以获取新的验证码".to_string()
    })?;

    if is_pending_mfa_expired(pending_session.created_at) {
        return Err(format!(
            "验证码会话已超过 {} 分钟，请重新登录以获取新的验证码",
            PENDING_MFA_TTL_MINUTES
        ));
    }

    if !verify_password(password, &pending_session.password_hash) {
        return Err("登录密码已变更，请重新输入账号密码并再次登录以重新发起验证".to_string());
    }

    Ok(pending_session.account_store)
}

fn apple_auth_failure_details(
    result: &serde_json::Map<String, Value>,
    has_mfa: bool,
) -> (String, String, bool) {
    let failure_type = result
        .get("failureType")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let customer_message = result
        .get("customerMessage")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let normalized_message = customer_message.to_lowercase();
    let bad_login_without_mfa = !has_mfa
        && failure_type.is_empty()
        && matches!(
            customer_message.as_str(),
            "MZFinance.BadLogin.Configurator_message" | "MZFinance.BadLogin.Configurator.message"
        );

    let explicit_mfa_message = normalized_message.contains("verification code")
        || normalized_message.contains("two-factor")
        || normalized_message.contains("two step")
        || normalized_message.contains("two-step")
        || normalized_message.contains("2fa")
        || normalized_message.contains("mfa");

    let explicit_mfa_failure = failure_type.contains("verificationCode")
        || failure_type.contains("authCode")
        || failure_type == "-22938"
        || failure_type == "-20101";

    let needs_mfa = bad_login_without_mfa || explicit_mfa_message || explicit_mfa_failure;

    let user_facing_msg = if bad_login_without_mfa {
        "此账号需要二次验证，请在验证码输入框输入 6 位验证码后再次点击登录".to_string()
    } else if explicit_mfa_message || explicit_mfa_failure {
        if has_mfa {
            "验证码无效、已过期，或当前验证会话已失效，请检查后重试".to_string()
        } else {
            customer_message.clone()
        }
    } else {
        match customer_message.as_str() {
            "MZFinance.BadLogin.Configurator_message"
            | "MZFinance.BadLogin.Configurator.message" => {
                "账号或密码错误，请检查后重试".to_string()
            }
            m if m.starts_with("MZFinance.BadLogin") => "账号或密码错误，请检查后重试".to_string(),
            m if m.contains("account.locked") || m.contains("account disabled") => {
                "账号已被锁定或停用".to_string()
            }
            m if m.contains("rate.limit") || m.contains("too many") => {
                "登录尝试过于频繁，请稍后再试".to_string()
            }
            _ if !customer_message.is_empty() => customer_message.clone(),
            _ if !failure_type.is_empty() => failure_type.clone(),
            _ => "登录失败，Apple 未返回具体错误信息".to_string(),
        }
    };

    (failure_type, user_facing_msg, needs_mfa)
}

fn resolve_admin_session(app_state: &AppState, token: &str) -> Result<AuthenticatedAdmin, String> {
    log::debug!("[auth:resolve] token={}..", &token[..8.min(token.len())]);

    let db = app_state.db.lock().map_err(|e| {
        log::error!("[auth:resolve] db lock failed: {:?}", e);
        "认证服务暂时不可用".to_string()
    })?;

    let session = db
        .get_session(token)
        .map_err(|e| {
            log::error!("[auth:resolve] get_session failed: {}", e);
            format!("查询登录态失败: {}", e)
        })?
        .ok_or_else(|| {
            log::debug!(
                "[auth:resolve] no valid session for token={}..",
                &token[..8.min(token.len())]
            );
            "未登录或登录已过期".to_string()
        })?;

    let user = db
        .get_admin_user(&session.username)
        .map_err(|e| {
            log::error!(
                "[auth:resolve] get_admin_user failed for {}: {}",
                session.username,
                e
            );
            format!("查询管理员失败: {}", e)
        })?
        .ok_or_else(|| {
            let _ = db.delete_session(token);
            log::warn!(
                "[auth:resolve] admin user not found: {}, session deleted",
                session.username
            );
            "管理员账号不存在".to_string()
        })?;

    log::debug!("[auth:resolve] ok user={}", user.username);

    Ok(AuthenticatedAdmin {
        username: user.username,
        is_default: user.is_default,
        session_token: session.token,
    })
}

impl FromRequest for AuthenticatedAdmin {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let app_state = match req.app_data::<web::Data<AppState>>() {
            Some(data) => data.clone(),
            None => return ready(Err(ErrorUnauthorized("认证服务未初始化"))),
        };

        let session_cookie = match req.cookie(ADMIN_SESSION_COOKIE) {
            Some(cookie) => cookie,
            None => return ready(Err(ErrorUnauthorized("未登录或登录已过期"))),
        };

        ready(
            resolve_admin_session(app_state.get_ref(), session_cookie.value())
                .map_err(ErrorUnauthorized),
        )
    }
}

async fn security_headers<B>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error>
where
    B: MessageBody + 'static,
{
    let res = next.call(req).await?;
    let mut res = res.map_into_left_body();

    let headers = res.headers_mut();
    headers.insert(
        actix_web::http::header::X_CONTENT_TYPE_OPTIONS,
        actix_web::http::header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        actix_web::http::header::X_FRAME_OPTIONS,
        actix_web::http::header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        actix_web::http::header::X_XSS_PROTECTION,
        actix_web::http::header::HeaderValue::from_static("1; mode=block"),
    );
    headers.insert(
        actix_web::http::header::REFERRER_POLICY,
        actix_web::http::header::HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        actix_web::http::header::HeaderName::from_static("permissions-policy"),
        actix_web::http::header::HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=()",
        ),
    );

    Ok(res)
}

async fn require_auth<B>(
    req: ServiceRequest,
    next: Next<B>,
) -> Result<ServiceResponse<EitherBody<B>>, Error>
where
    B: MessageBody + 'static,
{
    let app_state = match req.app_data::<web::Data<AppState>>() {
        Some(data) => data.clone(),
        None => {
            return Ok(req
                .into_response(unauthorized_response())
                .map_into_right_body())
        }
    };

    let Some(session_cookie) = req.cookie(ADMIN_SESSION_COOKIE) else {
        return Ok(req
            .into_response(unauthorized_response())
            .map_into_right_body());
    };

    if let Err(error_message) = resolve_admin_session(app_state.get_ref(), session_cookie.value()) {
        log::warn!("[auth:middleware] session rejected: {}", error_message);
        return Ok(req
            .into_response(
                HttpResponse::Unauthorized().json(ApiResponse::<String>::error(error_message)),
            )
            .map_into_right_body());
    }

    Ok(next.call(req).await?.map_into_left_body())
}

// 健康检查 — 验证数据库连接、下载目录可写性
async fn health(data: web::Data<AppState>) -> impl Responder {
    let mut checks: Vec<serde_json::Value> = Vec::new();

    // Check DB connectivity
    let db_ok = match data.db.lock() {
        Ok(db) => db.get_admin_user("__health_check__").is_ok(),
        Err(_) => false,
    };
    checks.push(serde_json::json!({
        "component": "database",
        "status": if db_ok { "ok" } else { "unavailable" }
    }));

    // Check downloads directory
    let dir_ok = data.downloads_dir.exists()
        && std::fs::metadata(&data.downloads_dir)
            .map(|m| m.is_dir())
            .unwrap_or(false);
    let writable = std::fs::File::create(data.downloads_dir.join(".health_check"))
        .map(drop)
        .is_ok();
    let _ = std::fs::remove_file(data.downloads_dir.join(".health_check"));
    checks.push(serde_json::json!({
        "component": "downloads_dir",
        "status": if dir_ok && writable { "ok" } else { "degraded" }
    }));

    let all_ok = db_ok && dir_ok && writable;

    let body = serde_json::json!({
        "status": if all_ok { "ok" } else { "degraded" },
        "checks": checks
    });

    if all_ok {
        HttpResponse::Ok().json(body)
    } else {
        HttpResponse::ServiceUnavailable().json(body)
    }
}

fn extract_version_array(json: &Value) -> Vec<Value> {
    if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
        data.clone()
    } else if let Some(array) = json.as_array() {
        array.clone()
    } else {
        vec![]
    }
}

fn parse_json_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|v| i64::try_from(v).ok()))
            .or_else(|| number.as_f64().map(|v| v as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn extract_version_label(item: &Value) -> String {
    item.get("bundle_version")
        .or(item.get("version"))
        .or(item.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn extract_version_external_identifier(item: &Value) -> i64 {
    parse_json_i64(
        item.get("external_identifier")
            .or(item.get("externalVersionId"))
            .or(item.get("version_id"))
            .or(item.get("appVersionId"))
            .or(item.get("id")),
    )
    .unwrap_or(0)
}

fn extract_version_size(item: &Value) -> i64 {
    parse_json_i64(item.get("size"))
        .or_else(|| parse_json_i64(item.get("fileSizeBytes")))
        .or_else(|| parse_json_i64(item.get("size_bytes")))
        .or_else(|| parse_json_i64(item.get("bundleSizeBytes")))
        .or_else(|| parse_json_i64(item.get("downloadSize")))
        .or_else(|| parse_json_i64(item.get("download_size")))
        .or_else(|| parse_json_i64(item.get("file_size")))
        .or_else(|| parse_json_i64(item.get("appSize")))
        .or_else(|| parse_json_i64(item.get("app_size")))
        .or_else(|| parse_json_i64(item.get("asset").and_then(|v| v.get("size"))))
        .or_else(|| parse_json_i64(item.get("metadata").and_then(|v| v.get("fileSizeBytes"))))
        .unwrap_or(0)
}

fn extract_version_created_at(item: &Value) -> String {
    item.get("created_at")
        .or(item.get("released_at"))
        .or(item.get("date"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn extract_version_source(item: &Value) -> String {
    item.get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn format_version_item(item: &Value) -> Value {
    let label = extract_version_label(item);
    let external_identifier = extract_version_external_identifier(item);
    let size = extract_version_size(item);
    let created_at = extract_version_created_at(item);
    let source = extract_version_source(item);
    serde_json::json!({
        "bundle_version": label,
        "version": label,
        "external_identifier": external_identifier,
        "version_id": if external_identifier > 0 { external_identifier.to_string() } else { item.get("version_id").and_then(|v| v.as_str()).unwrap_or("").to_string() },
        "size": size,
        "size_bytes": if size > 0 { Some(size) } else { None },
        "created_at": created_at,
        "released_at": item.get("released_at").and_then(|v| v.as_str()).unwrap_or(&created_at).to_string(),
        "description": item.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "source": source,
    })
}

fn merge_version_records(primary: &mut Value, fallback: &Value) {
    let primary_size = extract_version_size(primary);
    if primary_size <= 0 {
        let fallback_size = extract_version_size(fallback);
        if fallback_size > 0 {
            if let Some(primary_obj) = primary.as_object_mut() {
                primary_obj.insert("size".to_string(), serde_json::json!(fallback_size));
            }
        }
    }

    let primary_created_at = extract_version_created_at(primary);
    if primary_created_at.is_empty() {
        let fallback_created_at = extract_version_created_at(fallback);
        if !fallback_created_at.is_empty() {
            if let Some(primary_obj) = primary.as_object_mut() {
                primary_obj.insert(
                    "created_at".to_string(),
                    serde_json::json!(fallback_created_at),
                );
            }
        }
    }
}

fn version_merge_key(item: &Value) -> Option<String> {
    let external_identifier = extract_version_external_identifier(item);
    if external_identifier > 0 {
        return Some(format!("id:{}", external_identifier));
    }

    let bundle_version = extract_version_label(item);
    if bundle_version.is_empty() {
        return None;
    }

    let created_at = extract_version_created_at(item);
    if !created_at.is_empty() {
        Some(format!("ver:{}@{}", bundle_version, created_at))
    } else {
        Some(format!("ver:{}", bundle_version))
    }
}

fn merge_version_lists(primary: Vec<Value>, secondary: Vec<Value>) -> Vec<Value> {
    let mut merged = primary;
    let mut index_by_key: HashMap<String, usize> = HashMap::new();

    for (index, item) in merged.iter().enumerate() {
        if let Some(key) = version_merge_key(item) {
            index_by_key.insert(key, index);
        }
    }

    for item in secondary {
        if let Some(key) = version_merge_key(&item) {
            if let Some(existing_index) = index_by_key.get(&key).copied() {
                merge_version_records(&mut merged[existing_index], &item);
            } else {
                index_by_key.insert(key, merged.len());
                merged.push(item);
            }
        } else {
            merged.push(item);
        }
    }

    merged
}

fn with_version_source(items: Vec<Value>, source: &str) -> Vec<Value> {
    items
        .into_iter()
        .map(|mut item| {
            if let Some(obj) = item.as_object_mut() {
                obj.entry("source".to_string())
                    .or_insert_with(|| serde_json::json!(source));
            }
            item
        })
        .collect()
}

async fn fetch_external_version_source(url: String, source: &'static str) -> Vec<Value> {
    let client = build_http_client();
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<Value>().await {
            Ok(json) => with_version_source(extract_version_array(&json), source),
            Err(_) => vec![],
        },
        _ => vec![],
    }
}

async fn fetch_external_version_sources(appid: &str, region: &str) -> Vec<Value> {
    let url1 = format!(
        "https://api.timbrd.com/apple/app-version/index.php?id={}&country={}",
        appid, region
    );
    let url2 = format!(
        "https://apis.bilin.eu.org/history/{}?country={}",
        appid, region
    );

    let (versions1, versions2) = futures_util::future::join(
        fetch_external_version_source(url1, "timbrd"),
        fetch_external_version_source(url2, "bilin"),
    )
    .await;

    merge_version_lists(versions1, versions2)
}

fn community_versions_to_values(app: &CommunityDelistedAppDetail) -> Vec<Value> {
    app.versions
        .iter()
        .map(|version| {
            let external_identifier = version.version_id.trim().parse::<i64>().unwrap_or(0);
            let size = version.size_bytes.unwrap_or(0);
            serde_json::json!({
                "bundle_version": version.version,
                "version": version.version,
                "external_identifier": external_identifier,
                "version_id": version.version_id,
                "size": size,
                "size_bytes": version.size_bytes,
                "created_at": version.released_at.clone().unwrap_or_default(),
                "released_at": version.released_at.clone().unwrap_or_default(),
                "description": version.description.clone().unwrap_or_default(),
                "source": if version.source.trim().is_empty() { "community" } else { version.source.as_str() },
            })
        })
        .collect()
}

fn version_numeric_parts(label: &str) -> Vec<i64> {
    label
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|part| part.parse::<i64>().ok())
        .collect()
}

fn compare_versions_desc(a: &Value, b: &Value) -> std::cmp::Ordering {
    let left = version_numeric_parts(&extract_version_label(a));
    let right = version_numeric_parts(&extract_version_label(b));
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let av = left.get(index).copied().unwrap_or(0);
        let bv = right.get(index).copied().unwrap_or(0);
        if av != bv {
            return bv.cmp(&av);
        }
    }
    extract_version_created_at(b).cmp(&extract_version_created_at(a))
}

// 查询版本：聚合 ipa-archive 社区归档 + timbrd + bilin
async fn get_versions(query: web::Query<VersionQuery>) -> impl Responder {
    let appid = query.appid.trim();
    if !is_valid_app_id(appid) {
        return invalid_app_id_response();
    }
    let region = store_region_or(query.region.as_deref(), "US");
    let region = region.as_str();

    let (community_app, external_versions) = futures_util::future::join(
        fetch_community_delisted_app(appid),
        fetch_external_version_sources(appid, region),
    )
    .await;

    let community_versions = community_app
        .as_ref()
        .map(community_versions_to_values)
        .unwrap_or_default();

    // 社区归档优先作为 primary，第三方接口只补 size/date 等缺失字段。
    let mut final_versions = merge_version_lists(community_versions, external_versions);
    final_versions.sort_by(compare_versions_desc);

    let formatted_versions: Vec<Value> = final_versions
        .iter()
        .map(format_version_item)
        .filter(|v| {
            let label_ok = v
                .get("bundle_version")
                .and_then(|bv| bv.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let id_ok = v
                .get("external_identifier")
                .and_then(|ei| ei.as_i64())
                .map(|id| id > 0)
                .unwrap_or(false)
                || v.get("version_id")
                    .and_then(|id| id.as_str())
                    .map(|id| !id.trim().is_empty())
                    .unwrap_or(false);
            label_ok && id_ok
        })
        .collect();

    HttpResponse::Ok().json(ApiResponse::success(formatted_versions))
}

fn build_base_url(req: &HttpRequest) -> String {
    if let Ok(public_base_url) = std::env::var("IPA_TOOL_PUBLIC_BASE_URL") {
        let trimmed = public_base_url.trim().trim_end_matches('/');
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    let info = req.connection_info();
    format!("{}://{}", info.scheme(), info.host())
}

fn resolve_project_root() -> PathBuf {
    if let Ok(root) = std::env::var("IPA_TOOL_ROOT") {
        return PathBuf::from(root);
    }

    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.clone());
        if let Some(parent) = cwd.parent() {
            candidates.push(parent.to_path_buf());
            if let Some(grand_parent) = parent.parent() {
                candidates.push(grand_parent.to_path_buf());
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.to_path_buf());
            if let Some(grand_parent) = parent.parent() {
                candidates.push(grand_parent.to_path_buf());
                if let Some(great_grand_parent) = grand_parent.parent() {
                    candidates.push(great_grand_parent.to_path_buf());
                }
            }
        }
    }

    for candidate in candidates {
        if candidate.join("server/Cargo.toml").exists() && candidate.join("src").exists() {
            return candidate;
        }
        if candidate.file_name().and_then(|name| name.to_str()) == Some("server")
            && candidate.join("Cargo.toml").exists()
        {
            if let Some(parent) = candidate.parent() {
                return parent.to_path_buf();
            }
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_database_path(project_root: &Path) -> PathBuf {
    std::env::var("DATABASE_PATH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root.join("data").join("ipa-webtool.db"))
}

fn print_admin_password_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} reset-admin-password [--username <username>] --password <new-password>");
    eprintln!("  {program} reset-admin-password [--username <username>] --password-stdin");
    eprintln!();
    eprintln!("Defaults:");
    eprintln!("  --username admin");
}

fn read_password_from_stdin() -> std::io::Result<String> {
    use std::io::Read;

    let mut password = String::new();
    std::io::stdin().read_to_string(&mut password)?;
    Ok(password.trim_end_matches(['\r', '\n']).to_string())
}

fn handle_reset_admin_password_args(args: &[String]) -> std::io::Result<bool> {
    let Some(command) = args.get(1) else {
        return Ok(false);
    };
    if command != "reset-admin-password" {
        return Ok(false);
    }

    let program = args.first().map(String::as_str).unwrap_or("server");
    let mut username = "admin".to_string();
    let mut password: Option<String> = None;
    let mut password_stdin = false;

    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--username" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    print_admin_password_usage(program);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "missing value for --username",
                    ));
                };
                username = value.clone();
            }
            "--password" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    print_admin_password_usage(program);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "missing value for --password",
                    ));
                };
                password = Some(value.clone());
            }
            "--password-stdin" => {
                password_stdin = true;
            }
            "--help" | "-h" => {
                print_admin_password_usage(program);
                return Ok(true);
            }
            other => {
                print_admin_password_usage(program);
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("unknown argument: {other}"),
                ));
            }
        }
        index += 1;
    }

    if username.trim().is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "username must not be empty",
        ));
    }

    let new_password = if password_stdin {
        if password.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "use either --password or --password-stdin, not both",
            ));
        }
        read_password_from_stdin()?
    } else {
        password.unwrap_or_default()
    };

    if new_password.is_empty() {
        print_admin_password_usage(program);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "password must not be empty",
        ));
    }

    let project_root = resolve_project_root();
    let db_path = resolve_database_path(&project_root);
    if !db_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("database not found: {}", db_path.display()),
        ));
    }

    let db = Database::new(db_path.to_string_lossy().as_ref())
        .map_err(|e| std::io::Error::other(format!("failed to open database: {e}")))?;
    let updated = db
        .reset_admin_password(username.trim(), &new_password)
        .map_err(|e| std::io::Error::other(format!("failed to reset password: {e}")))?;

    if !updated {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("admin user '{}' not found", username.trim()),
        ));
    }

    println!(
        "Admin password reset successfully for user '{}' in {}. Existing sessions were revoked.",
        username.trim(),
        db_path.display()
    );
    Ok(true)
}

fn artifact_id_from_path(path: &Path, downloads_dir: &Path) -> Option<String> {
    let relative = path.strip_prefix(downloads_dir).ok()?;
    Some(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(relative.to_string_lossy().as_bytes()),
    )
}

fn resolve_artifact_path(downloads_dir: &Path, artifact_id: &str) -> Option<PathBuf> {
    let canonical_root = downloads_dir.canonicalize().ok()?;

    let try_relative = |relative: &str| -> Option<PathBuf> {
        let candidate = downloads_dir.join(relative);
        let canonical = candidate.canonicalize().ok()?;
        if canonical.starts_with(&canonical_root) {
            Some(canonical)
        } else {
            None
        }
    };

    if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(artifact_id) {
        if let Ok(relative) = String::from_utf8(decoded) {
            if let Some(path) = try_relative(&relative) {
                return Some(path);
            }
        }
    }

    // 兼容历史 artifact id：旧实现把 `/` 和 `\\` 都替换成 `__`，
    // 当文件名本身包含 `__` 时不可逆，因此这里通过重扫文件系统按旧规则比对，
    // 而不是继续使用 `replace("__", "/")` 这种有歧义的反解方式。
    scan_download_artifacts(downloads_dir)
        .into_iter()
        .find(|artifact| {
            artifact
                .path
                .strip_prefix(downloads_dir)
                .ok()
                .map(|relative| relative.to_string_lossy().replace(['\\', '/'], "__"))
                .as_deref()
                == Some(artifact_id)
        })
        .map(|artifact| artifact.path)
}

/// Lifetime of signed public links handed to iOS installers (OTA manifest -> IPA download).
const PUBLIC_LINK_TTL_SECS: i64 = 7 * 24 * 60 * 60;

static PUBLIC_LINK_KEY: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

fn public_link_key() -> &'static [u8] {
    // Falls back to a per-process key when `init_public_link_key` was not called (tests) or the
    // persisted key is unavailable; links then simply stop validating after a restart.
    PUBLIC_LINK_KEY.get_or_init(|| rand::random::<[u8; 32]>().to_vec())
}

/// Derive the link-signing key from the persisted encryption key so links survive restarts.
fn init_public_link_key(db: &Database) {
    match ipa_webtool_services::crypto::ensure_encryption_key(db) {
        Ok(key_hex) => {
            if let Some(derived) = hmac_sha256(key_hex.as_bytes(), b"ipatool-public-link-v1") {
                let _ = PUBLIC_LINK_KEY.set(derived);
            }
        }
        Err(error) => log::warn!(
            "[public-link] encryption key unavailable, using per-process signing key: {}",
            error
        ),
    }
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Option<Vec<u8>> {
    let key = openssl::pkey::PKey::hmac(key).ok()?;
    let mut signer =
        openssl::sign::Signer::new(openssl::hash::MessageDigest::sha256(), &key).ok()?;
    signer.update(message).ok()?;
    signer.sign_to_vec().ok()
}

fn sign_public_link(resource: &str, expires: i64) -> String {
    let message = format!("{}:{}", resource, expires);
    hmac_sha256(public_link_key(), message.as_bytes())
        .map(|mac| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&mac[..16]))
        .unwrap_or_default()
}

fn verify_public_link(resource: &str, expires: Option<i64>, sig: Option<&str>) -> bool {
    let (Some(expires), Some(sig)) = (expires, sig) else {
        return false;
    };
    if expires < Utc::now().timestamp() {
        return false;
    }
    let expected = sign_public_link(resource, expires);
    !expected.is_empty()
        && expected.len() == sig.len()
        && openssl::memcmp::eq(expected.as_bytes(), sig.as_bytes())
}

fn record_link_resource(record_id: i64) -> String {
    format!("record:{}", record_id)
}

/// Query parameters carried by signed public links.
#[derive(Deserialize, Default)]
struct SignedLinkQuery {
    expires: Option<i64>,
    sig: Option<String>,
}

/// Whether the request carries a valid admin session cookie.
fn request_has_admin_session(req: &HttpRequest, data: &AppState) -> bool {
    req.cookie(ADMIN_SESSION_COOKIE)
        .is_some_and(|cookie| resolve_admin_session(data, cookie.value()).is_ok())
}

/// Check that `link` is a signed download-record URL produced by `build_record_download_url`.
fn is_signed_record_link(link: &str) -> bool {
    let Some(record_id) = extract_record_id_from_download_url(link) else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(link) else {
        return false;
    };
    let mut expires = None;
    let mut sig = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "expires" => expires = value.parse::<i64>().ok(),
            "sig" => sig = Some(value.into_owned()),
            _ => {}
        }
    }
    verify_public_link(&record_link_resource(record_id), expires, sig.as_deref())
}

/// Public download URL for a record. iOS installers fetch it without cookies, so it carries an
/// expiring HMAC signature instead; the record id alone is sequential and must not grant access.
fn build_record_download_url(base_url: &str, record_id: i64) -> String {
    let expires = Utc::now().timestamp() + PUBLIC_LINK_TTL_SECS;
    format!(
        "{}/api/public/download-records/{}/file?expires={}&sig={}",
        base_url,
        record_id,
        expires,
        sign_public_link(&record_link_resource(record_id), expires)
    )
}

fn inspection_blocks_install(inspection: &IpaInspection) -> bool {
    !inspection.direct_install_ok
}

fn is_placeholder_bundle_id(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || trimmed.eq_ignore_ascii_case("unknown.bundle")
}

fn derive_delivery_decision(
    inspection: Option<&IpaInspection>,
    file_exists: bool,
) -> DeliveryDecision {
    match inspection {
        Some(inspection)
            if inspection.direct_install_ok && inspection.has_embedded_mobileprovision =>
        {
            // Developer-signed / sideloaded IPA with provisioning profile
            DeliveryDecision {
                package_kind: "ota_sideloadable".to_string(),
                ota_installable: true,
                install_method: "ota".to_string(),
            }
        }
        Some(inspection) if inspection.direct_install_ok && inspection.has_sc_info_manifest => {
            // App Store IPA with all declared sinfs properly injected.
            // FairPlay encrypted binaries are expected and handled by iOS at runtime.
            // This matches ipatool.js and ApplePackage (Asspp) behavior.
            DeliveryDecision {
                package_kind: "appstore_sinf_package".to_string(),
                ota_installable: true,
                install_method: "ota".to_string(),
            }
        }
        Some(inspection)
            if inspection.has_sc_info_manifest && !inspection.missing_sinf_paths.is_empty() =>
        {
            // App Store IPA but sinf injection incomplete — cannot install
            DeliveryDecision {
                package_kind: "appstore_incomplete_sinf".to_string(),
                ota_installable: false,
                install_method: if file_exists {
                    "download_only"
                } else {
                    "manual_review"
                }
                .to_string(),
            }
        }
        Some(inspection) if !inspection.encrypted_binaries.is_empty() => {
            // Encrypted binaries with no sinfs and no provisioning profile
            DeliveryDecision {
                package_kind: "fairplay_encrypted".to_string(),
                ota_installable: false,
                install_method: if file_exists {
                    "download_only"
                } else {
                    "manual_review"
                }
                .to_string(),
            }
        }
        Some(_) => DeliveryDecision {
            package_kind: "broken_or_unknown".to_string(),
            ota_installable: false,
            install_method: if file_exists {
                "download_only"
            } else {
                "manual_review"
            }
            .to_string(),
        },
        None => DeliveryDecision {
            package_kind: if file_exists {
                "unknown".to_string()
            } else {
                "missing_file".to_string()
            },
            ota_installable: false,
            install_method: if file_exists { "manual_review" } else { "none" }.to_string(),
        },
    }
}

fn persisted_inspection(record: &DownloadRecord) -> Option<IpaInspection> {
    record
        .inspection_json
        .as_ref()
        .and_then(|raw| serde_json::from_str::<IpaInspection>(raw).ok())
}

fn inspection_for_record(record: &DownloadRecord) -> Option<IpaInspection> {
    // 优先读 DB 持久化结果，避免每次请求都重新 inspect ZIP
    if let Some(cached) = persisted_inspection(record) {
        return Some(cached);
    }
    // DB 没有，才做文件 inspect（用于 backfill）
    record
        .file_path
        .as_ref()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .as_deref()
        .and_then(inspect_existing_ipa)
}

fn sync_record_delivery(
    db: &Database,
    record: &DownloadRecord,
    inspection: Option<&IpaInspection>,
    file_exists: bool,
) {
    let Some(record_id) = record.id else {
        return;
    };

    let decision = derive_delivery_decision(inspection, file_exists);
    let inspection_json = inspection.and_then(|value| serde_json::to_string(value).ok());

    let stored_matches = record.package_kind.as_deref() == Some(decision.package_kind.as_str())
        && record.ota_installable == Some(decision.ota_installable)
        && record.install_method.as_deref() == Some(decision.install_method.as_str())
        && match (&record.inspection_json, &inspection_json) {
            (Some(left), Some(right)) => left == right,
            (None, None) => true,
            _ => false,
        };

    if !stored_matches {
        let _ = db.update_download_record_delivery(
            record_id,
            Some(decision.package_kind.as_str()),
            Some(decision.ota_installable),
            Some(decision.install_method.as_str()),
            inspection_json.as_deref(),
        );
    }
}

fn extract_record_id_from_download_url(download_url: &str) -> Option<i64> {
    let marker = "/api/public/download-records/";
    let rest = download_url.split(marker).nth(1)?;
    let record_id = rest.split('/').next()?;
    record_id.parse().ok()
}

fn build_record_install_url(
    base_url: &str,
    record: &DownloadRecord,
    record_id: i64,
) -> Option<String> {
    let inspection = inspection_for_record(record);
    let file_exists = record
        .file_path
        .as_ref()
        .map(PathBuf::from)
        .map(|path| path.exists())
        .unwrap_or(false);
    let decision = derive_delivery_decision(inspection.as_ref(), file_exists);
    if !decision.ota_installable {
        return None;
    }

    let download_url = build_record_download_url(base_url, record_id);
    let mut bundle_id = record
        .bundle_id
        .clone()
        .filter(|value| !is_placeholder_bundle_id(value));
    if bundle_id.is_none() {
        bundle_id = record
            .file_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .and_then(|path| read_bundle_identifier_from_ipa(&path).ok().flatten())
            .filter(|value| !is_placeholder_bundle_id(value));
    }
    let bundle_id = bundle_id?;
    let bundle_version = record.version.clone().filter(|value| !value.is_empty())?;
    let title = if record.app_name.is_empty() {
        record
            .file_path
            .as_ref()?
            .rsplit('/')
            .next()?
            .trim_end_matches(".ipa")
            .to_string()
    } else {
        record.app_name.clone()
    };

    // 仿 OpenList：对外暴露 /i/{token}.plist，但额外带上 bundle_version 以保留真实版本信息。
    let name_full = format!("{}@{}", title, bundle_id);
    let link_encode = urlencoding::encode(&download_url);
    let name_encode = urlencoding::encode(&name_full);
    let version_encode = urlencoding::encode(&bundle_version);
    let raw = format!("{}/{}/{}", link_encode, name_encode, version_encode);
    let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes());

    let manifest_url = format!("{}/i/{}.plist", base_url, token);

    Some(format!(
        "itms-services://?action=download-manifest&url={}",
        urlencoding::encode(&manifest_url)
    ))
}

fn scan_download_artifacts(downloads_dir: &Path) -> Vec<DownloadArtifact> {
    fn visit(dir: &Path, root: &Path, artifacts: &mut Vec<DownloadArtifact>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, root, artifacts);
                continue;
            }

            if path.extension().and_then(|ext| ext.to_str()) != Some("ipa") {
                continue;
            }

            let meta = match entry.metadata() {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            let id = match artifact_id_from_path(&path, root) {
                Some(id) => id,
                None => continue,
            };
            let modified_at = meta.modified().ok().map(chrono::DateTime::<Utc>::from);
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("download.ipa")
                .to_string();

            artifacts.push(DownloadArtifact {
                id,
                path,
                file_name,
                file_size: meta.len(),
                modified_at,
            });
        }
    }

    let mut artifacts = Vec::new();
    if downloads_dir.exists() {
        visit(downloads_dir, downloads_dir, &mut artifacts);
        artifacts.sort_by_key(|a| std::cmp::Reverse(a.modified_at));
    }
    artifacts
}

fn inspect_existing_ipa(path: &Path) -> Option<IpaInspection> {
    match inspect_ipa_path(path) {
        Ok(inspection) => Some(inspection),
        Err(error) => {
            log::warn!("failed to inspect ipa {}: {}", path.display(), error);
            None
        }
    }
}

fn normalize_download_record_artifact_paths(db: &Database, downloads_dir: &Path) {
    let records = db.get_all_download_records().unwrap_or_default();
    let canonical_root = match downloads_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => return,
    };

    for mut record in records {
        let Some(record_id) = record.id else {
            continue;
        };
        let Some(bundle_id) = record
            .bundle_id
            .clone()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let Some(version) = record
            .version
            .clone()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let Some(file_path) = record.file_path.clone() else {
            continue;
        };

        let current_path = PathBuf::from(&file_path);
        if !current_path.exists() {
            continue;
        }
        let Ok(canonical_current) = current_path.canonicalize() else {
            continue;
        };
        if !canonical_current.starts_with(&canonical_root) {
            continue;
        }

        let current_name = canonical_current
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let expected_name = canonical_ipa_filename(&record.app_name, &version, Some(&bundle_id));

        if current_name == expected_name {
            continue;
        }

        let Some(parent_dir) = canonical_current.parent() else {
            continue;
        };
        let target_path = parent_dir.join(&expected_name);
        let final_path = if target_path.exists() {
            target_path
        } else {
            if let Err(error) = std::fs::rename(&canonical_current, &target_path) {
                log::warn!(
                    "failed to rename legacy ipa artifact {} -> {}: {}",
                    canonical_current.display(),
                    target_path.display(),
                    error
                );
                continue;
            }
            target_path
        };

        record.file_path = Some(final_path.to_string_lossy().to_string());
        if let Some(size) = std::fs::metadata(&final_path)
            .ok()
            .map(|meta| meta.len() as i64)
        {
            record.file_size = Some(size);
        }
        let _ = db.update_download_record(record_id, &record);
    }
}

fn sync_download_records_from_filesystem(db: &Database, downloads_dir: &Path) {
    let existing_records = db.get_all_download_records().unwrap_or_default();
    let mut known_paths: HashMap<String, DownloadRecord> = HashMap::new();
    for record in existing_records {
        if let Some(path) = record.file_path.clone() {
            known_paths.insert(path, record);
        }
    }

    for artifact in scan_download_artifacts(downloads_dir) {
        let absolute_path = artifact.path.to_string_lossy().to_string();
        if known_paths.contains_key(&absolute_path) {
            continue;
        }

        let app_name = artifact.file_name.trim_end_matches(".ipa").to_string();
        let inferred = DownloadRecord {
            id: None,
            job_id: None,
            app_version_id: None,
            app_name,
            app_id: "unknown".to_string(),
            bundle_id: None,
            version: None,
            account_email: "未知账号".to_string(),
            account_region: None,
            download_date: artifact.modified_at.map(|dt| dt.to_rfc3339()),
            status: "completed".to_string(),
            file_size: Some(artifact.file_size as i64),
            file_path: Some(absolute_path),
            install_url: None,
            artwork_url: None,
            artist_name: None,
            progress: Some(100),
            error: None,
            package_kind: None,
            ota_installable: None,
            install_method: None,
            inspection_json: None,
            created_at: None,
            delisted: None,
        };
        let _ = db.add_download_record(&inferred);
    }
}

fn build_job_manifest_url(req: &HttpRequest, job_id: &str) -> String {
    format!(
        "{}/api/public/manifest?jobId={}",
        build_base_url(req),
        urlencoding::encode(job_id)
    )
}

fn build_job_install_url(req: &HttpRequest, job_id: &str) -> String {
    let manifest_url = build_job_manifest_url(req, job_id);
    format!(
        "itms-services://?action=download-manifest&url={}",
        urlencoding::encode(&manifest_url)
    )
}

fn encode_sse<T: Serialize>(event_name: &str, payload: &T) -> Result<Bytes, Error> {
    let payload = serde_json::to_string(payload).map_err(ErrorInternalServerError)?;
    Ok(Bytes::from(format!(
        "event: {}\ndata: {}\n\n",
        event_name, payload
    )))
}

fn encode_job_event(event: JobEvent) -> Result<Bytes, Error> {
    match event {
        JobEvent::Progress(payload) => encode_sse("progress", &payload),
        JobEvent::Log(payload) => encode_sse("log", &payload),
        JobEvent::End(payload) => encode_sse("end", &payload),
    }
}

fn snapshot_progress_event(snapshot: &JobState) -> JobProgressEvent {
    JobProgressEvent {
        status: Some(snapshot.status.clone()),
        progress: Some(JobProgressPayload {
            stage: snapshot.stage.clone(),
            percent: snapshot.progress,
            downloaded: None,
            total: None,
            message: None,
        }),
        error: snapshot.error.clone(),
    }
}

fn build_download_task_slug(
    app_name: Option<&str>,
    app_id: &str,
    version: Option<&str>,
    app_ver_id: Option<&str>,
) -> String {
    let name = app_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(app_id);
    let version_part = version
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| app_ver_id.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or("latest");
    let raw = format!("{}-{}-{}", name, app_id, version_part);
    sanitize_ipa_filename(&raw)
        .trim_end_matches(".ipa")
        .trim_matches('_')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn signed_record_links_verify_and_reject_tampering() {
        let url = build_record_download_url("https://ipa.example.com", 42);
        assert!(url.starts_with("https://ipa.example.com/api/public/download-records/42/file?"));
        assert!(is_signed_record_link(&url));

        // Another record id, a modified expiry or a bare link must all be rejected.
        assert!(!is_signed_record_link(&url.replace("/42/", "/43/")));
        let expires = Utc::now().timestamp() + PUBLIC_LINK_TTL_SECS;
        let forged = url.replace(&format!("expires={}", expires), "expires=99999999999");
        assert!(!is_signed_record_link(&forged));
        assert!(!is_signed_record_link(
            "https://ipa.example.com/api/public/download-records/42/file"
        ));
        assert!(!is_signed_record_link("https://evil.example/app.ipa"));

        let past = Utc::now().timestamp() - 1;
        let expired_sig = sign_public_link(&record_link_resource(42), past);
        assert!(!verify_public_link(
            &record_link_resource(42),
            Some(past),
            Some(&expired_sig)
        ));
        assert!(!verify_public_link(
            &record_link_resource(42),
            Some(expires),
            Some("short")
        ));
    }

    #[test]
    fn admin_password_hash_check_detects_concurrent_change() {
        let db = Database::new(":memory:").unwrap();
        let snapshot = db.get_admin_user("admin").unwrap().unwrap().password_hash;
        assert!(admin_password_hash_unchanged(&db, "admin", &snapshot));

        db.update_admin_password("admin", &hash_password("changed-elsewhere"), false)
            .unwrap();
        assert!(!admin_password_hash_unchanged(&db, "admin", &snapshot));
        assert!(!admin_password_hash_unchanged(&db, "ghost", &snapshot));
    }

    #[test]
    fn app_id_and_region_validation() {
        assert!(is_valid_app_id("414478124"));
        assert!(!is_valid_app_id(""));
        assert!(!is_valid_app_id("../data/ipa-webtool"));
        assert!(!is_valid_app_id("123abc"));
        assert!(!is_valid_app_id(&"9".repeat(21)));

        assert_eq!(store_region_or(Some("cn"), "US"), "CN");
        assert_eq!(store_region_or(Some(" us "), "CN"), "US");
        assert_eq!(store_region_or(Some("US&x=1"), "US"), "US");
        assert_eq!(store_region_or(None, "CN"), "CN");
    }

    #[test]
    fn secret_masking_is_utf8_safe() {
        assert_eq!(mask_password(""), "");
        assert_eq!(mask_password("密码密码密码密码"), "密****码");
        assert_eq!(mask_password("abc"), "***");
        assert_eq!(
            mask_github_token("ghp_abcdefghijklmnopqrstuvwxyz"),
            "ghp_****wxyz"
        );
        assert_eq!(mask_github_token("short"), "****");
    }

    pub(super) async fn purchase_test_state(label: &str) -> (web::Data<AppState>, String, String) {
        let token = format!("purchase-{label}-{}", uuid::Uuid::new_v4());
        let email = format!("{token}@example.com");
        ACCOUNTS
            .write()
            .await
            .insert(token.clone(), AccountStore::new(&email));
        let db = Arc::new(Mutex::new(Database::new(":memory:").unwrap()));
        db.lock()
            .unwrap()
            .save_account(&ipa_webtool_services::Account {
                id: None,
                token: token.clone(),
                email: email.clone(),
                region: "US".into(),
                guid: None,
                cookie_user: None,
                cookies: None,
                created_at: None,
                updated_at: None,
            })
            .unwrap();
        let downloads_dir = PathBuf::from(
            std::env::var("TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir())
                .to_string_lossy()
                .into_owned(),
        );
        let state = web::Data::new(AppState {
            download_manager: Arc::new(DownloadManager::new(db.clone(), downloads_dir.clone())),
            db,
            downloads_dir,
            job_store: JobStore::new(),
            login_rate_limiter: Arc::new(Mutex::new(LoginRateLimiter::new())),
        });
        (state, token, email)
    }

    #[actix_web::test]
    async fn purchase_cached_failure_survives_single_and_batch_endpoints() {
        let (state, token, email) = purchase_test_state("cached").await;
        let error = "This item is not available in your storefront (license lookup failed)";
        PURCHASE_CACHE.write().await.insert(
            (email.clone(), "42".into()),
            PurchaseCacheEntry {
                purchased: false,
                needs_purchase: false,
                error: Some(error.into()),
                cached_at: Instant::now(),
            },
        );
        let app = actix_web::test::init_service(
            App::new()
                .app_data(state)
                .route("/single", web::get().to(get_purchase_status))
                .route("/batch", web::post().to(purchase_status_batch)),
        )
        .await;
        let request = actix_web::test::TestRequest::get()
            .uri(&format!("/single?token={token}&appid=42"))
            .to_request();
        let single: serde_json::Value =
            actix_web::test::call_and_read_body_json(&app, request).await;
        assert_eq!(single["data"]["error"], error);
        assert_eq!(single["data"]["status"], "unknown");
        let request = actix_web::test::TestRequest::post()
            .uri("/batch")
            .set_json(serde_json::json!({"token": token, "appids": ["42"]}))
            .to_request();
        let batch: serde_json::Value =
            actix_web::test::call_and_read_body_json(&app, request).await;
        assert_eq!(batch["data"]["results"]["42"], single["data"]);
        ACCOUNTS.write().await.remove(&token);
        PURCHASE_CACHE.write().await.remove(&(email, "42".into()));
    }

    #[actix_web::test]
    async fn purchase_owned_email_case_and_whitespace_avoids_upstream() {
        let (state, token, email) = purchase_test_state("owned").await;
        state
            .db
            .lock()
            .unwrap()
            .record_purchase(&email, "42", "test")
            .unwrap();
        ACCOUNTS
            .write()
            .await
            .get_mut(&token)
            .unwrap()
            .account_email = format!(" {} ", email.to_uppercase());
        let app = actix_web::test::init_service(
            App::new()
                .app_data(state)
                .route("/single", web::get().to(get_purchase_status)),
        )
        .await;
        let request = actix_web::test::TestRequest::get()
            .uri(&format!("/single?token={token}&appid=42"))
            .to_request();
        let result: serde_json::Value =
            actix_web::test::call_and_read_body_json(&app, request).await;
        assert_eq!(result["data"]["status"], "owned");
        ACCOUNTS.write().await.remove(&token);
    }

    #[actix_web::test]
    async fn purchase_database_failure_never_becomes_not_owned() {
        let (state, token, email) = purchase_test_state("db-failure").await;
        let path = state
            .downloads_dir
            .join(format!("purchase-db-{}.sqlite", uuid::Uuid::new_v4()));
        *state.db.lock().unwrap() = Database::new(path.to_str().unwrap()).unwrap();
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute("DROP TABLE purchase_records", [])
            .unwrap();
        PURCHASE_CACHE.write().await.insert(
            (email.clone(), "42".into()),
            PurchaseCacheEntry {
                purchased: false,
                needs_purchase: true,
                error: Some("license not found".into()),
                cached_at: Instant::now(),
            },
        );
        assert!(persist_purchase(&state, &email, "42", "test").is_err());
        let app = actix_web::test::init_service(
            App::new()
                .app_data(state)
                .route("/single", web::get().to(get_purchase_status))
                .route("/batch", web::post().to(purchase_status_batch)),
        )
        .await;
        for request in [
            actix_web::test::TestRequest::get()
                .uri(&format!("/single?token={token}&appid=42"))
                .to_request(),
            actix_web::test::TestRequest::post()
                .uri("/batch")
                .set_json(serde_json::json!({"token": token, "appids": ["42"]}))
                .to_request(),
        ] {
            let response = actix_web::test::call_service(&app, request).await;
            assert_eq!(
                response.status(),
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR
            );
            let body: serde_json::Value = actix_web::test::read_body_json(response).await;
            assert!(body["error"].as_str().unwrap().contains("purchase_records"));
        }
        ACCOUNTS.write().await.remove(&token);
        PURCHASE_CACHE.write().await.remove(&(email, "42".into()));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn purchase_known_not_owned_is_not_an_operational_error() {
        assert_eq!(purchase_failure_error("license not found"), None);
        assert_eq!(
            purchase_failure_error("2042: Sign In to the iTunes Store"),
            Some("2042: Sign In to the iTunes Store".into())
        );
    }

    #[test]
    fn purchase_authentication_failure_explicitly_requires_login() {
        assert!(purchase_requires_login("2042"));
        assert!(purchase_requires_login("Sign In to the iTunes Store"));
        assert!(purchase_requires_login("登录到 iTunes Store。"));
        assert!(!purchase_requires_login("license not found"));
    }

    #[test]
    fn purchase_failure_classification_does_not_guess_from_region_or_license_word() {
        for error in [
            "license server unavailable",
            "item not found in this storefront",
            "未找到此地区商品",
            "2042: Sign In to the iTunes Store",
            "2042: 登录到 iTunes Store。",
        ] {
            assert!(!purchase_required(error), "{error}");
        }
        assert!(purchase_required("license not found"));
        assert!(purchase_required("You have not purchased this app"));
    }

    #[test]
    fn test_bcrypt_hash_and_verify() {
        let hash =
            bcrypt::hash("admin", bcrypt::DEFAULT_COST).expect("bcrypt::hash should not fail");
        assert!(
            hash.starts_with("$2b$"),
            "bcrypt hash should start with $2b$, got: {}",
            hash
        );
        assert!(bcrypt::verify("admin", &hash).expect("bcrypt::verify should not fail"));
        assert!(!bcrypt::verify("wrong", &hash).expect("bcrypt::verify should not fail"));
    }

    #[test]
    fn test_generate_admin123_hash() {
        let hash = Database::hash_password("admin123");
        assert!(hash.len() == 60);
        // verify roundtrip
        assert!(bcrypt::verify("admin123", &hash).unwrap());
        // verify the stored hash in DB
        let stored = "$2b$12$0PJlomYxFRPWbHTKWypoSelCZjiq0FvqHwFKT.6z38Qv/nnk6Wl/W";
        assert!(bcrypt::verify("admin123", stored).unwrap());
    }

    #[test]
    fn build_download_task_slug_uses_stable_sanitized_components() {
        let slug =
            build_download_task_slug(Some("微信 / WeChat"), "414478124", Some("8.0.58"), None);
        assert_eq!(slug, "WeChat-414478124-8.0.58");
    }

    #[test]
    fn build_download_task_slug_falls_back_to_app_ver_id_or_latest() {
        let from_ver_id = build_download_task_slug(None, "414478124", None, Some("123456789"));
        assert_eq!(from_ver_id, "414478124-414478124-123456789");

        let latest = build_download_task_slug(Some("WeChat"), "414478124", None, None);
        assert_eq!(latest, "WeChat-414478124-latest");
    }

    #[tokio::test]
    async fn remove_empty_legacy_job_dir_only_removes_empty_directory() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ipatool-job-cleanup-{}", nonce));
        let empty_dir = root.join("empty-job");
        let non_empty_dir = root.join("non-empty-job");
        fs::create_dir_all(&empty_dir).unwrap();
        fs::create_dir_all(&non_empty_dir).unwrap();
        fs::write(non_empty_dir.join("artifact.txt"), b"keep").unwrap();

        remove_empty_legacy_job_dir(&root, "empty-job").await;
        remove_empty_legacy_job_dir(&root, "non-empty-job").await;

        assert!(!empty_dir.exists());
        assert!(non_empty_dir.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn account_session_survives_restart() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ipatool-account-session-{}", nonce));
        fs::create_dir_all(&root).unwrap();
        let db_path = root.join("test.db");
        let token = format!("session-test-{}", nonce);
        let email = format!("{}@example.com", nonce);

        {
            let db = Database::new(db_path.to_string_lossy().as_ref()).unwrap();
            db.save_account(&ipa_webtool_services::Account {
                id: None,
                token: token.clone(),
                email: email.clone(),
                region: "US".to_string(),
                guid: None,
                cookie_user: None,
                cookies: None,
                created_at: None,
                updated_at: None,
            })
            .unwrap();
            let store = AccountStore::restore(
                &email,
                "AABBCCDDEEFF".to_string(),
                ipa_webtool_services::AuthInfo {
                    ds_person_id: Some("12345".to_string()),
                    password_token: Some("secret-token".to_string()),
                    display_name: Some("Test".to_string()),
                    email: Some(email.clone()),
                    region: Some("US".to_string()),
                },
                1_700_000_000,
            );
            persist_account_session(&db, &token, &store).unwrap();

            let raw = db.get_account_sessions().unwrap();
            assert!(
                !raw[0].2.contains("secret-token"),
                "session must be encrypted"
            );
        }

        // 模拟重启：重新打开 DB 并恢复
        let db_arc = Arc::new(Mutex::new(
            Database::new(db_path.to_string_lossy().as_ref()).unwrap(),
        ));
        restore_account_sessions(&db_arc).await;

        let accounts = ACCOUNTS.read().await;
        let restored = accounts.get(&token).expect("account should be restored");
        assert_eq!(restored.account_email, email);
        assert_eq!(restored.store.guid, "AABBCCDDEEFF");
        assert_eq!(restored.authenticated_at, 1_700_000_000);
        let auth = restored.auth_info.as_ref().unwrap();
        assert_eq!(auth.ds_person_id.as_deref(), Some("12345"));
        assert_eq!(auth.password_token.as_deref(), Some("secret-token"));
        drop(accounts);

        let _ = fs::remove_dir_all(&root);
    }
}

async fn remove_empty_legacy_job_dir(job_root: &Path, job_id: &str) {
    let legacy_dir = job_root.join(job_id);
    if !legacy_dir.exists() {
        return;
    }
    if let Ok(mut entries) = tokio::fs::read_dir(&legacy_dir).await {
        if let Ok(None) = entries.next_entry().await {
            let _ = tokio::fs::remove_dir(&legacy_dir).await;
        }
    }
}

async fn start_download_direct(
    req_http: HttpRequest,
    body: web::Bytes,
    data: web::Data<AppState>,
) -> impl Responder {
    let req: StartDownloadDirectRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[start-download-direct] JSON parse error: {}", e);
            return HttpResponse::BadRequest()
                .json(ApiResponse::<String>::error(format!("请求解析失败: {}", e)));
        }
    };
    if !is_valid_app_id(req.appid.trim()) {
        return invalid_app_id_response();
    }
    let accounts = ACCOUNTS.read().await;
    let account_store = match accounts.get(&req.token) {
        Some(account) => account.clone(),
        None => {
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("无效的 token".to_string()));
        }
    };
    drop(accounts);
    let account_store = RecoveryStore::new(&req.token, &account_store, &data);

    if !req.autoPurchase {
        match account_store
            .download_product(&req.appid, req.appVerId.as_deref())
            .await
        {
            Ok(result) => {
                let state = result
                    .get("_state")
                    .and_then(|value| value.as_str())
                    .unwrap_or("failure");

                if state != "success" {
                    let error_message = result
                        .get("customerMessage")
                        .or(result.get("failureType"))
                        .or(result.get("message"))
                        .and_then(|value| value.as_str())
                        .unwrap_or("下载失败")
                        .to_string();

                    let is_license_error = purchase_required(&error_message);

                    if is_license_error {
                        return HttpResponse::BadRequest().json(serde_json::json!({
                            "ok": false,
                            "needsPurchase": true,
                            "error": get_license_error_message(&result),
                        }));
                    }

                    return HttpResponse::BadRequest()
                        .json(ApiResponse::<String>::error(error_message));
                }
            }
            Err(error) => {
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("创建任务失败: {}", error),
                ))
            }
        }
    }

    if !req.autoPurchase {
        if let Err(error) = persist_purchase(
            &data,
            &account_store.account_email,
            &req.appid,
            "direct_precheck",
        ) {
            return purchase_database_error(error);
        }
    }

    let appid = req.appid.clone();
    let app_ver_id = req.appVerId.clone();
    let app_name_hint = req.appName.clone().filter(|value| !value.is_empty());
    let bundle_id_hint = req
        .bundleId
        .clone()
        .filter(|value| !is_placeholder_bundle_id(value));
    let app_version_hint = req.appVersion.clone().filter(|value| !value.is_empty());
    let artwork_url_hint = req.artworkUrl.clone().filter(|value| !value.is_empty());
    let artist_name_hint = req.artistName.clone().filter(|value| !value.is_empty());
    let auto_purchase = req.autoPurchase;
    let account_email = account_store.account_email.clone();
    let account_region = data
        .db
        .lock()
        .ok()
        .and_then(|db| db.get_account_by_token(&req.token).ok().flatten())
        .map(|account| account.region)
        .filter(|value| !value.is_empty());
    let app_version_key = app_version_hint
        .clone()
        .or_else(|| app_ver_id.clone())
        .unwrap_or_else(|| "latest".to_string());
    let task_slug = build_download_task_slug(
        app_name_hint.as_deref(),
        &appid,
        Some(app_version_key.as_str()),
        app_ver_id.as_deref(),
    );
    let job_root = data.downloads_dir.join("jobs");
    let task_dir = job_root.join(&task_slug);

    // Look up a reusable record, then release the DB lock before any IPA inspection.
    let reusable_record = data
        .db
        .lock()
        .ok()
        .map(|db| db.find_reusable_download_record(&appid, &app_version_key, &account_email));
    if let Some(Ok(Some(existing_record))) = reusable_record {
        if let Some(file_path) = existing_record.file_path.clone() {
            let path = PathBuf::from(&file_path);
            if path.exists() {
                let inspection =
                    inspection_for_record(&existing_record).or_else(|| inspect_existing_ipa(&path));
                let decision = derive_delivery_decision(inspection.as_ref(), true);
                let base_url = build_base_url(&req_http);
                let download_url = existing_record
                    .id
                    .map(|record_id| build_record_download_url(&base_url, record_id));
                let install_url = if decision.ota_installable {
                    match existing_record.id {
                        Some(record_id) => {
                            build_record_install_url(&base_url, &existing_record, record_id)
                        }
                        None => None,
                    }
                } else {
                    None
                };
                let response = ExistingDownloadResponse {
                    job_id: existing_record
                        .job_id
                        .clone()
                        .unwrap_or_else(|| task_slug.clone()),
                    record_id: existing_record.id,
                    app_id: existing_record.app_id.clone(),
                    version: existing_record
                        .version
                        .clone()
                        .unwrap_or_else(|| app_version_key.clone()),
                    app_name: existing_record.app_name.clone(),
                    account_email: existing_record.account_email.clone(),
                    file_path: file_path.clone(),
                    file_size: existing_record
                        .file_size
                        .or_else(|| std::fs::metadata(&path).ok().map(|meta| meta.len() as i64)),
                    download_url: download_url.unwrap_or_default(),
                    install_url,
                    package_kind: decision.package_kind,
                    ota_installable: decision.ota_installable,
                    install_method: decision.install_method,
                    artwork_url: existing_record.artwork_url.clone(),
                    artist_name: existing_record.artist_name.clone(),
                    bundle_id: existing_record.bundle_id.clone(),
                    reused: true,
                    task_dir: task_dir.to_string_lossy().to_string(),
                };
                return HttpResponse::Ok().json(ApiResponse::success(response));
            }
        }
    }

    let job_id = Uuid::new_v4().to_string();
    eprintln!("[start-download-direct] job created: {}", job_id);
    let job = data.job_store.create_job(job_id.clone()).await;
    job.append_log(format!("[job] 已创建任务 {}", job_id)).await;
    job.append_log(format!("[job] 任务目录：{}", task_dir.display()))
        .await;

    let job_for_task = job.clone();
    let job_id_for_task = job_id.clone();
    let db = data.db.clone();
    let task_dir_for_job = task_dir.clone();

    tokio::spawn(async move {
        if let Err(error) = tokio::fs::create_dir_all(&task_dir_for_job).await {
            let message = format!("创建任务目录失败: {}", error);
            job_for_task
                .append_log(format!("[error] {}", message))
                .await;
            job_for_task.mark_failed(message).await;
            return;
        }
        remove_empty_legacy_job_dir(&job_root, &job_id_for_task).await;

        job_for_task.set_running().await;
        job_for_task
            .append_log("[job] 开始下载任务".to_string())
            .await;

        // Apply progress events in order on one task: spawning a task per event let a later
        // percentage land before an earlier one and interleave log lines.
        let (progress_tx, mut progress_rx) =
            tokio::sync::mpsc::unbounded_channel::<ipa_webtool_services::DownloadProgress>();
        let progress_job = job_for_task.clone();
        let progress_worker = tokio::spawn(async move {
            while let Some(progress) = progress_rx.recv().await {
                progress_job.append_log(progress.message.clone()).await;
                progress_job.update_from_progress(&progress).await;
            }
        });
        let progress_callback =
            std::sync::Arc::new(move |progress: ipa_webtool_services::DownloadProgress| {
                let _ = progress_tx.send(progress);
            });

        let download_path = task_dir_for_job.to_string_lossy().to_string();
        let params = DownloadParams {
            store: &account_store,
            email: &account_email,
            appid: &appid,
            app_ver_id: app_ver_id.as_deref(),
            download_path: &download_path,
            auto_purchase,
            token: None,
            progress_callback: Some(progress_callback),
        };

        let download_result = download_ipa_with_account(params).await;
        // The callback (and its sender) is dropped with `params`; wait until every queued
        // progress event is applied before recording the terminal state.
        let _ = progress_worker.await;

        match download_result {
            Ok(result) if result.ok => {
                let purchase_write = db
                    .lock()
                    .map_err(|error| format!("数据库锁错误: {}", error))
                    .and_then(|db| {
                        db.record_purchase(
                            &canonical_purchase_email(&account_email),
                            &appid,
                            "direct_download",
                        )
                        .map_err(|error| error.to_string())
                    });
                if let Err(error) = purchase_write {
                    job_for_task
                        .mark_failed(format!("购买记录数据库错误: {}", error))
                        .await;
                    return;
                }
                if let Some(file_path) = result.file {
                    job_for_task
                        .append_log(format!("[ready] 文件已就绪：{}", file_path))
                        .await;

                    let file_name = Path::new(&file_path)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or(&appid)
                        .to_string();
                    // Inspection and iTunesMetadata extraction read the ZIP: blocking pool.
                    let (file_meta, inspection, ipa_meta) = {
                        let file_path = file_path.clone();
                        tokio::task::spawn_blocking(move || {
                            let path = Path::new(&file_path);
                            (
                                std::fs::metadata(path).ok(),
                                inspect_existing_ipa(path),
                                // 从 IPA 提取完整元数据（iTunesMetadata.plist）
                                ipa_webtool_services::extract_itunes_metadata_from_ipa(path),
                            )
                        })
                        .await
                        .unwrap_or((None, None, None))
                    };
                    let decision = derive_delivery_decision(inspection.as_ref(), true);
                    let inspection_json = inspection
                        .as_ref()
                        .and_then(|value| serde_json::to_string(value).ok());
                    let meta = result.metadata.clone();

                    // 如果请求中没有 app_name 和 artwork_url，说明 App Store 查不到
                    let is_delisted = app_name_hint.is_none() && artwork_url_hint.is_none();

                    let record = DownloadRecord {
                        id: None,
                        job_id: Some(job_id_for_task.clone()),
                        app_version_id: app_ver_id.clone(),
                        // 优先级：下载metadata > iTunesMetadata(itemName中文名) > hint > 文件名
                        app_name: meta
                            .as_ref()
                            .map(|item| item.bundle_display_name.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| ipa_meta.as_ref().and_then(|m| m.item_name.clone()))
                            .or_else(|| app_name_hint.clone())
                            .or_else(|| {
                                ipa_meta
                                    .as_ref()
                                    .and_then(|m| m.bundle_display_name.clone())
                            })
                            .unwrap_or(file_name),
                        app_id: appid.clone(),
                        bundle_id: meta
                            .as_ref()
                            .map(|item| item.bundle_id.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| ipa_meta.as_ref().and_then(|m| m.bundle_id.clone()))
                            .or_else(|| bundle_id_hint.clone()),
                        version: meta
                            .as_ref()
                            .map(|item| item.bundle_short_version_string.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| app_version_hint.clone())
                            .or_else(|| app_ver_id.clone()),
                        account_email: account_email.clone(),
                        account_region: account_region.clone(),
                        download_date: Some(Utc::now().to_rfc3339()),
                        status: "completed".to_string(),
                        file_size: file_meta.map(|info| info.len() as i64),
                        file_path: Some(file_path.clone()),
                        install_url: None,
                        artwork_url: meta
                            .as_ref()
                            .map(|item| item.artwork_url.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| ipa_meta.as_ref().and_then(|m| m.icon_url.clone()))
                            .or_else(|| artwork_url_hint.clone()),
                        artist_name: meta
                            .as_ref()
                            .map(|item| item.artist_name.clone())
                            .filter(|value| !value.is_empty())
                            .or_else(|| ipa_meta.as_ref().and_then(|m| m.artist_name.clone()))
                            .or_else(|| artist_name_hint.clone()),
                        progress: Some(100),
                        error: None,
                        package_kind: Some(decision.package_kind),
                        ota_installable: Some(decision.ota_installable),
                        install_method: Some(decision.install_method),
                        inspection_json,
                        delisted: if is_delisted { Some(true) } else { None },
                        created_at: None,
                    };
                    if let Err(e) = db
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .add_download_record(&record)
                    {
                        eprintln!("[record] Failed to save download record: {}", e);
                    }

                    // Mark ready only once the record exists, so the job-info request that
                    // follows the "ready" event already sees the persisted record and its
                    // signed download/install links.
                    job_for_task
                        .mark_ready(file_path.clone(), result.metadata.clone(), None)
                        .await;

                    // WebDAV 自动上传（若已配置并启用）。
                    // 先在独立作用域内读取配置并释放 DB 锁，避免 MutexGuard 跨 .await。
                    let webdav_target = {
                        let db_guard = db.lock().unwrap_or_else(|e| e.into_inner());
                        db_guard
                            .get_webdav_config()
                            .ok()
                            .flatten()
                            .filter(|c| c.enabled)
                            .and_then(|c| ipa_webtool_services::webdav::target_from_config(&c))
                    };
                    if let Some(target) = webdav_target {
                        job_for_task
                            .append_log("[webdav] 开始上传到 WebDAV…".to_string())
                            .await;
                        match ipa_webtool_services::webdav::upload_file(
                            &target,
                            Path::new(&file_path),
                        )
                        .await
                        {
                            Ok(remote_url) => {
                                job_for_task
                                    .append_log(format!("[webdav] 上传完成: {}", remote_url))
                                    .await;
                            }
                            Err(e) => {
                                job_for_task
                                    .append_log(format!("[webdav] 上传失败: {}", e))
                                    .await;
                                log::warn!("[webdav] upload failed: {}", e);
                            }
                        }
                    }
                } else {
                    let message = "下载完成，但未找到产物文件".to_string();
                    job_for_task
                        .append_log(format!("[error] {}", message))
                        .await;
                    job_for_task.mark_failed(message).await;
                }
            }
            Ok(result) => {
                let message = result.error.unwrap_or_else(|| "下载失败".to_string());
                job_for_task
                    .append_log(format!("[error] {}", message))
                    .await;
                job_for_task.mark_failed(message).await;
            }
            Err(error) => {
                let message = error.to_string();
                job_for_task
                    .append_log(format!("[error] {}", message))
                    .await;
                job_for_task.mark_failed(message).await;
            }
        }
    });

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
        "jobId": job_id,
        "taskDir": task_dir.to_string_lossy().to_string(),
        "reused": false
    })))
}

async fn progress_sse(
    query: web::Query<JobIdQuery>,
    data: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let job = data
        .job_store
        .get(&query.jobId)
        .await
        .ok_or_else(|| ErrorNotFound("任务不存在"))?;

    // Subscribe before taking the snapshot: if the job finished in between, the "end" event
    // would otherwise be missed and the stream would hang until the job is evicted.
    let receiver = job.subscribe();
    let snapshot = job.snapshot().await;
    let mut initial_events: Vec<Result<Bytes, Error>> = Vec::new();

    for line in &snapshot.logs {
        initial_events.push(encode_sse("log", &JobLogEvent { line: line.clone() }));
    }

    initial_events.push(encode_sse("progress", &snapshot_progress_event(&snapshot)));

    if snapshot.status == "ready" || snapshot.status == "failed" {
        initial_events.push(encode_sse(
            "end",
            &JobEndEvent {
                status: snapshot.status.clone(),
                error: snapshot.error.clone(),
            },
        ));

        return Ok(HttpResponse::Ok()
            .insert_header(("Content-Type", "text/event-stream"))
            .insert_header(("Cache-Control", "no-cache"))
            .insert_header(("X-Accel-Buffering", "no"))
            .streaming(stream::iter(initial_events)));
    }

    let live_stream = stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(event) => return Some((encode_job_event(event), receiver)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Ok(HttpResponse::Ok()
        .insert_header(("Content-Type", "text/event-stream"))
        .insert_header(("Cache-Control", "no-cache"))
        .insert_header(("X-Accel-Buffering", "no"))
        .streaming(stream::iter(initial_events).chain(live_stream)))
}

async fn download_file(
    query: web::Query<JobIdQuery>,
    data: web::Data<AppState>,
) -> Result<fs::NamedFile, Error> {
    let file_path = if let Some(job) = data.job_store.get(&query.jobId).await {
        let snapshot = job.snapshot().await;
        if snapshot.status != "ready" {
            return Err(ErrorNotFound("任务尚未就绪"));
        }
        snapshot
            .file_path
            .clone()
            .ok_or_else(|| ErrorNotFound("下载文件不存在"))?
    } else {
        let record = data
            .db
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_download_record_by_job_id(&query.jobId)
            .map_err(ErrorInternalServerError)?
            .ok_or_else(|| ErrorNotFound("任务不存在"))?;
        record
            .file_path
            .clone()
            .ok_or_else(|| ErrorNotFound("下载文件不存在"))?
    };

    let path = PathBuf::from(&file_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.ipa")
        .to_string();

    let file = fs::NamedFile::open_async(path)
        .await
        .map_err(ErrorNotFound)?
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(file_name)],
        });

    Ok(file)
}

async fn get_job_info(
    req: HttpRequest,
    query: web::Query<JobIdQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let job = match data.job_store.get(&query.jobId).await {
        Some(job) => job,
        None => {
            let persisted_record = match data
                .db
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_download_record_by_job_id(&query.jobId)
            {
                Ok(Some(record)) => record,
                Ok(None) => {
                    return HttpResponse::NotFound()
                        .json(ApiResponse::<String>::error("任务不存在".to_string()))
                }
                Err(error) => {
                    return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                        format!("读取任务记录失败：{error}"),
                    ))
                }
            };

            let file_exists = persisted_record
                .file_path
                .as_ref()
                .map(|path| PathBuf::from(path).exists())
                .unwrap_or(false);
            let inspection = inspection_for_record(&persisted_record);
            sync_record_delivery(
                &data.db.lock().unwrap_or_else(|e| e.into_inner()),
                &persisted_record,
                inspection.as_ref(),
                file_exists,
            );
            let decision = derive_delivery_decision(inspection.as_ref(), file_exists);
            let file_size = persisted_record
                .file_path
                .as_ref()
                .and_then(|path| std::fs::metadata(path).ok())
                .map(|meta| meta.len());
            let install_url = if decision.ota_installable {
                persisted_record.id.and_then(|record_id| {
                    build_record_install_url(&build_base_url(&req), &persisted_record, record_id)
                })
            } else {
                None
            };
            let download_url = persisted_record.id.and_then(|record_id| {
                if file_exists {
                    Some(build_record_download_url(&build_base_url(&req), record_id))
                } else {
                    None
                }
            });

            return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                 "jobId": query.jobId.clone(),
                 "status": if file_exists { "ready" } else { "failed" },
                 "stage": if file_exists { "done" } else { "missing-file" },
                 "progress": if file_exists { 100 } else { persisted_record.progress.unwrap_or(0) },
                 "downloadUrl": download_url,
                 "installUrl": install_url,
                 "packageKind": decision.package_kind,
                 "otaInstallable": decision.ota_installable,
                 "installMethod": decision.install_method,
                 "inspection": inspection,
                 "error": if file_exists { serde_json::Value::Null } else { serde_json::Value::String("任务记录存在，但安装包文件已丢失".to_string()) },
                 "metadata": serde_json::Value::Null,
                 "filePath": persisted_record.file_path,
                 "fileSize": file_size,
             })));
        }
    };

    let snapshot = job.snapshot().await;

    let persisted_record = if snapshot.status == "ready" {
        data.db
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_download_record_by_job_id(&query.jobId)
            .ok()
            .flatten()
    } else {
        None
    };

    // job-info is polled every ~1.5s per task: only open the IPA when no persisted record
    // (which caches the inspection in the DB) is available for a finished job.
    let snapshot_inspection = if persisted_record.is_none() && snapshot.status == "ready" {
        snapshot
            .file_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .as_deref()
            .and_then(inspect_existing_ipa)
    } else {
        None
    };
    let snapshot_file_exists = snapshot
        .file_path
        .as_ref()
        .map(PathBuf::from)
        .map(|path| path.exists())
        .unwrap_or(false);
    let persisted_record_inspection = persisted_record.as_ref().and_then(inspection_for_record);
    if let Some(record) = persisted_record.as_ref() {
        if snapshot.status == "ready" {
            if let Ok(db) = data.db.lock() {
                sync_record_delivery(
                    &db,
                    record,
                    persisted_record_inspection.as_ref(),
                    snapshot_file_exists,
                );
            }
        }
    }
    let snapshot_decision = if persisted_record.is_some() {
        derive_delivery_decision(persisted_record_inspection.as_ref(), snapshot_file_exists)
    } else {
        derive_delivery_decision(snapshot_inspection.as_ref(), snapshot_file_exists)
    };
    let install_url = if snapshot.status == "ready" {
        if !snapshot_decision.ota_installable {
            None
        } else if let Some(record) = persisted_record.as_ref() {
            record.id.and_then(|record_id| {
                build_record_install_url(&build_base_url(&req), record, record_id)
            })
        } else if snapshot.file_path.is_some() {
            Some(build_job_install_url(&req, &query.jobId))
        } else {
            None
        }
    } else {
        snapshot.install_url.clone()
    };
    let download_url = if snapshot.status == "ready" {
        if let Some(record) = persisted_record.as_ref() {
            if let Some(record_id) = record.id {
                Some(build_record_download_url(&build_base_url(&req), record_id))
            } else {
                Some(format!(
                    "{}/api/public/download-file?jobId={}",
                    build_base_url(&req),
                    urlencoding::encode(&query.jobId)
                ))
            }
        } else {
            Some(format!(
                "{}/api/public/download-file?jobId={}",
                build_base_url(&req),
                urlencoding::encode(&query.jobId)
            ))
        }
    } else {
        None
    };
    let file_size = snapshot
        .file_path
        .as_ref()
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len());

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
         "jobId": snapshot.job_id,
         "status": snapshot.status,
         "stage": snapshot.stage,
         "progress": snapshot.progress,
         "downloadUrl": download_url,
         "installUrl": install_url,
         "packageKind": snapshot_decision.package_kind,
         "otaInstallable": snapshot_decision.ota_installable,
         "installMethod": snapshot_decision.install_method,
         "inspection": if persisted_record.is_some() { persisted_record_inspection } else { snapshot_inspection },
         "error": snapshot.error,
         "metadata": snapshot.metadata,
         "filePath": snapshot.file_path,
         "fileSize": file_size,
     })))
}

// 获取下载链接
async fn get_download_url(
    query: web::Query<DownloadUrlQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let account_store = accounts.get(&query.token);

    if account_store.is_none() {
        return HttpResponse::Unauthorized()
            .json(ApiResponse::<String>::error("无效的 token".to_string()));
    }

    let account_store = RecoveryStore::new(&query.token, &account_store.unwrap().clone(), &data);
    drop(accounts);

    // 调用 download_product
    match account_store
        .download_product(&query.appid, query.appVerId.as_deref())
        .await
    {
        Ok(result) => {
            let state = result
                .get("_state")
                .and_then(|v| v.as_str())
                .unwrap_or("failure");

            if state == "success" {
                if let Err(error) = persist_purchase(
                    &data,
                    &account_store.account_email,
                    &query.appid,
                    "download_url",
                ) {
                    return purchase_database_error(error);
                }
                // 提取下载链接
                if let Some(song_list) = result.get("songList").and_then(|sl| sl.as_array()) {
                    if let Some(first_song) = song_list.first() {
                        if let Some(url) = first_song.get("URL").and_then(|u| u.as_str()) {
                            // 提取元数据
                            let metadata = first_song.get("metadata").and_then(|m| m.as_object());

                            return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                                 "url": url,
                                 "fileName": canonical_ipa_filename(
                                     metadata.and_then(|m| m.get("bundleDisplayName")).and_then(|v| v.as_str()).unwrap_or("app"),
                                     metadata.and_then(|m| m.get("bundleShortVersionString")).and_then(|v| v.as_str()).unwrap_or("1.0.0"),
                                     metadata.and_then(|m| m.get("bundleId")).and_then(|v| v.as_str())
                                 ),
                                 "metadata": {
                                     "bundle_display_name": metadata.and_then(|m| m.get("bundleDisplayName")).and_then(|v| v.as_str()).unwrap_or(""),
                                     "bundle_short_version_string": metadata.and_then(|m| m.get("bundleShortVersionString")).and_then(|v| v.as_str()).unwrap_or(""),
                                     "bundle_id": metadata.and_then(|m| m.get("bundleId")).and_then(|v| v.as_str()).unwrap_or(""),
                                     "artwork_url": metadata.and_then(|m| m.get("artworkUrl")).and_then(|v| v.as_str()).unwrap_or(""),
                                     "artist_name": metadata.and_then(|m| m.get("artistName")).and_then(|v| v.as_str()).unwrap_or(""),
                                 }
                             })));
                        }
                    }
                }

                let error_msg = result
                    .get("customerMessage")
                    .or(result.get("failureType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("无法获取下载链接");

                HttpResponse::BadRequest().json(ApiResponse::<String>::error(error_msg.to_string()))
            } else {
                // 检查是否需要购买
                let error_msg = result
                    .get("customerMessage")
                    .or(result.get("failureType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("下载失败");

                let is_license_error = purchase_required(error_msg);

                if is_license_error {
                    HttpResponse::BadRequest().json(serde_json::json!({
                        "ok": false,
                        "needsPurchase": true,
                        "error": error_msg
                    }))
                } else {
                    HttpResponse::BadRequest()
                        .json(ApiResponse::<String>::error(error_msg.to_string()))
                }
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "获取下载链接失败: {}",
            e
        ))),
    }
}

async fn claim_app(req: web::Json<ClaimRequest>, data: web::Data<AppState>) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let account_store = match accounts.get(&req.token) {
        Some(account) => account,
        None => {
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("无效的 token".to_string()))
        }
    };
    let account_store = RecoveryStore::new(&req.token, &account_store.clone(), &data);
    drop(accounts);

    match account_store
        .ensure_license(&req.appid, req.appVerId.as_deref())
        .await
    {
        Ok(result) => {
            let state = result
                .get("_state")
                .and_then(|v| v.as_str())
                .unwrap_or("failure");

            if state != "success" {
                let error_msg = get_license_error_message(&result);
                return HttpResponse::BadRequest().json(ApiResponse::<String>::error(error_msg));
            }

            let verify_delays_ms = [0_u64, 1200, 2500, 4000];
            let mut last_verify_error = String::new();

            for (attempt, delay_ms) in verify_delays_ms.iter().enumerate() {
                if *delay_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(*delay_ms)).await;
                }

                match account_store
                    .download_product(&req.appid, req.appVerId.as_deref())
                    .await
                {
                    Ok(verify_result) => {
                        let verify_state = verify_result
                            .get("_state")
                            .and_then(|v| v.as_str())
                            .unwrap_or("failure");

                        if verify_state == "success" {
                            // Record the purchase in DB
                            let account_id = canonical_purchase_email(&account_store.account_email);
                            {
                                if let Err(error) =
                                    persist_purchase(&data, &account_id, &req.appid, "claim")
                                {
                                    return purchase_database_error(error);
                                }
                            }
                            // Update memory cache
                            {
                                let mut cache = PURCHASE_CACHE.write().await;
                                evict_expired_purchase_cache_locked(&mut cache);
                                cache.insert(
                                    (account_id, req.appid.clone()),
                                    PurchaseCacheEntry {
                                        purchased: true,
                                        needs_purchase: false,
                                        error: None,
                                        cached_at: Instant::now(),
                                    },
                                );
                            }
                            return HttpResponse::Ok().json(ApiResponse::success(
                                serde_json::json!({
                                    "claimed": true,
                                    "verified": true,
                                    "verifyAttempts": attempt + 1,
                                }),
                            ));
                        }

                        let verify_error = verify_result
                            .get("customerMessage")
                            .or(verify_result.get("failureType"))
                            .or(verify_result.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim()
                            .to_string();

                        let retryable = {
                            let lowered = verify_error.to_lowercase();
                            lowered.contains("license not found")
                                || lowered.contains("not purchased")
                                || lowered.contains("not found")
                                || verify_error.contains("未购买")
                                || verify_error.contains("未领取")
                                || verify_error.contains("未找到")
                        };

                        last_verify_error = verify_error;
                        if !retryable {
                            break;
                        }
                    }
                    Err(e) => {
                        return HttpResponse::InternalServerError().json(
                            ApiResponse::<String>::error(format!("领取后校验失败: {}", e)),
                        );
                    }
                }
            }

            let error_msg = if last_verify_error.is_empty() {
                "Apple 尚未确认领取成功，请稍后刷新或重试".to_string()
            } else {
                format!("Apple 正在同步领取状态，请稍后重试：{}", last_verify_error)
            };

            HttpResponse::BadRequest().json(ApiResponse::<String>::error(error_msg))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("领取失败: {}", e))),
    }
}

async fn get_purchase_status(
    query: web::Query<PurchaseStatusQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let account_store = match accounts.get(&query.token) {
        Some(account) => account,
        None => {
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("无效的 token".to_string()))
        }
    };
    let account_store = RecoveryStore::new(&query.token, &account_store.clone(), &data);
    drop(accounts);
    let account_id = canonical_purchase_email(&account_store.account_email);
    let appid = query.appid.clone();

    // L1: Check DB for known purchased apps
    {
        let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
        let purchased = match db.is_purchased(&account_id, &appid) {
            Ok(purchased) => purchased,
            Err(error) => return purchase_database_error(error),
        };
        if purchased {
            return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "purchased": true,
                "needsPurchase": false,
                "status": "owned",
                "error": null,
            })));
        }
    }

    // L2: Check in-memory cache
    {
        let cache = PURCHASE_CACHE.read().await;
        if let Some(entry) = cache.get(&(account_id.clone(), appid.clone())) {
            if entry.cached_at.elapsed().as_secs() < PURCHASE_CACHE_TTL_SECS {
                return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                     "purchased": entry.purchased,
                     "needsPurchase": entry.needs_purchase,
                     "status": if entry.purchased { "owned" } else if entry.needs_purchase { "not_owned" } else { "unknown" },
                     "error": entry.error,
                     "requiresLogin": entry.error.as_deref().is_some_and(purchase_requires_login),
                 })));
            }
        }
    }

    // L3: Call Apple API
    match account_store
        .download_product(&query.appid, query.appVerId.as_deref())
        .await
    {
        Ok(result) => {
            let state = result
                .get("_state")
                .and_then(|v| v.as_str())
                .unwrap_or("failure");

            if state == "success" {
                // Write to DB (permanent record)
                if let Err(error) = persist_purchase(&data, &account_id, &appid, "apple_api") {
                    return purchase_database_error(error);
                }
                // Update memory cache
                {
                    let mut cache = PURCHASE_CACHE.write().await;
                    evict_expired_purchase_cache_locked(&mut cache);
                    cache.insert(
                        (account_id, appid),
                        PurchaseCacheEntry {
                            purchased: true,
                            needs_purchase: false,
                            error: None,
                            cached_at: Instant::now(),
                        },
                    );
                }
                return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "purchased": true,
                    "needsPurchase": false,
                    "status": "owned",
                    "error": null,
                })));
            }

            let error_msg = result
                .get("customerMessage")
                .or(result.get("failureType"))
                .or(result.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("下载失败")
                .to_string();

            let is_license_error = purchase_required(&error_msg);

            // Update memory cache (not purchased, with TTL)
            {
                let mut cache = PURCHASE_CACHE.write().await;
                evict_expired_purchase_cache_locked(&mut cache);
                cache.insert(
                    (account_id.clone(), appid.clone()),
                    PurchaseCacheEntry {
                        purchased: false,
                        needs_purchase: is_license_error,
                        error: purchase_failure_error(&error_msg),
                        cached_at: Instant::now(),
                    },
                );
            }

            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "purchased": false,
                "needsPurchase": is_license_error,
                "status": if is_license_error { "not_owned" } else { "unknown" },
                "requiresLogin": purchase_requires_login(&error_msg),
                "error": purchase_failure_error(&error_msg),
            })))
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "查询购买状态失败: {}",
            e
        ))),
    }
}

// Batch purchase status check
async fn purchase_status_batch(
    req: web::Json<PurchaseStatusBatchRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let account_store = match accounts.get(&req.token) {
        Some(account) => account,
        None => {
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("无效的 token".to_string()))
        }
    };
    let account_store = RecoveryStore::new(&req.token, &account_store.clone(), &data);
    drop(accounts);
    let account_id = canonical_purchase_email(&account_store.account_email);

    if req.appids.is_empty() {
        return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "results": {}
        })));
    }

    let mut results = serde_json::Map::new();

    // L1: Batch check DB
    let db_purchased: std::collections::HashSet<String> = {
        let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
        match db.batch_check_purchased(&account_id, &req.appids) {
            Ok(purchased) => purchased,
            Err(error) => return purchase_database_error(error),
        }
    };

    let mut need_check: Vec<String> = Vec::new();

    for appid in &req.appids {
        if db_purchased.contains(appid) {
            results.insert(
                appid.clone(),
                serde_json::json!({
                    "purchased": true,
                    "needsPurchase": false,
                    "status": "owned",
                    "error": null,
                }),
            );
        } else {
            need_check.push(appid.clone());
        }
    }

    if need_check.is_empty() {
        return HttpResponse::Ok().json(ApiResponse::success(
            serde_json::json!({ "results": results }),
        ));
    }

    // L2: Check in-memory cache for remaining
    let mut still_need_apple: Vec<String> = Vec::new();
    {
        let cache = PURCHASE_CACHE.read().await;
        for appid in &need_check {
            if let Some(entry) = cache.get(&(account_id.clone(), appid.clone())) {
                if entry.cached_at.elapsed().as_secs() < PURCHASE_CACHE_TTL_SECS {
                    results.insert(appid.clone(), serde_json::json!({
                         "purchased": entry.purchased,
                         "needsPurchase": entry.needs_purchase,
                         "status": if entry.purchased { "owned" } else if entry.needs_purchase { "not_owned" } else { "unknown" },
                         "error": entry.error,
                     "requiresLogin": entry.error.as_deref().is_some_and(purchase_requires_login),
                     }));
                    continue;
                }
            }
            still_need_apple.push(appid.clone());
        }
    }

    if still_need_apple.is_empty() {
        return HttpResponse::Ok().json(ApiResponse::success(
            serde_json::json!({ "results": results }),
        ));
    }

    // L3: Call Apple API for remaining apps concurrently
    let apple_futures: Vec<_> = still_need_apple
        .iter()
        .map(|appid| {
            let appid = appid.clone();
            async {
                let result = account_store.download_product(&appid, None).await;
                (appid, result)
            }
        })
        .collect();

    let apple_results = futures::future::join_all(apple_futures).await;

    // Process Apple results
    {
        let mut cache = PURCHASE_CACHE.write().await;
        evict_expired_purchase_cache_locked(&mut cache);
        for (appid, apple_result) in apple_results {
            match apple_result {
                Ok(result) => {
                    let state = result
                        .get("_state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("failure");
                    if state == "success" {
                        // Write to DB
                        if let Err(error) =
                            persist_purchase(&data, &account_id, &appid, "apple_api")
                        {
                            return purchase_database_error(error);
                        }
                        cache.insert(
                            (account_id.clone(), appid.clone()),
                            PurchaseCacheEntry {
                                purchased: true,
                                needs_purchase: false,
                                error: None,
                                cached_at: Instant::now(),
                            },
                        );
                        results.insert(
                            appid,
                            serde_json::json!({
                                "purchased": true,
                                "needsPurchase": false,
                                "status": "owned",
                                "error": null,
                            }),
                        );
                    } else {
                        let error_msg = result
                            .get("customerMessage")
                            .or(result.get("failureType"))
                            .or(result.get("message"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("下载失败")
                            .to_string();
                        let is_license_error = purchase_required(&error_msg);
                        cache.insert(
                            (account_id.clone(), appid.clone()),
                            PurchaseCacheEntry {
                                purchased: false,
                                needs_purchase: is_license_error,
                                error: purchase_failure_error(&error_msg),
                                cached_at: Instant::now(),
                            },
                        );
                        results.insert(
                            appid,
                            serde_json::json!({
                                "purchased": false,
                                "needsPurchase": is_license_error,
                                "status": if is_license_error { "not_owned" } else { "unknown" },
                                "requiresLogin": purchase_requires_login(&error_msg),
                "error": purchase_failure_error(&error_msg),
                            }),
                        );
                    }
                }
                Err(e) => {
                    results.insert(
                        appid,
                        serde_json::json!({
                            "purchased": false,
                            "needsPurchase": false,
                            "status": "error",
                            "error": format!("查询失败: {}", e),
                        }),
                    );
                }
            }
        }
    }

    HttpResponse::Ok().json(ApiResponse::success(
        serde_json::json!({ "results": results }),
    ))
}

// Confirm purchase - force check Apple API
async fn confirm_purchase(
    req: web::Json<ConfirmPurchaseRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let account_store = match accounts.get(&req.token) {
        Some(account) => account,
        None => {
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("无效的 token".to_string()))
        }
    };
    let account_store = RecoveryStore::new(&req.token, &account_store.clone(), &data);
    drop(accounts);
    let account_id = canonical_purchase_email(&account_store.account_email);

    // Force call Apple API - ignore cache TTL
    match account_store.download_product(&req.appid, None).await {
        Ok(result) => {
            let state = result
                .get("_state")
                .and_then(|v| v.as_str())
                .unwrap_or("failure");

            if state == "success" {
                // Write to DB (permanent)
                {
                    if let Err(error) =
                        persist_purchase(&data, &account_id, &req.appid, "paid_confirm")
                    {
                        return purchase_database_error(error);
                    }
                }
                // Update memory cache
                {
                    let mut cache = PURCHASE_CACHE.write().await;
                    evict_expired_purchase_cache_locked(&mut cache);
                    cache.insert(
                        (account_id.clone(), req.appid.clone()),
                        PurchaseCacheEntry {
                            purchased: true,
                            needs_purchase: false,
                            error: None,
                            cached_at: Instant::now(),
                        },
                    );
                }
                return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "purchased": true,
                    "needsPurchase": false,
                    "status": "owned",
                    "error": null,
                    "message": "已确认购买",
                })));
            }

            let error_msg = result
                .get("customerMessage")
                .or(result.get("failureType"))
                .or(result.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("下载失败")
                .to_string();

            let is_license_error = purchase_required(&error_msg);

            // Update memory cache even for not-purchased result
            {
                let mut cache = PURCHASE_CACHE.write().await;
                evict_expired_purchase_cache_locked(&mut cache);
                cache.insert(
                    (account_id.clone(), req.appid.clone()),
                    PurchaseCacheEntry {
                        purchased: false,
                        needs_purchase: is_license_error,
                        error: purchase_failure_error(&error_msg),
                        cached_at: Instant::now(),
                    },
                );
            }

            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "purchased": false,
                "needsPurchase": is_license_error,
                "status": if is_license_error { "not_owned" } else { "unknown" },
                "requiresLogin": purchase_requires_login(&error_msg),
                "error": purchase_failure_error(&error_msg),
                "message": "未检测到购买记录，请确认购买已完成",
            })))
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "确认购买状态失败: {}",
            e
        ))),
    }
}

fn sanitize_upload_filename(name: &str) -> String {
    let base = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit('\\')
        .next()
        .unwrap_or(name);

    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty() {
        "upload.ipa".to_string()
    } else {
        cleaned
    }
}

// 上传 IPA（手动上传到服务器）
async fn upload_ipa(mut payload: Multipart, data: web::Data<AppState>) -> impl Responder {
    const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;

    let job_id = Uuid::new_v4().to_string();
    let uploads_dir = data.downloads_dir.join("uploads");
    if let Err(e) = tokio::fs::create_dir_all(&uploads_dir).await {
        return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "创建上传目录失败: {}",
            e
        )));
    }

    let mut saved_file_name: Option<String> = None;
    let mut saved_file_path: Option<PathBuf> = None;
    let mut saved_file_size: u64 = 0;

    while let Ok(Some(mut field)) = payload.try_next().await {
        let filename = field
            .content_disposition()
            .get_filename()
            .map(sanitize_upload_filename)
            .unwrap_or_else(|| "upload.ipa".to_string());

        if !filename.to_lowercase().ends_with(".ipa") {
            return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                "只能上传 .ipa 文件".to_string(),
            ));
        }

        // Only accept the first file field
        if saved_file_path.is_some() {
            continue;
        }

        let target_path = uploads_dir.join(format!("{}-{}", job_id, filename));
        let mut f = match tokio::fs::File::create(&target_path).await {
            Ok(file) => file,
            Err(e) => {
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("创建上传文件失败: {}", e),
                ));
            }
        };

        loop {
            let chunk = match field.try_next().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(e) => {
                    // A broken upload stream must not leave a truncated IPA recorded as complete.
                    let _ = tokio::fs::remove_file(&target_path).await;
                    return HttpResponse::BadRequest()
                        .json(ApiResponse::<String>::error(format!("上传中断: {}", e)));
                }
            };
            saved_file_size = saved_file_size.saturating_add(chunk.len() as u64);
            if saved_file_size > MAX_UPLOAD_BYTES {
                let _ = tokio::fs::remove_file(&target_path).await;
                return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                    "上传文件不能超过 2GB".to_string(),
                ));
            }
            if let Err(e) = f.write_all(&chunk).await {
                let _ = tokio::fs::remove_file(&target_path).await;
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("写入上传文件失败: {}", e),
                ));
            }
        }
        if let Err(e) = f.flush().await {
            let _ = tokio::fs::remove_file(&target_path).await;
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                format!("写入上传文件失败: {}", e),
            ));
        }

        saved_file_name = Some(filename);
        saved_file_path = Some(target_path);
    }

    let Some(file_path) = saved_file_path else {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<String>::error("未找到上传文件".to_string()));
    };

    let file_name = saved_file_name.unwrap_or_else(|| "upload.ipa".to_string());
    let absolute_path = file_path.to_string_lossy().to_string();

    let inspection = {
        let path = file_path.clone();
        web::block(move || inspect_existing_ipa(&path))
            .await
            .ok()
            .flatten()
    };
    if let Ok(db) = data.db.lock() {
        let decision = derive_delivery_decision(inspection.as_ref(), true);
        let inspection_json = inspection
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok());
        let record = DownloadRecord {
            id: None,
            job_id: Some(job_id.clone()),
            app_version_id: None,
            app_name: file_name.trim_end_matches(".ipa").to_string(),
            app_id: "uploaded".to_string(),
            bundle_id: None,
            version: None,
            account_email: "手动上传".to_string(),
            account_region: None,
            download_date: Some(Utc::now().to_rfc3339()),
            status: "completed".to_string(),
            file_size: Some(saved_file_size as i64),
            file_path: Some(absolute_path.clone()),
            install_url: None,
            artwork_url: None,
            artist_name: None,
            progress: Some(100),
            error: None,
            package_kind: Some(decision.package_kind),
            ota_installable: Some(decision.ota_installable),
            install_method: Some(decision.install_method),
            inspection_json,
            created_at: None,
            delisted: None,
        };
        let _ = db.add_download_record(&record);
    }

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
        "jobId": job_id,
        "fileName": file_name,
        "fileSize": saved_file_size,
        "filePath": absolute_path,
        "installUrl": null,
    })))
}

fn extract_itunes_track_id(app: &serde_json::Value) -> String {
    app.get("trackId")
        .and_then(|v| v.as_i64().map(|n| n.to_string()))
        .or_else(|| {
            app.get("trackId")
                .and_then(|v| v.as_u64().map(|n| n.to_string()))
        })
        .or_else(|| {
            app.get("trackId")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| {
            app.get("id")
                .and_then(|v| v.as_i64().map(|n| n.to_string()))
        })
        .or_else(|| {
            app.get("id")
                .and_then(|v| v.as_u64().map(|n| n.to_string()))
        })
        .or_else(|| {
            app.get("id")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
        .or_else(|| {
            app.get("trackViewUrl")
                .and_then(|v| v.as_str())
                .and_then(|url| {
                    url.split("/id").nth(1).map(|rest| {
                        rest.chars()
                            .take_while(|c| c.is_ascii_digit())
                            .collect::<String>()
                    })
                })
                .filter(|id| !id.is_empty())
        })
        .unwrap_or_default()
}

fn format_itunes_app(app: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "trackId": extract_itunes_track_id(app),
        "trackName": app.get("trackName").and_then(|v| v.as_str()).unwrap_or(""),
        "bundleId": app.get("bundleId").and_then(|v| v.as_str()).unwrap_or(""),
        "artistName": app.get("artistName").and_then(|v| v.as_str()).unwrap_or(""),
        "artworkUrl60": app.get("artworkUrl60").and_then(|v| v.as_str()).unwrap_or(""),
        "artworkUrl100": app.get("artworkUrl100").and_then(|v| v.as_str()).unwrap_or(""),
        "version": app.get("version").and_then(|v| v.as_str()).unwrap_or(""),
        "averageUserRating": app.get("averageUserRating").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "price": app.get("price").and_then(|v| v.as_f64()).unwrap_or(0.0),
        "formattedPrice": app.get("formattedPrice").and_then(|v| v.as_str()).unwrap_or(""),
        "fileSizeBytes": app.get("fileSizeBytes")
            .map(|v| parse_json_i64(Some(v)).unwrap_or(0) as u64)
            .unwrap_or(0),
        "genres": app.get("genres").and_then(|v| v.as_array()).cloned().unwrap_or(vec![]),
    })
}

async fn app_meta(query: web::Query<AppMetaQuery>) -> impl Responder {
    let region = store_region_or(query.region.as_deref(), "US");
    let url = format!(
        "https://itunes.apple.com/lookup?id={}&country={}",
        urlencoding::encode(query.appid.trim()),
        region
    );

    match build_http_client().get(&url).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    "lookup API 返回错误".to_string(),
                ));
            }

            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    if let Some(app) = json
                        .get("results")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                    {
                        HttpResponse::Ok().json(ApiResponse::success(format_itunes_app(app)))
                    } else {
                        HttpResponse::NotFound()
                            .json(ApiResponse::<String>::error("未找到应用元数据".to_string()))
                    }
                }
                Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("解析应用元数据失败: {}", e),
                )),
            }
        }
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "获取应用元数据失败: {}",
            e
        ))),
    }
}

// 搜索应用
async fn search_app(
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let term = match query.get("term") {
        Some(t) => t.as_str(),
        None => "",
    };
    let region = match query.get("region") {
        Some(r) => r.as_str(),
        None => "US",
    };
    let media = match query.get("media") {
        Some(m) => m.as_str(),
        None => "software",
    };
    let limit = match query.get("limit") {
        Some(l) => l.as_str(),
        None => "25",
    };

    if term.is_empty() {
        return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
            "搜索关键词不能为空".to_string(),
        ));
    }

    // 调用 Apple Search API
    let url = format!(
        "https://itunes.apple.com/search?term={}&country={}&media={}&limit={}",
        urlencoding::encode(term),
        urlencoding::encode(region),
        urlencoding::encode(media),
        urlencoding::encode(limit)
    );

    let client = build_http_client();
    match client.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.json::<serde_json::Value>().await {
                    Ok(json) => {
                        if let Some(results) = json.get("resultCount").and_then(|v| v.as_u64()) {
                            if results > 0 {
                                if let Some(apps) = json.get("results").and_then(|v| v.as_array()) {
                                    // 转换为我们的格式
                                    let formatted_apps: Vec<serde_json::Value> =
                                        apps.iter().map(format_itunes_app).collect();

                                    return HttpResponse::Ok()
                                        .json(ApiResponse::success(formatted_apps));
                                }
                            }
                        }

                        // 没有找到结果
                        HttpResponse::Ok().json(ApiResponse::<Vec<Value>>::success(vec![]))
                    }
                    Err(e) => {
                        log::error!("解析搜索结果失败: {}", e);
                        HttpResponse::InternalServerError()
                            .json(ApiResponse::<String>::error("解析搜索结果失败".to_string()))
                    }
                }
            } else {
                log::error!("搜索 API 返回错误: {}", response.status());
                HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    "搜索 API 返回错误".to_string(),
                ))
            }
        }
        Err(e) => {
            log::error!("搜索请求失败: {}", e);
            HttpResponse::InternalServerError()
                .json(ApiResponse::<String>::error(format!("搜索请求失败: {}", e)))
        }
    }
}

// 登录
async fn apple_login(
    req: web::Json<AppleLoginRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let mfa_code = normalize_mfa_code(req.mfa.as_deref());
    let has_mfa = mfa_code.is_some();
    log::info!(
        "Apple login attempt: email={}, has_mfa={}, mfa_len={}",
        req.email,
        has_mfa,
        mfa_code.as_ref().map(|s| s.len()).unwrap_or(0)
    );

    // 密码留空时使用已保存的密码（重新登录只需输入验证码）
    let password = if req.password.is_empty() {
        match data
            .db
            .lock()
            .ok()
            .and_then(|db| load_saved_password(&db, &req.email))
        {
            Some(password) => password,
            None => {
                return HttpResponse::Ok().json(ApiResponse::<String>::error(
                    "该账号没有保存的密码，请输入密码".to_string(),
                ))
            }
        }
    } else {
        req.password.clone()
    };

    // 如果有 MFA code，优先复用第一轮暂存的 AccountStore（保留 GUID）
    // 否则创建新的
    let mut account_store = if has_mfa {
        match take_pending_mfa(&req.email, &password).await {
            Ok(store) => {
                log::info!("Reusing pending MFA session for {}", req.email);
                store
            }
            Err(message) => {
                log::warn!(
                    "Pending MFA session unavailable for {}: {}",
                    req.email,
                    message
                );
                return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "status": "need_restart",
                    "message": message,
                })));
            }
        }
    } else {
        AccountStore::new(&req.email)
    };

    match account_store
        .authenticate(&password, mfa_code.as_deref())
        .await
    {
        Ok(result) => {
            let state = result
                .get("_state")
                .and_then(|v| v.as_str())
                .unwrap_or("failure");

            log::info!(
                "Apple auth result: state={}, keys={:?}",
                state,
                result.keys().take(10).collect::<Vec<_>>()
            );

            if state == "success" {
                // 清理可能残留的 pending MFA 条目
                clear_pending_mfa(&req.email).await;

                // 生成 token
                let token = uuid::Uuid::new_v4().to_string();
                let dsid = result
                    .get("dsPersonId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut region_for_response = resolved_account_region(&result, None);

                // 持久化账号到 DB
                if let Ok(db) = data.db.lock() {
                    let existing_region = db
                        .get_latest_account_region_by_email(&req.email)
                        .ok()
                        .flatten();
                    let region = resolved_account_region(&result, existing_region);
                    region_for_response = region.clone();
                    let db_account = ipa_webtool_services::Account {
                        id: None,
                        token: token.clone(),
                        email: req.email.clone(),
                        region,
                        guid: None,
                        cookie_user: None,
                        cookies: None,
                        created_at: None,
                        updated_at: None,
                    };
                    if let Err(error) = db
                        .save_account(&db_account)
                        .map_err(|e| e.to_string())
                        .and_then(|()| persist_account_session(&db, &token, &account_store))
                    {
                        return purchase_database_error(error);
                    }

                    // 可选：加密保存凭证
                    if req.save_credentials.unwrap_or(false) {
                        if let Ok(enc_key) =
                            ipa_webtool_services::crypto::ensure_encryption_key(&db)
                        {
                            if let Ok((ct, iv, tag)) =
                                ipa_webtool_services::crypto::encrypt(&password, &enc_key)
                            {
                                let key_id = db
                                    .get_current_encryption_key()
                                    .ok()
                                    .flatten()
                                    .map(|k| k.key_id)
                                    .unwrap_or_default();
                                let creds = ipa_webtool_services::Credentials {
                                    id: None,
                                    email: req.email.clone(),
                                    password_encrypted: ct,
                                    key_id,
                                    iv,
                                    auth_tag: tag,
                                    created_at: None,
                                    updated_at: None,
                                };
                                let _ = db.save_credentials(&creds);
                            }
                        }
                    }
                }

                // 存储账号到内存（同一邮箱重新登录时替换旧会话）
                {
                    let mut accounts = ACCOUNTS.write().await;
                    accounts.retain(|_, store| store.account_email != req.email);
                    accounts.insert(token.clone(), account_store);
                }

                log::info!("Apple login success: email={}, dsid={}", req.email, dsid);

                HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                    "token": token,
                    "email": req.email,
                    "dsid": dsid,
                    "region": region_for_response,
                    "displayName": result.get("displayName"),
                })))
            } else {
                let result_obj: serde_json::Map<String, Value> =
                    result.clone().into_iter().collect();
                let (failure_type, user_facing_msg, needs_mfa) =
                    apple_auth_failure_details(&result_obj, has_mfa);
                let customer_message = result_obj
                    .get("customerMessage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                log::warn!(
                     "Apple auth failure: failureType='{}', customerMessage='{}', needs_mfa={}, has_mfa={}",
                     failure_type, customer_message, needs_mfa, has_mfa
                 );

                if needs_mfa && !has_mfa {
                    // 第一轮：暂存 AccountStore 保留 GUID，等用户提交验证码
                    save_pending_mfa(&req.email, &password, account_store).await;

                    log::info!("Saved pending MFA session for {}", req.email);

                    return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                        "status": "need_mfa",
                        "message": user_facing_msg,
                    })));
                }

                // MFA code provided but still got MFA-related error — code may be wrong/expired
                if needs_mfa && has_mfa {
                    // Re-save AccountStore for retry
                    save_pending_mfa(&req.email, &password, account_store).await;

                    log::warn!("MFA code rejected for {}", req.email);

                    return HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                        "status": "mfa_failed",
                        "message": user_facing_msg,
                    })));
                }

                // Non-MFA auth failure: do not blindly keep a pending MFA session.
                // Preserve pending state only for explicit MFA branches above, otherwise
                // wrong-password / locked-account flows can poison the next login attempt.
                clear_pending_mfa(&req.email).await;

                log::error!("Apple auth failed for {}: {}", req.email, user_facing_msg);
                HttpResponse::Ok().json(ApiResponse::<String>::error(user_facing_msg.to_string()))
            }
        }
        Err(e) => {
            let err_msg = e.to_string();
            log::error!("Apple auth exception for {}: {}", req.email, err_msg);

            // 如果是 JSON 解析错误，说明 Apple 返回了非 JSON 响应
            if err_msg.contains("error decoding response body")
                || err_msg.contains("expected value")
            {
                HttpResponse::Ok().json(ApiResponse::<String>::error(
                    "登录请求被 Apple 拒绝，请检查网络、账号密码，或者尝试使用应用专用密码登录"
                        .to_string(),
                ))
            } else if err_msg.contains("timed out") || err_msg.contains("deadline") {
                HttpResponse::Ok().json(ApiResponse::<String>::error(
                    "连接 Apple 超时，请检查网络环境".to_string(),
                ))
            } else {
                HttpResponse::Ok().json(ApiResponse::<String>::error(format!(
                    "登录失败: {}",
                    err_msg
                )))
            }
        }
    }
}

// 获取已登录的 Apple 账号列表
async fn get_account_list(data: web::Data<AppState>) -> impl Responder {
    let accounts = ACCOUNTS.read().await;
    let mut list: Vec<serde_json::Value> = Vec::new();

    let (saved_credential_emails, account_regions): (
        std::collections::HashSet<String>,
        HashMap<String, String>,
    ) = if let Ok(db) = data.db.lock() {
        let saved_credential_emails = match db.get_all_credentials() {
            Ok(creds) => creds.into_iter().map(|c| c.email).collect(),
            Err(_) => std::collections::HashSet::new(),
        };
        let account_regions = match db.get_all_accounts() {
            Ok(rows) => rows
                .into_iter()
                .map(|account| (account.token, account.region))
                .collect(),
            Err(_) => HashMap::new(),
        };
        (saved_credential_emails, account_regions)
    } else {
        (std::collections::HashSet::new(), HashMap::new())
    };

    for (token, store) in accounts.iter() {
        let dsid = store
            .auth_info
            .as_ref()
            .and_then(|ai| ai.ds_person_id.clone())
            .unwrap_or_default();
        let email = store
            .auth_info
            .as_ref()
            .and_then(|ai| ai.email.clone())
            .unwrap_or_else(|| store.account_email.clone());
        let display_name = store
            .auth_info
            .as_ref()
            .and_then(|ai| ai.display_name.clone());
        let has_saved_credentials = saved_credential_emails.contains(&email);
        let region = account_regions
            .get(token)
            .cloned()
            .filter(|value| !value.is_empty())
            .or_else(|| store.auth_info.as_ref().and_then(|ai| ai.region.clone()))
            .unwrap_or_else(|| "US".to_string());

        list.push(serde_json::json!({
            "token": token,
            "email": email,
            "dsid": dsid,
            "region": region,
            "displayName": display_name,
            "hasSavedCredentials": has_saved_credentials,
            "loggedInAt": store.authenticated_at,
        }));
    }

    // 这里只返回当前内存中仍然有效的登录会话。
    // DB 中残留但尚未恢复到 ACCOUNTS 的记录不应伪装成“已登录账号”，
    // 否则前端会拿着陈旧 token 去做下载/刷新，得到“无效的 token”。
    HttpResponse::Ok().json(ApiResponse::success(list))
}

// 删除 Apple 账号
async fn delete_account(token: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let token = token.into_inner();

    // 从内存删除
    let mut accounts = ACCOUNTS.write().await;
    let removed_account = accounts.remove(&token);
    let email = removed_account.as_ref().map(|a| a.account_email.clone());

    // 从 DB 删除
    if let Ok(db) = data.db.lock() {
        let _ = db.delete_account(&token);
        // 同时删除该 email 的凭证
        if let Some(email) = email {
            let _ = db.delete_credentials(&email);
        }
    }

    if removed_account.is_some() {
        HttpResponse::Ok().json(ApiResponse::success("已删除"))
    } else {
        HttpResponse::Ok().json(ApiResponse::success("已删除（仅数据库记录）"))
    }
}

// 获取已保存的凭证邮箱列表（不返回密码）
async fn get_credentials_list(data: web::Data<AppState>) -> impl Responder {
    let db = match data.db.lock() {
        Ok(db) => db,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(ApiResponse::<String>::error("数据库不可用".to_string()))
        }
    };

    match db.get_all_credentials() {
        Ok(creds) => {
            let emails: Vec<serde_json::Value> = creds
                .iter()
                .map(|c| serde_json::json!({ "email": c.email }))
                .collect();
            HttpResponse::Ok().json(ApiResponse::success(emails))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("获取凭证失败: {}", e))),
    }
}

async fn admin_login(
    req: HttpRequest,
    body: web::Json<AdminLoginRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let username = body.username.trim().to_string();
    let password = body.password.clone();

    log::info!("[auth:login] attempt username={}", username);

    if username.is_empty() || password.trim().is_empty() {
        log::warn!("[auth:login] rejected: empty username or password");
        return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
            "用户名和密码不能为空".to_string(),
        ));
    }

    // Rate limiting: check IP and username-based limits
    let client_ip = req
        .connection_info()
        .realip_remote_addr()
        .unwrap_or("unknown")
        .to_string();
    {
        let mut limiter = data
            .login_rate_limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Err(reason) = limiter.check_and_record(&client_ip, &username) {
            log::warn!(
                "[auth:login] rate-limited ip={} user={}: {}",
                client_ip,
                username,
                reason
            );
            return HttpResponse::TooManyRequests().json(ApiResponse::<String>::error(reason));
        }
    }

    let user = {
        let db = match data.db.lock() {
            Ok(db) => db,
            Err(_) => {
                log::error!("[auth:login] failed to acquire db lock");
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    "认证服务暂时不可用".to_string(),
                ));
            }
        };
        match db.get_admin_user(&username) {
            Ok(user) => user,
            Err(e) => {
                log::error!("[auth:login] db error looking up user {}: {}", username, e);
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("查询管理员失败: {}", e),
                ));
            }
        }
    };

    // bcrypt takes ~100ms+ of CPU: run it on the blocking pool without holding the DB lock,
    // so concurrent logins cannot stall every other request.
    let stored_hash = user.as_ref().map(|user| user.password_hash.clone());
    let (verified, upgraded_hash) = web::block(move || match stored_hash {
        Some(hash) => match matching_login_password(&password, &hash) {
            Some(matched) => (
                true,
                is_legacy_password_hash(&hash).then(|| hash_password(&matched)),
            ),
            None => (false, None),
        },
        None => {
            let _ = verify_password(&password, &DUMMY_PASSWORD_HASH);
            (false, None)
        }
    })
    .await
    .unwrap_or((false, None));

    let user = match user {
        Some(user) if verified => user,
        user => {
            if user.is_none() {
                log::warn!("[auth:login] user not found: {}", username);
            } else {
                log::warn!("[auth:login] wrong password for user={}", username);
            }
            data.login_rate_limiter
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .record_failure(&client_ip, &username);
            return HttpResponse::Unauthorized()
                .json(ApiResponse::<String>::error("用户名或密码错误".to_string()));
        }
    };

    // Login succeeded — clear rate limit entries
    data.login_rate_limiter
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear_success(&client_ip, &username);

    let db = match data.db.lock() {
        Ok(db) => db,
        Err(_) => {
            log::error!("[auth:login] failed to acquire db lock");
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                "认证服务暂时不可用".to_string(),
            ));
        }
    };

    // bcrypt ran without the DB lock: if a password change/reset landed meanwhile, the password
    // just verified is stale and must neither create a session nor drive the hash upgrade.
    if !admin_password_hash_unchanged(&db, &user.username, &user.password_hash) {
        log::warn!(
            "[auth:login] password changed during verification for user={}",
            username
        );
        return HttpResponse::Unauthorized()
            .json(ApiResponse::<String>::error("用户名或密码错误".to_string()));
    }

    if let Some(upgraded_hash) = upgraded_hash {
        // Legacy unsalted SHA-256 hashes are replaced with bcrypt on first successful login.
        match db.update_admin_password(&user.username, &upgraded_hash, user.is_default) {
            Ok(()) => log::info!(
                "[auth:login] upgraded legacy password hash for {}",
                user.username
            ),
            Err(e) => log::warn!("[auth:login] legacy hash upgrade failed: {}", e),
        }
    }

    let token = Uuid::new_v4().to_string();
    if let Err(e) = db.cleanup_expired_sessions() {
        log::warn!("[auth:login] cleanup expired sessions failed: {}", e);
    }

    if let Err(e) = db.create_session(&token, &user.username, &session_expires_at()) {
        log::error!("[auth:login] create session failed for {}: {}", username, e);
        return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "创建登录态失败: {}",
            e
        )));
    }

    log::info!(
        "[auth:login] success user={} is_default={} token={}..",
        user.username,
        user.is_default,
        &token[..8]
    );

    HttpResponse::Ok()
        .cookie(build_session_cookie(&token, request_uses_https(&req)))
        .json(ApiResponse::success(AuthUserPayload::from(&user)))
}

async fn logout(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    if let Some(session_cookie) = req.cookie(ADMIN_SESSION_COOKIE) {
        match data.db.lock() {
            Ok(db) => {
                if let Err(e) = db.delete_session(session_cookie.value()) {
                    log::warn!("清理登录态失败: {}", e);
                }
            }
            Err(_) => log::warn!("认证服务暂时不可用，跳过服务端 session 清理"),
        }
    }

    HttpResponse::Ok()
        .cookie(clear_session_cookie(request_uses_https(&req)))
        .json(ApiResponse::success("已退出登录".to_string()))
}

async fn me(admin: AuthenticatedAdmin) -> impl Responder {
    HttpResponse::Ok().json(ApiResponse::success(AuthUserPayload {
        username: admin.username,
        is_default: admin.is_default,
    }))
}

async fn change_password(
    admin: AuthenticatedAdmin,
    req: web::Json<ChangePasswordRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    log::info!(
        "[auth:change-pwd] user={} new_username={:?} session={}..",
        admin.username,
        req.new_username,
        &admin.session_token[..8.min(admin.session_token.len())]
    );

    if req.new_password.trim().is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<String>::error("新密码不能为空".to_string()));
    }

    if req.new_password.chars().count() < MIN_ADMIN_PASSWORD_CHARS {
        return HttpResponse::BadRequest().json(ApiResponse::<String>::error(format!(
            "新密码至少需要 {} 个字符",
            MIN_ADMIN_PASSWORD_CHARS
        )));
    }

    if req.current_password == req.new_password {
        return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
            "新密码不能与当前密码相同".to_string(),
        ));
    }

    let user = {
        let db = match data.db.lock() {
            Ok(db) => db,
            Err(_) => {
                log::error!(
                    "[auth:change-pwd] db lock failed for user={}",
                    admin.username
                );
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    "认证服务暂时不可用".to_string(),
                ));
            }
        };

        match db.get_admin_user(&admin.username) {
            Ok(Some(user)) => user,
            Ok(None) => {
                log::warn!("[auth:change-pwd] user not found: {}", admin.username);
                return HttpResponse::Unauthorized()
                    .json(ApiResponse::<String>::error("管理员账号不存在".to_string()));
            }
            Err(e) => {
                log::error!("[auth:change-pwd] db error for {}: {}", admin.username, e);
                return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                    format!("查询管理员失败: {}", e),
                ));
            }
        }
    };

    // Verify + hash with bcrypt on the blocking pool, outside the DB lock.
    let current_password = req.current_password.clone();
    let new_password = req.new_password.clone();
    let stored_hash = user.password_hash.clone();
    let new_password_hash = web::block(move || {
        matching_login_password(&current_password, &stored_hash)
            .map(|_| hash_password(&new_password))
    })
    .await
    .ok()
    .flatten();

    let Some(new_password_hash) = new_password_hash else {
        log::warn!(
            "[auth:change-pwd] wrong current password for user={}",
            admin.username
        );
        return HttpResponse::BadRequest()
            .json(ApiResponse::<String>::error("当前密码不正确".to_string()));
    };

    // Determine final username before any DB writes
    let new_username = req
        .new_username
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != admin.username);

    let db = match data.db.lock() {
        Ok(db) => db,
        Err(_) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                "认证服务暂时不可用".to_string(),
            ));
        }
    };

    // The current password was verified without the DB lock; refuse to write if another
    // request changed it in the meantime (the old password is no longer proof of identity).
    if !admin_password_hash_unchanged(&db, &admin.username, &user.password_hash) {
        log::warn!(
            "[auth:change-pwd] password changed concurrently for user={}",
            admin.username
        );
        return HttpResponse::Conflict().json(ApiResponse::<String>::error(
            "密码已被其他会话修改，请重新登录后再试".to_string(),
        ));
    }

    // Atomic: update password + rename in one transaction; other sessions are revoked.
    let final_username = match db.change_password_and_rename(
        &admin.username,
        &new_password_hash,
        false,
        new_username,
        Some(&admin.session_token),
    ) {
        Ok(name) => {
            log::info!(
                "[auth:change-pwd] success user={} -> {} is_default=false",
                admin.username,
                name
            );
            name
        }
        Err(e) => {
            log::error!(
                "[auth:change-pwd] transaction failed for {}: {}",
                admin.username,
                e
            );
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                format!("修改密码/用户名失败: {}", e),
            ));
        }
    };

    HttpResponse::Ok().json(ApiResponse::success(AuthUserPayload {
        username: final_username,
        is_default: false,
    }))
}

// 生成 plist 清单文件
async fn get_manifest(
    req: HttpRequest,
    query: web::Query<ManifestQuery>,
    data: web::Data<AppState>,
) -> impl Responder {
    let manifest_result = if let Some(job_id) = &query.jobId {
        let job = match data.job_store.get(job_id).await {
            Some(job) => job,
            None => {
                return HttpResponse::NotFound()
                    .json(ApiResponse::<String>::error("任务不存在".to_string()))
            }
        };
        let snapshot = job.snapshot().await;

        if snapshot.status != "ready" {
            return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                "任务尚未完成，无法生成 manifest".to_string(),
            ));
        }

        let file_exists = snapshot
            .file_path
            .as_ref()
            .map(PathBuf::from)
            .map(|path| path.exists())
            .unwrap_or(false);
        let inspection = snapshot
            .file_path
            .as_ref()
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .as_deref()
            .and_then(inspect_existing_ipa);
        let decision = derive_delivery_decision(inspection.as_ref(), file_exists);
        if !decision.ota_installable {
            let reason = inspection
                .as_ref()
                .map(|value| value.summary.clone())
                .unwrap_or_else(|| "该 IPA 不支持 OTA 安装".to_string());
            return HttpResponse::Forbidden().json(ApiResponse::<String>::error(format!(
                "已拦截 manifest 生成：{}",
                reason
            )));
        }

        let metadata = match snapshot.metadata {
            Some(metadata) => metadata,
            None => {
                return HttpResponse::InternalServerError()
                    .json(ApiResponse::<String>::error("任务缺少元数据".to_string()))
            }
        };

        let download_url = format!(
            "{}/api/public/download-file?jobId={}",
            build_base_url(&req),
            urlencoding::encode(job_id)
        );

        generate_plist(
            download_url,
            metadata.bundle_id,
            metadata.bundle_short_version_string,
            metadata.bundle_display_name,
        )
    } else {
        // Arbitrary-URL manifests would let anyone mint install prompts on this domain for
        // foreign IPAs; only signed-in admins may use this variant.
        if !request_has_admin_session(&req, &data) {
            return unauthorized_response();
        }
        let url = match &query.url {
            Some(url) => url.clone(),
            None => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::<String>::error("url 不能为空".to_string()))
            }
        };
        let bundle_id = match &query.bundle_id {
            Some(bundle_id) => bundle_id.clone(),
            None => {
                return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                    "bundle_id 不能为空".to_string(),
                ))
            }
        };
        let bundle_version = match &query.bundle_version {
            Some(bundle_version) => bundle_version.clone(),
            None => {
                return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                    "bundle_version 不能为空".to_string(),
                ))
            }
        };
        let title = match &query.title {
            Some(title) => title.clone(),
            None => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::<String>::error("title 不能为空".to_string()))
            }
        };

        generate_plist(url, bundle_id, bundle_version, title)
    };

    match manifest_result {
        Ok(plist) => HttpResponse::Ok()
            .content_type("application/xml; charset=utf-8")
            .insert_header(("Cache-Control", "no-store"))
            .body(plist),
        Err(error) => {
            log::error!("Failed to generate plist: {}", error);
            HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
                "生成 plist 失败: {}",
                error
            )))
        }
    }
}

// Plist token 解析端点（仿 OpenList /i/:link_name.plist）
async fn plist_from_token(
    req: HttpRequest,
    path: web::Path<String>,
    data: web::Data<AppState>,
) -> impl Responder {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as BASE64;
    use base64::Engine as _;

    let raw = path.into_inner();
    let token = raw.strip_suffix(".plist").unwrap_or(&raw);

    let decoded = match BASE64.decode(token) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("invalid plist token base64: {}", e);
            return HttpResponse::BadRequest().body("invalid plist token");
        }
    };

    let decoded_str = match String::from_utf8(decoded) {
        Ok(s) => s,
        Err(_) => {
            return HttpResponse::BadRequest().body("invalid plist token encoding");
        }
    };

    let parts: Vec<&str> = decoded_str.splitn(3, '/').collect();
    if parts.len() < 2 {
        return HttpResponse::BadRequest().body("malformed plist token");
    }

    let link_encode = parts[0];
    let name_encode = parts[1];
    let version_encode = parts.get(2).copied();

    let link_str = match urlencoding::decode(link_encode) {
        Ok(v) => v.into_owned(),
        Err(_) => return HttpResponse::BadRequest().body("invalid url in plist token"),
    };

    // Tokens are unauthenticated input: only serve manifests for download links this server
    // signed, otherwise anyone could mint OTA install prompts for arbitrary IPAs on this domain.
    if !is_signed_record_link(&link_str) && !request_has_admin_session(&req, &data) {
        return HttpResponse::Forbidden()
            .content_type("text/plain; charset=utf-8")
            .insert_header(("Cache-Control", "no-store"))
            .body("plist token invalid or expired");
    }

    // Hard block: if this token points to a download record whose IPA is not directly side-loadable,
    // stop before returning a plist. This prevents users from installing and then hitting black-screen crash.
    if let Some(record_id) = extract_record_id_from_download_url(&link_str) {
        let record = data
            .db
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_download_record(record_id)
            .ok()
            .flatten();

        if let Some(record) = record {
            if let Some(file_path) = record.file_path.clone() {
                let path = PathBuf::from(file_path);
                if path.exists() {
                    if let Some(inspection) = inspect_existing_ipa(&path) {
                        if inspection_blocks_install(&inspection) {
                            let message = format!("已拦截安装：{}", inspection.summary);
                            return HttpResponse::Forbidden()
                                .content_type("text/plain; charset=utf-8")
                                .insert_header(("Cache-Control", "no-store"))
                                .body(message);
                        }
                    }
                }
            }
        }
    }

    let name_full = match urlencoding::decode(name_encode) {
        Ok(v) => v.into_owned(),
        Err(_) => return HttpResponse::BadRequest().body("invalid name in plist token"),
    };

    let mut name = name_full.clone();
    let mut identifier = format!("org.ipatool.{}", name_full);
    if let Some(idx) = name_full.rfind('@') {
        name = name_full[..idx].to_string();
        identifier = name_full[idx + 1..].to_string();
    }

    if is_placeholder_bundle_id(&identifier) {
        if let Some(record_id) = extract_record_id_from_download_url(&link_str) {
            let record = data
                .db
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_download_record(record_id)
                .ok()
                .flatten();
            if let Some(record) = record {
                if let Some(file_path) = record.file_path.clone() {
                    let path = PathBuf::from(file_path);
                    if path.exists() {
                        if let Ok(Some(real_bundle_id)) = read_bundle_identifier_from_ipa(&path) {
                            if !is_placeholder_bundle_id(&real_bundle_id) {
                                identifier = real_bundle_id;
                            }
                        }
                    }
                }
            }
        }
    }

    let bundle_version = match version_encode {
        Some(encoded) => match urlencoding::decode(encoded) {
            Ok(v) => v.into_owned(),
            Err(_) => return HttpResponse::BadRequest().body("invalid version in plist token"),
        },
        None => "1.0".to_string(),
    };

    let plist_result = generate_plist(link_str, identifier, bundle_version, name);

    match plist_result {
        Ok(plist) => HttpResponse::Ok()
            .content_type("application/xml; charset=utf-8")
            .insert_header(("Cache-Control", "no-store"))
            .body(plist),
        Err(err) => {
            log::error!("Failed to generate plist from token: {}", err);
            HttpResponse::InternalServerError().body("failed to generate plist")
        }
    }
}

// OTA 安装端点 - 跳转到 itms-services 触发 iOS 原生安装
async fn install(query: web::Query<InstallQuery>) -> impl Responder {
    log::info!("OTA install request, manifest URL: {}", query.manifest);

    let install_url = format!(
        "itms-services://?action=download-manifest&url={}",
        urlencoding::encode(&query.manifest)
    );

    HttpResponse::Found()
        .insert_header(("Location", install_url))
        .finish()
}

// ============ 批量下载相关端点 ============

#[derive(Deserialize)]
struct BatchDownloadRequest {
    task_name: String,
    items: Vec<BatchItemRequest>,
}

#[derive(Deserialize)]
struct BatchItemRequest {
    app_id: String,
    app_name: Option<String>,
    version: Option<String>,
    account_email: String,
}

// 开始批量下载
async fn start_batch_download(
    req: web::Json<BatchDownloadRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    if req.task_name.trim().is_empty() {
        return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
            "task_name 不能为空".to_string(),
        ));
    }

    if req.items.is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<String>::error("items 不能为空".to_string()));
    }

    // 从全局 ACCOUNTS 中按 email 找到已认证的 AccountStore
    let accounts = ACCOUNTS.read().await;

    let mut batch_items: Vec<BatchItem<RecoveryStore>> = Vec::with_capacity(req.items.len());

    for item in &req.items {
        let account = accounts
            .values()
            .find(|acc| acc.account_email == item.account_email)
            .cloned();

        let account = match account {
            Some(a) => a,
            None => {
                return HttpResponse::BadRequest().json(ApiResponse::<String>::error(format!(
                    "账号未登录或不存在: {}",
                    item.account_email
                )));
            }
        };

        if account.auth_info.is_none() {
            return HttpResponse::BadRequest().json(ApiResponse::<String>::error(format!(
                "账号尚未完成认证: {}",
                item.account_email
            )));
        }

        batch_items.push(BatchItem {
            store: RecoveryStore::new(
                accounts
                    .iter()
                    .find(|(_, acc)| acc.account_email == account.account_email)
                    .unwrap()
                    .0,
                &account,
                &data,
            ),
            app_id: item.app_id.clone(),
            app_name: item.app_name.clone(),
            // 这里的 version 实际是 appVerId（external_identifier），用于 download_product 的 app_ver_id 参数
            version: item.version.clone(),
            account_email: item.account_email.clone(),
        });
    }

    drop(accounts);

    match data
        .download_manager
        .start_batch_download::<RecoveryStore>(&req.task_name, batch_items)
        .await
    {
        Ok(batch_id) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "batchId": batch_id,
            "taskName": req.task_name,
            "totalCount": req.items.len(),
        }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "创建批量任务失败: {}",
            e
        ))),
    }
}

// 获取所有批量下载任务
async fn get_batch_tasks(data: web::Data<AppState>) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_batch_tasks()
    {
        Ok(tasks) => HttpResponse::Ok().json(ApiResponse::success(tasks)),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "获取批量任务失败: {}",
            e
        ))),
    }
}

// 获取单个批量下载任务详情
async fn get_batch_task(path: web::Path<i64>, data: web::Data<AppState>) -> impl Responder {
    let batch_id = path.into_inner();

    // 获取任务信息
    let task = match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_batch_tasks()
    {
        Ok(tasks) => tasks.into_iter().find(|t| t.id == Some(batch_id)),
        Err(e) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
                "获取批量任务失败: {}",
                e
            )))
        }
    };

    if task.is_none() {
        return HttpResponse::NotFound()
            .json(ApiResponse::<String>::error("批量任务不存在".to_string()));
    }

    let task = task.unwrap();

    // 获取任务项目
    let items = match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_batch_items(batch_id)
    {
        Ok(items) => items,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
                "获取批量任务项失败: {}",
                e
            )))
        }
    };

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
        "task": task,
        "items": items
    })))
}

// 删除批量下载任务
async fn delete_batch_task(path: web::Path<i64>, data: web::Data<AppState>) -> impl Responder {
    let batch_id = path.into_inner();

    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_batch_task(batch_id)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success("批量任务已删除".to_string())),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "删除批量任务失败: {}",
            e
        ))),
    }
}

// ============ 下载记录端点 ============

// 获取所有下载记录
async fn get_download_records(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let base_url = build_base_url(&req);
    let db = data.db.clone();
    let downloads_dir = data.downloads_dir.clone();
    // Filesystem sync + IPA inspection is blocking work: keep it off the async worker.
    match web::block(move || list_download_records(&db, &downloads_dir, &base_url)).await {
        Ok(Ok(items)) => HttpResponse::Ok().json(ApiResponse::success(items)),
        Ok(Err(error)) => {
            log::error!("[records] failed to get records: {}", error);
            HttpResponse::InternalServerError()
                .json(ApiResponse::<String>::error("获取下载记录失败".to_string()))
        }
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
            format!("获取下载记录失败: {}", error),
        )),
    }
}

/// Persist inspections computed for records that had none, together with the derived delivery
/// decision (previously only the JSON was written and the decision columns were nulled).
fn persist_inspection_backfill(db: &Mutex<Database>, backfill: Vec<(i64, IpaInspection)>) {
    if backfill.is_empty() {
        return;
    }
    let db = db.lock().unwrap_or_else(|e| e.into_inner());
    for (record_id, inspection) in backfill {
        let Ok(json) = serde_json::to_string(&inspection) else {
            continue;
        };
        let decision = derive_delivery_decision(Some(&inspection), true);
        let _ = db.update_download_record_delivery(
            record_id,
            Some(decision.package_kind.as_str()),
            Some(decision.ota_installable),
            Some(decision.install_method.as_str()),
            Some(json.as_str()),
        );
    }
}

fn list_download_records(
    db: &Mutex<Database>,
    downloads_dir: &Path,
    base_url: &str,
) -> Result<Vec<DownloadRecordView>, String> {
    let records = {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        normalize_download_record_artifact_paths(&db, downloads_dir);
        sync_download_records_from_filesystem(&db, downloads_dir);
        db.get_all_download_records().map_err(|e| e.to_string())?
    };

    // Backfill inspection for records that lack it (outside db lock to avoid deadlock)
    let mut backfill: Vec<(i64, IpaInspection)> = Vec::new();
    let items = records
        .into_iter()
        .map(|mut record| {
            let file_exists = record
                .file_path
                .as_ref()
                .map(|path| Path::new(path).exists())
                .unwrap_or(false);
            let inspection = match persisted_inspection(&record) {
                Some(inspection) => Some(inspection),
                None if file_exists => {
                    let inspection = record
                        .file_path
                        .as_deref()
                        .and_then(|path| inspect_existing_ipa(Path::new(path)));
                    if let (Some(id), Some(inspection)) = (record.id, inspection.as_ref()) {
                        backfill.push((id, inspection.clone()));
                        // Cache on the record so build_record_install_url does not re-inspect.
                        record.inspection_json = serde_json::to_string(inspection).ok();
                    }
                    inspection
                }
                None => None,
            };
            let decision = derive_delivery_decision(inspection.as_ref(), file_exists);
            let record_id = record.id;
            let download_url = record_id.map(|id| build_record_download_url(base_url, id));
            let install_url = if decision.ota_installable {
                record_id.and_then(|id| build_record_install_url(base_url, &record, id))
            } else {
                None
            };

            DownloadRecordView {
                id: record.id,
                job_id: record.job_id,
                app_version_id: record.app_version_id,
                app_name: record.app_name,
                app_id: record.app_id,
                bundle_id: record.bundle_id,
                version: record.version,
                account_email: record.account_email,
                account_region: record.account_region,
                download_date: record.download_date,
                status: record.status,
                file_size: record.file_size,
                file_path: record.file_path,
                download_url,
                install_url,
                artwork_url: record.artwork_url,
                artist_name: record.artist_name,
                progress: record.progress,
                error: record.error,
                package_kind: decision.package_kind,
                ota_installable: decision.ota_installable,
                install_method: decision.install_method,
                created_at: record.created_at,
                file_exists,
                inspection,
            }
        })
        .collect::<Vec<_>>();

    persist_inspection_backfill(db, backfill);
    Ok(items)
}

async fn download_record_file(
    req: HttpRequest,
    path: web::Path<i64>,
    query: web::Query<SignedLinkQuery>,
    data: web::Data<AppState>,
) -> Result<fs::NamedFile, Error> {
    let id = path.into_inner();
    if !verify_public_link(
        &record_link_resource(id),
        query.expires,
        query.sig.as_deref(),
    ) && !request_has_admin_session(&req, &data)
    {
        return Err(actix_web::error::ErrorForbidden("下载链接无效或已过期"));
    }
    let record = data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_download_record(id)
        .map_err(ErrorInternalServerError)?
        .ok_or_else(|| ErrorNotFound("记录不存在"))?;
    let file_path = record
        .file_path
        .clone()
        .ok_or_else(|| ErrorNotFound("记录未保存文件路径"))?;
    let path = PathBuf::from(&file_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.ipa")
        .to_string();

    Ok(fs::NamedFile::open_async(path)
        .await
        .map_err(ErrorNotFound)?
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(file_name)],
        }))
}

async fn get_ipa_files(req: HttpRequest, data: web::Data<AppState>) -> impl Responder {
    let base_url = build_base_url(&req);
    let db = data.db.clone();
    let downloads_dir = data.downloads_dir.clone();
    // Directory walk + ZIP reads are blocking work: keep them off the async worker.
    match web::block(move || list_ipa_artifacts(&db, &downloads_dir, &base_url)).await {
        Ok(items) => HttpResponse::Ok().json(ApiResponse::success(items)),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
            format!("获取 IPA 列表失败: {}", error),
        )),
    }
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.is_empty()).cloned()
}

fn list_ipa_artifacts(
    db: &Mutex<Database>,
    downloads_dir: &Path,
    base_url: &str,
) -> Vec<IpaArtifactView> {
    let records = {
        let db = db.lock().unwrap_or_else(|e| e.into_inner());
        normalize_download_record_artifact_paths(&db, downloads_dir);
        sync_download_records_from_filesystem(&db, downloads_dir);
        db.get_all_download_records().unwrap_or_default()
    };

    let mut record_by_path = HashMap::new();
    for record in records {
        if let Some(path) = record.file_path.clone() {
            record_by_path.insert(path, record);
        }
    }

    let artifacts = scan_download_artifacts(downloads_dir);
    let mut backfill: Vec<(i64, IpaInspection)> = Vec::new();

    let items = artifacts
        .into_iter()
        .map(|artifact| {
            let path_string = artifact.path.to_string_lossy().to_string();
            let mut record = record_by_path.remove(&path_string);
            let fallback_name = artifact.file_name.trim_end_matches(".ipa").to_string();
            let download_url = format!("{}/api/ipa-files/{}/download", base_url, artifact.id);
            let inspection = match record.as_ref().and_then(persisted_inspection) {
                Some(inspection) => Some(inspection),
                None => {
                    let inspection = inspect_existing_ipa(&artifact.path);
                    if let (Some(record), Some(inspection)) = (record.as_mut(), inspection.as_ref())
                    {
                        if let Some(id) = record.id {
                            backfill.push((id, inspection.clone()));
                        }
                        record.inspection_json = serde_json::to_string(inspection).ok();
                    }
                    inspection
                }
            };
            let decision = derive_delivery_decision(inspection.as_ref(), true);
            let install_url = if decision.ota_installable {
                record.as_ref().and_then(|record| {
                    let record_id = record.id?;
                    build_record_install_url(base_url, record, record_id)
                })
            } else {
                None
            };

            let record_app_name = record
                .as_ref()
                .map(|item| item.app_name.clone())
                .filter(|value| !value.is_empty() && value != "unknown");
            let record_app_id = record
                .as_ref()
                .map(|item| item.app_id.clone())
                .filter(|value| !value.is_empty() && value != "unknown");
            let record_bundle_id = record
                .as_ref()
                .and_then(|item| non_empty(item.bundle_id.as_ref()));
            let record_version = record
                .as_ref()
                .and_then(|item| non_empty(item.version.as_ref()));
            let record_app_version_id = record
                .as_ref()
                .and_then(|item| non_empty(item.app_version_id.as_ref()));
            let record_artwork_url = record
                .as_ref()
                .and_then(|item| non_empty(item.artwork_url.as_ref()));
            let record_artist_name = record
                .as_ref()
                .and_then(|item| non_empty(item.artist_name.as_ref()));

            // Reading iTunesMetadata.plist opens the ZIP; only do it when the record is missing
            // something it could fill in, instead of for every IPA on every listing request.
            let needs_ipa_meta = record_app_name.is_none()
                || record_app_id.is_none()
                || record_bundle_id.is_none()
                || record_version.is_none()
                || record_artwork_url.is_none()
                || record_artist_name.is_none();
            let ipa_meta = needs_ipa_meta
                .then(|| ipa_webtool_services::extract_itunes_metadata_from_ipa(&artifact.path))
                .flatten();

            IpaArtifactView {
                id: artifact.id,
                file_name: artifact.file_name,
                file_size: artifact.file_size,
                file_path: path_string,
                modified_at: artifact.modified_at.map(|dt| dt.to_rfc3339()),
                app_name: record_app_name
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.item_name.clone()))
                    .or_else(|| {
                        ipa_meta
                            .as_ref()
                            .and_then(|m| m.bundle_display_name.clone())
                    })
                    .unwrap_or(fallback_name),
                app_id: record_app_id
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.item_id.clone()))
                    .unwrap_or_else(|| "unknown".to_string()),
                bundle_id: record_bundle_id
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.bundle_id.clone())),
                version: record_version
                    .or_else(|| {
                        ipa_meta
                            .as_ref()
                            .and_then(|m| m.bundle_short_version.clone())
                    })
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.bundle_version.clone())),
                app_version_id: record_app_version_id,
                account_email: record.as_ref().map(|item| item.account_email.clone()),
                account_region: record.as_ref().and_then(|item| item.account_region.clone()),
                artwork_url: record_artwork_url
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.icon_url.clone())),
                artist_name: record_artist_name
                    .or_else(|| ipa_meta.as_ref().and_then(|m| m.artist_name.clone())),
                record_id: record.as_ref().and_then(|item| item.id),
                download_url,
                install_url,
                package_kind: decision.package_kind,
                ota_installable: decision.ota_installable,
                install_method: decision.install_method,
                inspection,
            }
        })
        .collect::<Vec<_>>();

    persist_inspection_backfill(db, backfill);
    items
}

async fn download_ipa_file(
    req: HttpRequest,
    path: web::Path<String>,
    data: web::Data<AppState>,
) -> Result<fs::NamedFile, Error> {
    // Also mounted under /api/public for backwards compatibility; artifact ids are derived from
    // predictable file paths, so require an admin session either way.
    if !request_has_admin_session(&req, &data) {
        return Err(ErrorUnauthorized("未登录或登录已过期"));
    }
    let artifact_id = path.into_inner();
    let file_path = resolve_artifact_path(&data.downloads_dir, &artifact_id)
        .ok_or_else(|| ErrorNotFound("IPA 文件不存在"))?;
    let file_name = file_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.ipa")
        .to_string();

    Ok(fs::NamedFile::open_async(file_path)
        .await
        .map_err(ErrorNotFound)?
        .set_content_disposition(ContentDisposition {
            disposition: DispositionType::Attachment,
            parameters: vec![DispositionParam::Filename(file_name)],
        }))
}

async fn cleanup_empty_jobs_parent(path_buf: &Path, downloads_dir: &Path) {
    if let Some(parent) = path_buf.parent() {
        let is_jobs_child = parent
            .strip_prefix(downloads_dir.join("jobs"))
            .ok()
            .is_some();
        if is_jobs_child {
            let mut empty = true;
            if let Ok(mut entries) = tokio::fs::read_dir(parent).await {
                if entries.next_entry().await.ok().flatten().is_some() {
                    empty = false;
                }
            }
            if empty {
                let _ = tokio::fs::remove_dir(parent).await;
            }
        }
    }
}

async fn delete_ipa_file(path: web::Path<String>, data: web::Data<AppState>) -> impl Responder {
    let artifact_id = path.into_inner();
    let file_path = match resolve_artifact_path(&data.downloads_dir, &artifact_id) {
        Some(path) => path,
        None => {
            return HttpResponse::NotFound()
                .json(ApiResponse::<String>::error("IPA 文件不存在".to_string()))
        }
    };

    if let Err(error) = tokio::fs::remove_file(&file_path).await {
        return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(format!(
            "删除文件失败: {}",
            error
        )));
    }

    cleanup_empty_jobs_parent(&file_path, &data.downloads_dir).await;

    let file_path_string = file_path.to_string_lossy().to_string();
    let _ = data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_download_record_by_file_path(&file_path_string);

    HttpResponse::Ok().json(ApiResponse::success("IPA 已删除".to_string()))
}

// 删除下载记录
async fn delete_download_record(path: web::Path<i64>, data: web::Data<AppState>) -> impl Responder {
    let id = path.into_inner();

    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_download_record(id)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success("记录已删除".to_string())),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("删除记录失败: {}", e))),
    }
}

async fn clear_download_records(data: web::Data<AppState>) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear_all_download_records()
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success("记录已清空".to_string())),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("清空记录失败: {}", e))),
    }
}

async fn cleanup_download_record_file(
    path: web::Path<i64>,
    data: web::Data<AppState>,
) -> impl Responder {
    let id = path.into_inner();

    let record = match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_download_record(id)
    {
        Ok(Some(record)) => record,
        Ok(None) => {
            return HttpResponse::NotFound()
                .json(ApiResponse::<String>::error("记录不存在".to_string()))
        }
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(ApiResponse::<String>::error(format!("查询记录失败: {}", e)))
        }
    };

    let file_path = match record.file_path.clone() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return HttpResponse::BadRequest().json(ApiResponse::<String>::error(
                "记录未保存文件路径".to_string(),
            ))
        }
    };

    let path_buf = PathBuf::from(&file_path);
    let downloads_root = match tokio::fs::canonicalize(&data.downloads_dir).await {
        Ok(root) => root,
        Err(e) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                format!("下载目录不可访问: {}", e),
            ));
        }
    };
    let mut freed_bytes = 0u64;
    let mut file_deleted = false;

    if let Ok(meta) = tokio::fs::metadata(&path_buf).await {
        let canonical_path = match tokio::fs::canonicalize(&path_buf).await {
            Ok(path) => path,
            Err(e) => {
                return HttpResponse::BadRequest()
                    .json(ApiResponse::<String>::error(format!("文件路径无效: {}", e)));
            }
        };
        if !canonical_path.starts_with(&downloads_root) || !meta.is_file() {
            return HttpResponse::Forbidden().json(ApiResponse::<String>::error(
                "拒绝删除下载目录之外的文件".to_string(),
            ));
        }
        freed_bytes = meta.len();
        if let Err(e) = tokio::fs::remove_file(&canonical_path).await {
            return HttpResponse::InternalServerError().json(ApiResponse::<String>::error(
                format!("删除安装包失败: {}", e),
            ));
        }
        file_deleted = true;
    }

    cleanup_empty_jobs_parent(&path_buf, &data.downloads_dir).await;

    if let Err(e) = data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_download_record(id)
    {
        return HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("删除记录失败: {}", e)));
    }

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
        "recordId": id,
        "fileDeleted": file_deleted,
        "freed_bytes": freed_bytes,
        "freed_mb": ((freed_bytes as f64) / 1024.0 / 1024.0 * 10.0).round() / 10.0,
        "filePath": file_path,
    })))
}

// 清理服务器上的下载文件
async fn cleanup_downloads(data: web::Data<AppState>) -> impl Responder {
    let jobs_dir = data.downloads_dir.join("jobs");
    let mut cleaned = 0i64;
    let mut freed_bytes = 0u64;

    if let Ok(mut entries) = tokio::fs::read_dir(&jobs_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(mut dir_entries) = tokio::fs::read_dir(&path).await {
                    while let Ok(Some(f)) = dir_entries.next_entry().await {
                        if f.path().extension().map(|e| e == "ipa").unwrap_or(false) {
                            if let Ok(meta) = f.metadata().await {
                                freed_bytes += meta.len();
                            }
                        }
                    }
                }
                if let Err(e) = tokio::fs::remove_dir_all(&path).await {
                    eprintln!("[cleanup] Failed to remove {:?}: {}", path, e);
                } else {
                    cleaned += 1;
                }
            }
        }
    }

    HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
        "cleaned": cleaned,
        "freed_bytes": freed_bytes,
        "freed_mb": (freed_bytes as f64 / 1024.0 / 1024.0).round(),
    })))
}

// ============ 订阅相关端点 ============

#[derive(Deserialize)]
struct SubscriptionRequest {
    app_id: String,
    app_name: String,
    bundle_id: Option<String>,
    account_email: String,
    account_region: Option<String>,
    artwork_url: Option<String>,
    artist_name: Option<String>,
}

// 获取所有订阅
async fn get_subscriptions(data: web::Data<AppState>) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_all_subscriptions()
    {
        Ok(subs) => HttpResponse::Ok().json(ApiResponse::success(subs)),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("获取订阅失败: {}", e))),
    }
}

// 添加订阅
async fn add_subscription(
    req: web::Json<SubscriptionRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let subscription = NewSubscription {
        app_id: &req.app_id,
        app_name: &req.app_name,
        bundle_id: req.bundle_id.as_deref(),
        account_email: &req.account_email,
        account_region: req.account_region.as_deref(),
        artwork_url: req.artwork_url.as_deref(),
        artist_name: req.artist_name.as_deref(),
    };

    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .add_subscription(&subscription)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success("订阅已添加".to_string())),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("添加订阅失败: {}", e))),
    }
}

// 移除订阅
async fn remove_subscription(
    query: web::Query<SubscriptionRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove_subscription(&query.app_id, &query.account_email)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success("订阅已移除".to_string())),
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("移除订阅失败: {}", e))),
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct ArchiveApp {
    id: String,
    name: String,
    icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_bak_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_base64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_content_type: Option<String>,
    bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artist_name: Option<String>,
    versions: Vec<ArchiveVersion>,
    #[serde(default)]
    delisted: bool,
    added_at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    added_by: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ArchiveVersion {
    version_id: String,
    version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    released_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size_bytes: Option<i64>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct AddArchiveRequest {
    app_id: String,
    app_name: String,
    icon_url: Option<String>,
    bundle_id: Option<String>,
    #[serde(default)]
    artist_name: Option<String>,
    versions: Vec<ArchiveVersion>,
}

#[derive(Deserialize)]
struct SaveGitHubTokenRequest {
    token: String,
}

#[derive(Serialize)]
struct GitHubTokenResponse {
    configured: bool,
    username: String,
    masked_token: Option<String>,
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct CommunityPublishRequest {
    app_id: String,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_data_base64: Option<String>,
}

#[derive(Serialize)]
struct CommunityPublishResponse {
    app_id: String,
    commit_sha: Option<String>,
    pr_url: Option<String>,
    pr_number: Option<i64>,
    files_committed: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct CommunityDelistedLiteItem {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artist_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
    #[serde(
        default,
        alias = "last_seen_version",
        skip_serializing_if = "Option::is_none"
    )]
    latest_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct CommunityDelistedLiteIndex {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generated_at: Option<String>,
    #[serde(default)]
    schema_version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(default)]
    count: usize,
    #[serde(default)]
    apps: Vec<CommunityDelistedLiteItem>,
}

// ---- Community Archive Schema (v1) ----

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommunityDelistedAppDetail {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_asset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub delisted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    pub versions: Vec<CommunityVersion>,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommunityVersion {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version_id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

#[derive(Serialize, Clone)]
struct LocalDelistedCandidate {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artist_name: Option<String>,
    versions: Vec<ArchiveVersion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_download_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_record_count: Option<usize>,
    #[serde(default)]
    already_archived_locally: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    countries: Vec<String>,
}

impl CommunityDelistedAppDetail {
    pub(crate) fn from_local_archive(
        app: &ArchiveApp,
        artist_name: Option<String>,
        icon_asset: Option<String>,
        notes: Vec<String>,
        countries: Vec<String>,
    ) -> Self {
        let now = Utc::now().to_rfc3339();
        CommunityDelistedAppDetail {
            id: app.id.clone(),
            name: app.name.clone(),
            bundle_id: app.bundle_id.clone(),
            artist_name: artist_name.or_else(|| {
                let by = &app.added_by;
                if !by.is_empty() && by != "user" {
                    Some(by.clone())
                } else {
                    None
                }
            }),
            icon_asset,
            icon_url: None,
            delisted: true,
            notes,
            countries,
            versions: app
                .versions
                .iter()
                .map(|v| CommunityVersion {
                    version_id: v.version_id.clone(),
                    version: v.version.clone(),
                    released_at: v.released_at.clone(),
                    size_bytes: v.size_bytes,
                    description: v.description.clone(),
                    source: "local".to_string(),
                })
                .collect(),
            updated_at: now,
        }
    }
}

#[derive(Deserialize)]
struct PrepareContributionRequest {
    app_id: String,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Serialize)]
struct PrepareContributionResponse {
    app_id: String,
    source: String,
    github_token_configured: bool,
    app: CommunityDelistedAppDetail,
    icon_path: Option<String>,
    /// Base64 data URL of the app icon (data:image/png;base64,...)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon_data_url: Option<String>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct ContributingIdsResponse {
    archived: Vec<String>,
    in_review: Vec<String>,
}

fn archive_file_path(app_id: &str) -> PathBuf {
    resolve_archive_dir().join(format!("{}.json", app_id))
}

fn load_archive_app_from_path(file_path: &Path) -> Result<Option<ArchiveApp>, String> {
    if !file_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(file_path)
        .map_err(|error| format!("读取收藏记录失败: {}", error))?;

    let app = serde_json::from_str::<ArchiveApp>(&content)
        .map_err(|error| format!("解析收藏记录失败: {}", error))?;

    Ok(Some(app))
}

fn save_archive_app(file_path: &Path, app: &ArchiveApp) -> Result<(), String> {
    let json =
        serde_json::to_string_pretty(app).map_err(|error| format!("序列化收藏失败: {}", error))?;

    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建收藏目录失败: {}", error))?;
    }

    std::fs::write(file_path, json).map_err(|error| format!("保存收藏失败: {}", error))
}

fn community_archive_base_url() -> String {
    std::env::var("IPA_ARCHIVE_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://raw.githubusercontent.com/ruanrrn/ipa-archive/main".to_string())
        .trim_end_matches('/')
        .to_string()
}

fn community_archive_app_path(app_id: &str) -> String {
    format!("apps/delisted/{}.json", app_id)
}

fn parse_delisted_lite_index(data: Value) -> CommunityDelistedLiteIndex {
    // 1. 尝试纯数组
    if let Some(arr) = data.as_array() {
        let apps: Vec<CommunityDelistedLiteItem> = arr
            .iter()
            .filter_map(|item| {
                serde_json::from_value::<CommunityDelistedLiteItem>(item.clone()).ok()
            })
            .collect();
        return CommunityDelistedLiteIndex {
            schema_version: 1,
            source: Some("flat-array".to_string()),
            count: apps.len(),
            apps,
            ..Default::default()
        };
    }
    // 2. 尝试包装对象 { apps: [...] }
    if let Some(obj) = data.as_object() {
        if let Some(apps_arr) = obj.get("apps").and_then(|v| v.as_array()) {
            let apps: Vec<CommunityDelistedLiteItem> = apps_arr
                .iter()
                .filter_map(|item| {
                    serde_json::from_value::<CommunityDelistedLiteItem>(item.clone()).ok()
                })
                .collect();
            return CommunityDelistedLiteIndex {
                generated_at: obj
                    .get("generated_at")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                schema_version: obj
                    .get("schema_version")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1) as i32,
                source: obj.get("source").and_then(|v| v.as_str()).map(String::from),
                count: apps
                    .len()
                    .max(obj.get("count").and_then(|v| v.as_i64()).unwrap_or(0) as usize),
                apps,
            };
        }
    }
    CommunityDelistedLiteIndex::default()
}

async fn fetch_community_delisted_index() -> CommunityDelistedLiteIndex {
    {
        let cache = COMMUNITY_DELISTED_INDEX_CACHE.read().await;
        if let Some((cached_at, index)) = cache.as_ref() {
            if cached_at.elapsed().as_secs() < COMMUNITY_ARCHIVE_CACHE_TTL_SECS {
                return index.clone();
            }
        }
    }

    let client = build_http_client();
    let candidates = [
        format!(
            "{}/indexes/delisted-lite.json",
            community_archive_base_url()
        ),
        format!("{}/delisted.json", community_archive_base_url()),
    ];

    for url in candidates {
        if let Ok(resp) = client.get(&url).send().await {
            if !resp.status().is_success() {
                continue;
            }
            if let Ok(data) = resp.json::<Value>().await {
                let index = parse_delisted_lite_index(data);
                if !index.apps.is_empty() {
                    let mut cache = COMMUNITY_DELISTED_INDEX_CACHE.write().await;
                    *cache = Some((Instant::now(), index.clone()));
                    return index;
                }
            }
        }
    }

    CommunityDelistedLiteIndex::default()
}

async fn fetch_community_delisted_app(app_id: &str) -> Option<CommunityDelistedAppDetail> {
    if !is_valid_app_id(app_id) {
        return None;
    }
    {
        let cache = COMMUNITY_DELISTED_APP_CACHE.read().await;
        if let Some((cached_at, app)) = cache.get(app_id) {
            if cached_at.elapsed().as_secs() < COMMUNITY_ARCHIVE_CACHE_TTL_SECS {
                return Some(app.clone());
            }
        }
    }

    let client = build_http_client();
    let url = format!(
        "{}/{}",
        community_archive_base_url(),
        community_archive_app_path(app_id)
    );
    let response = client.get(url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let app = response.json::<CommunityDelistedAppDetail>().await.ok()?;
    {
        let mut cache = COMMUNITY_DELISTED_APP_CACHE.write().await;
        cache.insert(app_id.to_string(), (Instant::now(), app.clone()));
    }
    Some(app)
}

/// 检查应用是否仍在 App Store 上架
/// 返回 true 表示仍在商店（不应作为下架候选）
async fn check_app_still_on_store(app_id: &str) -> bool {
    {
        let cache = STORE_STATUS_CACHE.read().await;
        if let Some((cached_at, still_on_store)) = cache.get(app_id) {
            if cached_at.elapsed().as_secs() < STORE_STATUS_CACHE_TTL_SECS {
                return *still_on_store;
            }
        }
    }

    let url = format!(
        "https://itunes.apple.com/lookup?id={}&country=CN",
        urlencoding::encode(app_id)
    );
    let client = build_http_client();
    let still_on_store = match client.get(&url).send().await {
        Ok(response) if response.status().is_success() => {
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    json.get("resultCount")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        > 0
                }
                Err(_) => false,
            }
        }
        _ => false,
    };

    {
        let mut cache = STORE_STATUS_CACHE.write().await;
        cache.insert(app_id.to_string(), (Instant::now(), still_on_store));
    }

    still_on_store
}

/// 从本地 archive JSON 补全应用元信息（名称、图标）
fn load_archive_app_by_id(app_id: &str) -> Option<ArchiveApp> {
    let file_path = archive_file_path(app_id);
    load_archive_app_from_path(&file_path).ok().flatten()
}

async fn build_local_delisted_candidates(
    records: Vec<DownloadRecord>,
) -> Vec<LocalDelistedCandidate> {
    let local_archive_ids = std::fs::read_dir(resolve_archive_dir())
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .and_then(|value| value.to_str())
                .map(String::from)
        })
        .collect::<HashSet<_>>();

    // 第一阶段：按 app_id 分组聚合下载记录 + 收集每 app 的原始记录（用于后续文件存在性检查）
    let mut grouped: HashMap<String, LocalDelistedCandidate> = HashMap::new();
    let mut all_records_for_app: HashMap<String, Vec<&DownloadRecord>> = HashMap::new();

    for record in &records {
        if record.app_id.trim().is_empty() {
            continue;
        }
        if !record.status.eq_ignore_ascii_case("completed") {
            continue;
        }
        // 只看下载时已标记为下架的记录
        if record.delisted != Some(true) {
            continue;
        }

        all_records_for_app
            .entry(record.app_id.clone())
            .or_default()
            .push(record);

        let entry =
            grouped
                .entry(record.app_id.clone())
                .or_insert_with(|| LocalDelistedCandidate {
                    id: record.app_id.clone(),
                    name: record.app_name.clone(),
                    bundle_id: record.bundle_id.clone(),
                    icon_url: record.artwork_url.clone(),
                    artist_name: record.artist_name.clone(),
                    versions: Vec::new(),
                    last_download_date: record.download_date.clone().or(record.created_at.clone()),
                    source_record_count: Some(0),
                    already_archived_locally: local_archive_ids.contains(&record.app_id),
                    countries: Vec::new(),
                });

        if entry.name.trim().is_empty() && !record.app_name.trim().is_empty() {
            entry.name = record.app_name.clone();
        }
        if entry.bundle_id.is_none() {
            entry.bundle_id = record.bundle_id.clone();
        }
        if entry.icon_url.is_none() {
            entry.icon_url = record.artwork_url.clone();
        }
        if entry.artist_name.is_none() {
            entry.artist_name = record.artist_name.clone();
        }
        if entry.last_download_date.is_none() {
            entry.last_download_date = record.download_date.clone().or(record.created_at.clone());
        }
        entry.source_record_count = Some(entry.source_record_count.unwrap_or(0) + 1);

        if let Some(ref region) = record.account_region {
            let region = region.trim().to_uppercase();
            if !region.is_empty() && !entry.countries.contains(&region) {
                entry.countries.push(region);
            }
        }

        let version_id = record
            .app_version_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                record
                    .job_id
                    .clone()
                    .filter(|value| !value.trim().is_empty())
            })
            .unwrap_or_else(|| record.version.clone().unwrap_or_default());
        let version_label = record
            .version
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        if !version_id.trim().is_empty()
            && !entry
                .versions
                .iter()
                .any(|item| item.version_id == version_id)
        {
            entry.versions.push(ArchiveVersion {
                version_id,
                version: version_label,
                description: Some("由本地下载记录聚合".to_string()),
                released_at: None,
                size_bytes: record.file_size,
            });
        }
    }

    // 第二阶段：从本地 archive JSON 补全缺失的 name / icon / artist_name
    for (app_id, candidate) in grouped.iter_mut() {
        if let Some(archive_app) = load_archive_app_by_id(app_id) {
            if candidate.name.trim().is_empty() && !archive_app.name.trim().is_empty() {
                candidate.name = archive_app.name.clone();
            }
            if candidate.icon_url.is_none() {
                candidate.icon_url = archive_app.icon_url.clone();
            }
            // ArchiveApp 没有直接存 artist_name，跳过
        }
    }

    // 第三阶段：先做本地过滤（已本地归档 / 本地文件不存在），再并发确认是否仍在商店。
    // 逐个串行查询 iTunes lookup 时，N 个候选会叠加 N 次网络往返。
    let pending = grouped
        .into_iter()
        .filter(|(app_id, candidate)| {
            !candidate.already_archived_locally
                && all_records_for_app.get(app_id).is_some_and(|recs| {
                    recs.iter().any(|r| {
                        r.file_path
                            .as_ref()
                            .filter(|p| !p.trim().is_empty())
                            .is_some_and(|p| std::path::Path::new(p).exists())
                    })
                })
        })
        .collect::<Vec<_>>();

    let mut items = stream::iter(pending)
        .map(|(app_id, candidate)| async move {
            let still_on_store = check_app_still_on_store(&app_id).await;
            (candidate, still_on_store)
        })
        .buffer_unordered(STORE_STATUS_CHECK_CONCURRENCY)
        .filter_map(|(mut candidate, still_on_store)| async move {
            if still_on_store {
                return None;
            }
            if candidate.countries.is_empty() {
                candidate.countries.push("CN".to_string());
            }
            Some(candidate)
        })
        .collect::<Vec<_>>()
        .await;

    items.sort_by(|a, b| b.last_download_date.cmp(&a.last_download_date));
    items
}

fn to_archive_app_from_candidate(candidate: &LocalDelistedCandidate) -> ArchiveApp {
    ArchiveApp {
        id: candidate.id.clone(),
        name: candidate.name.clone(),
        icon_url: candidate.icon_url.clone(),
        icon_bak_url: None,
        icon_base64: None,
        icon_content_type: None,
        bundle_id: candidate.bundle_id.clone(),
        artist_name: candidate.artist_name.clone(),
        versions: candidate.versions.clone(),
        delisted: true,
        added_at: candidate
            .last_download_date
            .clone()
            .unwrap_or_else(|| Utc::now().to_rfc3339()),
        added_by: "local-download-records".to_string(),
    }
}

fn mask_github_token(token: &str) -> String {
    mask_secret(token.trim(), 4, 4)
}

async fn get_github_token(admin: AuthenticatedAdmin, data: web::Data<AppState>) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_github_token(&admin.username)
    {
        Ok(Some(token)) => HttpResponse::Ok().json(ApiResponse::success(GitHubTokenResponse {
            configured: true,
            username: admin.username,
            masked_token: Some(mask_github_token(&token.token)),
            updated_at: token.updated_at,
        })),
        Ok(None) => HttpResponse::Ok().json(ApiResponse::success(GitHubTokenResponse {
            configured: false,
            username: admin.username,
            masked_token: None,
            updated_at: None,
        })),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "读取 GitHub Token 失败: {}",
            error
        ))),
    }
}

async fn save_github_token(
    admin: AuthenticatedAdmin,
    body: web::Json<SaveGitHubTokenRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let token = body.token.trim();
    if token.is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<()>::error("GitHub PAT 不能为空".to_string()));
    }

    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .upsert_github_token(&admin.username, token)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(GitHubTokenResponse {
            configured: true,
            username: admin.username,
            masked_token: Some(mask_github_token(token)),
            updated_at: Some(Utc::now().to_rfc3339()),
        })),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "保存 GitHub Token 失败: {}",
            error
        ))),
    }
}

async fn delete_github_token(
    admin: AuthenticatedAdmin,
    data: web::Data<AppState>,
) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_github_token(&admin.username)
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(true)),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "删除 GitHub Token 失败: {}",
            error
        ))),
    }
}

// ===== WebDAV 配置 =====

#[derive(Debug, Deserialize)]
struct WebDAVConfigRequest {
    enabled: bool,
    url: String,
    username: String,
    password: String,
    remote_path: String,
}

/// Password to use for a WebDAV request. An empty field means "keep the saved password", but
/// the saved one is only reused when the request targets the saved server URL and username;
/// otherwise an admin session (or an XSS) could send the stored credential to an arbitrary host
/// via test/browse/save.
#[allow(clippy::result_large_err)]
fn resolve_webdav_password(
    data: &AppState,
    password: &str,
    url: &str,
    username: &str,
) -> Result<String, HttpResponse> {
    if !password.is_empty() {
        return Ok(password.to_string());
    }
    let saved = data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_webdav_config()
        .ok()
        .flatten();
    match saved {
        Some(config)
            if config.url.trim() == url.trim() && config.username.trim() == username.trim() =>
        {
            Ok(config.password)
        }
        Some(config) if !config.password.is_empty() => Err(HttpResponse::BadRequest().json(
            ApiResponse::<()>::error("WebDAV 地址或用户名已变更，请重新输入密码".to_string()),
        )),
        _ => Ok(String::new()),
    }
}

/// 获取 WebDAV 配置。出于安全，password 永不返回明文，只返回 masked。
async fn get_webdav_config(
    _admin: AuthenticatedAdmin,
    data: web::Data<AppState>,
) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_webdav_config()
    {
        Ok(Some(config)) => {
            let masked = mask_password(&config.password);
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "enabled": config.enabled,
                "url": config.url,
                "username": config.username,
                "password_configured": !config.password.is_empty(),
                "password_masked": masked,
                "remote_path": config.remote_path,
                "updated_at": config.updated_at,
            })))
        }
        Ok(None) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "enabled": false,
            "url": "",
            "username": "",
            "password_configured": false,
            "password_masked": "",
            "remote_path": "/",
            "updated_at": null,
        }))),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "获取 WebDAV 配置失败: {}",
            e
        ))),
    }
}

/// 保存 WebDAV 配置。
/// password 为空字符串时表示「保持原密码不变」，后端读回原值再写入。
async fn save_webdav_config(
    _admin: AuthenticatedAdmin,
    req: web::Json<WebDAVConfigRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    // password 为空 = 保持原密码（仅当地址与用户名未变更）
    let password = match resolve_webdav_password(&data, &req.password, &req.url, &req.username) {
        Ok(password) => password,
        Err(response) => return response,
    };

    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .upsert_webdav_config(
            req.enabled,
            req.url.trim(),
            req.username.trim(),
            &password,
            req.remote_path.trim(),
        ) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(true)),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "保存 WebDAV 配置失败: {}",
            e
        ))),
    }
}

/// 删除 WebDAV 配置（重置为默认/禁用）。
async fn delete_webdav_config(
    _admin: AuthenticatedAdmin,
    data: web::Data<AppState>,
) -> impl Responder {
    match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .delete_webdav_config()
    {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(true)),
        Err(e) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "删除 WebDAV 配置失败: {}",
            e
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct WebDAVTestRequest {
    url: String,
    username: String,
    password: String,
    remote_path: String,
}

/// 测试 WebDAV 连接（不保存配置，仅验证可达性 + 凭据）。
async fn test_webdav_connection(
    _admin: AuthenticatedAdmin,
    req: web::Json<WebDAVTestRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    // password 为空 = 用已保存的密码测试（仅当地址与用户名未变更）
    let password = match resolve_webdav_password(&data, &req.password, &req.url, &req.username) {
        Ok(password) => password,
        Err(response) => return response,
    };

    let config = ipa_webtool_services::WebDAVConfig {
        enabled: true,
        url: req.url.trim().to_string(),
        username: req.username.trim().to_string(),
        password,
        remote_path: req.remote_path.trim().to_string(),
        updated_at: None,
    };
    let target = match ipa_webtool_services::webdav::target_from_config(&config) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::<()>::error("WebDAV URL 不能为空".to_string()))
        }
    };

    match ipa_webtool_services::webdav::test_connection(&target).await {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "ok": true,
            "message": "连接成功"
        }))),
        Err(e) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "ok": false,
            "message": e.0
        }))),
    }
}

/// 浏览 WebDAV 目录（不保存配置，仅列出指定路径的子目录/文件）。
async fn browse_webdav_directory(
    _admin: AuthenticatedAdmin,
    req: web::Json<WebDAVTestRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    // password 为空 = 用已保存的密码（仅当地址与用户名未变更）
    let password = match resolve_webdav_password(&data, &req.password, &req.url, &req.username) {
        Ok(password) => password,
        Err(response) => return response,
    };

    let config = ipa_webtool_services::WebDAVConfig {
        enabled: true,
        url: req.url.trim().to_string(),
        username: req.username.trim().to_string(),
        password,
        remote_path: req.remote_path.trim().to_string(),
        updated_at: None,
    };
    let target = match ipa_webtool_services::webdav::target_from_config(&config) {
        Some(t) => t,
        None => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::<()>::error("WebDAV URL 不能为空".to_string()))
        }
    };

    match ipa_webtool_services::webdav::list_directory(&target).await {
        Ok(entries) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "entries": entries,
        }))),
        Err(e) => HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
            "ok": false,
            "message": e.0
        }))),
    }
}

/// 密码脱敏：只显示首尾各一个字符，不回显明文。
fn mask_password(password: &str) -> String {
    if password.is_empty() {
        return String::new();
    }
    mask_secret(password, 1, 1)
}

/// 上传单个文件到 GitHub 仓库指定分支
#[allow(clippy::too_many_arguments)]
async fn github_upload_file(
    client: &Client,
    token: &str,
    owner: &str,
    repo: &str,
    branch: &str,
    file_path: &str,
    content_base64: &str,
    commit_message: &str,
    existing_sha: Option<&str>,
) -> Result<Value, String> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}",
        owner,
        repo,
        github_contents_api_path(file_path)
    );
    let mut body = serde_json::json!({
        "message": commit_message,
        "content": content_base64,
        "branch": branch,
    });
    if let Some(sha) = existing_sha {
        body["sha"] = serde_json::json!(sha);
    }

    let resp = client
        .put(&url)
        .bearer_auth(token)
        .header(reqwest::header::USER_AGENT, "ipatool-community-publisher")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("上传文件失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("上传文件失败: HTTP {} {}", status, text));
    }

    resp.json::<Value>()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))
}

async fn publish_community_archive(
    admin: AuthenticatedAdmin,
    body: web::Json<CommunityPublishRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let app_id = body.app_id.trim().to_string();
    let notes = body.notes.clone();
    let icon_data_base64 = body.icon_data_base64.clone();

    if app_id.is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<()>::error("app_id 不能为空".to_string()));
    }
    if !app_id.chars().all(|ch| ch.is_ascii_digit()) {
        return HttpResponse::BadRequest().json(ApiResponse::<()>::error(
            "app_id 必须是数字 Apple ID".to_string(),
        ));
    }

    let github_token = match data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_github_token(&admin.username)
    {
        Ok(Some(token)) => token.token,
        Ok(None) => {
            return HttpResponse::BadRequest()
                .json(ApiResponse::<()>::error("请先配置 GitHub PAT".to_string()))
        }
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
                "读取 GitHub Token 失败: {}",
                error
            )))
        }
    };

    // 目标仓库（支持环境变量覆盖）
    let (owner, repo) = community_archive_repo();

    // 从 local archive 或 download records 加载 app 数据
    let file_path = archive_file_path(&app_id);
    let (app, source) = match load_archive_app_from_path(&file_path) {
        Ok(Some(app)) => (app, "local-archive".to_string()),
        Ok(None) => {
            let records = {
                let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
                normalize_download_record_artifact_paths(&db, &data.downloads_dir);
                sync_download_records_from_filesystem(&db, &data.downloads_dir);
                db.get_all_download_records().unwrap_or_default()
            };
            let candidates = build_local_delisted_candidates(records).await;
            match candidates.iter().find(|c| c.id == app_id) {
                Some(candidate) => (
                    to_archive_app_from_candidate(candidate),
                    "download-records".to_string(),
                ),
                None => {
                    return HttpResponse::NotFound().json(ApiResponse::<()>::error(
                        "本地归档与待贡献列表中都未找到该应用".to_string(),
                    ))
                }
            }
        }
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error));
        }
    };

    // 构建 CommunityDelistedAppDetail
    let icon_prefix = if app_id.len() >= 2 {
        &app_id[..2]
    } else {
        &app_id[..]
    };
    let icon_asset = Some(format!("assets/icons/{}/{}.png", icon_prefix, app_id));

    // 收集 countries：从下载记录中推断该 app_id 的 regions
    let countries = {
        let records = {
            let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
            normalize_download_record_artifact_paths(&db, &data.downloads_dir);
            sync_download_records_from_filesystem(&db, &data.downloads_dir);
            db.get_all_download_records().unwrap_or_default()
        };
        let regions: HashSet<String> = records
            .iter()
            .filter(|r| r.app_id == app_id && r.status.eq_ignore_ascii_case("completed"))
            .filter_map(|r| r.account_region.as_ref())
            .map(|r| r.trim().to_uppercase())
            .filter(|r| !r.is_empty())
            .collect();
        if regions.is_empty() {
            vec!["CN".to_string()]
        } else {
            regions.into_iter().collect()
        }
    };

    let detail = CommunityDelistedAppDetail::from_local_archive(
        &app,
        app.artist_name.clone(),
        icon_asset,
        notes,
        countries,
    );

    // 序列化为 JSON
    let app_json = match serde_json::to_string_pretty(&detail) {
        Ok(json) => json,
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
                "序列化 JSON 失败: {}",
                error
            )))
        }
    };

    let client = build_http_client();
    let default_branch = std::env::var("IPA_ARCHIVE_DEFAULT_BRANCH")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "main".to_string());
    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let feature_branch = format!("contribute/{}-{}-{}", app_id, timestamp, Uuid::new_v4());

    // Step 1: 获取默认分支 SHA 并创建 feature branch
    let ref_url = format!(
        "https://api.github.com/repos/{}/{}/git/ref/heads/{}",
        owner, repo, default_branch
    );
    let base_sha = match client
        .get(&ref_url)
        .bearer_auth(&github_token)
        .header(reqwest::header::USER_AGENT, "ipatool-community-publisher")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => {
            response.json::<Value>().await.ok().and_then(|p| {
                p.get("object")
                    .and_then(|o| o.get("sha"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
        }
        _ => {
            return HttpResponse::BadGateway().json(ApiResponse::<()>::error(
                "无法获取默认分支信息，请检查仓库和 PAT 权限".to_string(),
            ))
        }
    };

    let Some(sha) = base_sha else {
        return HttpResponse::BadGateway()
            .json(ApiResponse::<()>::error("无法解析默认分支 SHA".to_string()));
    };

    let create_ref_url = format!("https://api.github.com/repos/{}/{}/git/refs", owner, repo);
    let create_resp = match client
        .post(&create_ref_url)
        .bearer_auth(&github_token)
        .header(reqwest::header::USER_AGENT, "ipatool-community-publisher")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .json(&serde_json::json!({
            "ref": format!("refs/heads/{}", feature_branch),
            "sha": sha,
        }))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::BadGateway()
                .json(ApiResponse::<()>::error(format!("创建分支失败: {}", e)));
        }
    };

    if !create_resp.status().is_success() {
        let status = create_resp.status();
        let text = create_resp.text().await.unwrap_or_default();
        return HttpResponse::BadGateway().json(ApiResponse::<()>::error(format!(
            "创建分支失败: HTTP {} {}",
            status, text
        )));
    }

    // Step 2: 上传 app JSON
    let app_publish_path = format!("apps/delisted/{}.json", app_id);
    let content_base64 = base64::engine::general_purpose::STANDARD.encode(app_json.as_bytes());
    let commit_msg = format!("Add delisted app: {} ({})", detail.name, detail.id);

    let upload_result = github_upload_file(
        &client,
        &github_token,
        &owner,
        &repo,
        &feature_branch,
        &app_publish_path,
        &content_base64,
        &commit_msg,
        None,
    )
    .await;

    let commit_sha = match upload_result {
        Ok(result) => result
            .get("commit")
            .and_then(|c| c.get("sha"))
            .and_then(|v| v.as_str())
            .map(String::from),
        Err(e) => {
            return HttpResponse::BadGateway().json(ApiResponse::<()>::error(e));
        }
    };

    let mut files_committed = vec![app_publish_path];

    // Step 3: 如果有 icon_data_base64，上传 icon PNG
    if let Some(icon_b64) = &icon_data_base64 {
        if !icon_b64.is_empty() {
            let icon_path = format!("assets/icons/{}/{}.png", icon_prefix, app_id);
            let icon_commit_msg = format!("Add icon for {} ({})", detail.name, detail.id);
            if let Err(e) = github_upload_file(
                &client,
                &github_token,
                &owner,
                &repo,
                &feature_branch,
                &icon_path,
                icon_b64,
                &icon_commit_msg,
                None,
            )
            .await
            {
                log::warn!("上传 icon 失败: {}", e);
            } else {
                files_committed.push(icon_path);
            }
        }
    }

    // Step 4: 创建 PR
    let pr_api = format!("https://api.github.com/repos/{}/{}/pulls", owner, repo);
    let pr_title = format!("feat: add delisted app {} ({})", detail.name, detail.id);
    let bundle_id_display = detail.bundle_id.as_deref().unwrap_or("unknown");
    let version_count = detail.versions.len();
    let version_summary: Vec<String> = detail
        .versions
        .iter()
        .map(|v| {
            let size_str = v
                .size_bytes
                .map(|s| format!(" ({} bytes)", s))
                .unwrap_or_default();
            format!("- {}{}", v.version, size_str)
        })
        .collect();
    let notes_str = if detail.notes.is_empty() {
        String::new()
    } else {
        format!("\n\n## Notes\n{}", detail.notes.join("\n"))
    };

    let pr_body = format!(
         "## Summary\n\n- **App**: {} ({})\n- **Bundle ID**: {}\n- **Versions**: {}\n- **Source**: {}{}\n\n## Versions\n{}",
         detail.name,
         detail.id,
         bundle_id_display,
         version_count,
         source,
         notes_str,
         version_summary.join("\n"),
     );

    let (final_pr_url, final_pr_number) = match client
        .post(&pr_api)
        .bearer_auth(&github_token)
        .header(reqwest::header::USER_AGENT, "ipatool-community-publisher")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .json(&serde_json::json!({
            "title": pr_title,
            "body": pr_body,
            "head": feature_branch,
            "base": default_branch,
        }))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => match r.json::<Value>().await {
            Ok(pr_data) => (
                pr_data
                    .get("html_url")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                pr_data.get("number").and_then(|v| v.as_i64()),
            ),
            Err(_) => {
                log::warn!("解析 PR 响应失败");
                (None, None)
            }
        },
        Ok(r) => {
            let s = r.status();
            let t = r.text().await.unwrap_or_default();
            log::warn!("PR 创建失败: HTTP {} {}", s, t);
            (None, None)
        }
        Err(e) => {
            log::warn!("PR 创建失败: {}", e);
            (None, None)
        }
    };

    HttpResponse::Ok().json(ApiResponse::success(CommunityPublishResponse {
        app_id: detail.id,
        commit_sha,
        pr_url: final_pr_url,
        pr_number: final_pr_number,
        files_committed,
    }))
}

fn resolve_archive_dir() -> PathBuf {
    // resolve_project_root probes several directories; it cannot change while running.
    static ARCHIVE_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ARCHIVE_DIR
        .get_or_init(|| resolve_project_root().join("data").join("archive"))
        .clone()
}

async fn get_archive_apps() -> impl Responder {
    let archive_dir = resolve_archive_dir();
    let mut apps: Vec<ArchiveApp> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(archive_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(app) = serde_json::from_str::<ArchiveApp>(&content) {
                    apps.push(app);
                }
            }
        }
    }

    apps.sort_by(|a, b| b.added_at.cmp(&a.added_at));
    HttpResponse::Ok().json(ApiResponse::success(apps))
}

async fn add_archive_app(body: web::Json<AddArchiveRequest>) -> impl Responder {
    if !is_valid_app_id(&body.app_id) {
        return invalid_app_id_response();
    }
    let archive_dir = resolve_archive_dir();
    if let Err(error) = std::fs::create_dir_all(&archive_dir) {
        return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(format!(
            "创建收藏目录失败: {}",
            error
        )));
    }

    let file_path = archive_file_path(&body.app_id);
    let mut app = match load_archive_app_from_path(&file_path) {
        Ok(Some(existing)) => ArchiveApp {
            id: body.app_id.clone(),
            name: body.app_name.clone(),
            icon_url: body.icon_url.clone().or(existing.icon_url.clone()),
            icon_bak_url: existing.icon_bak_url,
            icon_base64: existing.icon_base64,
            icon_content_type: existing.icon_content_type,
            bundle_id: body.bundle_id.clone().or(existing.bundle_id.clone()),
            artist_name: body.artist_name.clone().or(existing.artist_name.clone()),
            versions: existing.versions,
            delisted: existing.delisted,
            added_at: existing.added_at,
            added_by: existing.added_by,
        },
        Ok(None) => ArchiveApp {
            id: body.app_id.clone(),
            name: body.app_name.clone(),
            icon_url: body.icon_url.clone(),
            icon_bak_url: None,
            icon_base64: None,
            icon_content_type: None,
            bundle_id: body.bundle_id.clone(),
            artist_name: body.artist_name.clone(),
            versions: Vec::new(),
            delisted: false,
            added_at: Utc::now().to_rfc3339(),
            added_by: "user".to_string(),
        },
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error));
        }
    };

    for version in &body.versions {
        if version.version_id.trim().is_empty() {
            continue;
        }

        if let Some(existing_version) = app
            .versions
            .iter_mut()
            .find(|existing| existing.version_id == version.version_id)
        {
            existing_version.version = if version.version.trim().is_empty() {
                existing_version.version.clone()
            } else {
                version.version.clone()
            };
            existing_version.description = version.description.clone();
            if version.released_at.is_some() {
                existing_version.released_at = version.released_at.clone();
            }
            if version.size_bytes.is_some() {
                existing_version.size_bytes = version.size_bytes;
            }
        } else {
            app.versions.push(version.clone());
        }
    }

    match save_archive_app(&file_path, &app) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(app)),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error)),
    }
}

async fn remove_archive_app_version(path: web::Path<(String, String)>) -> impl Responder {
    let (id, version_id) = path.into_inner();
    if !is_valid_app_id(&id) {
        return invalid_app_id_response();
    }
    let file_path = archive_file_path(&id);

    let Some(mut app) = (match load_archive_app_from_path(&file_path) {
        Ok(app) => app,
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error));
        }
    }) else {
        return HttpResponse::Ok().json(ApiResponse::success(true));
    };

    app.versions
        .retain(|version| version.version_id != version_id);

    if app.versions.is_empty() {
        if let Err(error) = std::fs::remove_file(&file_path) {
            return HttpResponse::InternalServerError()
                .json(ApiResponse::<()>::error(format!("取消收藏失败: {}", error)));
        }

        return HttpResponse::Ok().json(ApiResponse::success(true));
    }

    match save_archive_app(&file_path, &app) {
        Ok(_) => HttpResponse::Ok().json(ApiResponse::success(true)),
        Err(error) => HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error)),
    }
}

async fn remove_archive_app(path: web::Path<String>) -> impl Responder {
    let id = path.into_inner();
    if !is_valid_app_id(&id) {
        return invalid_app_id_response();
    }
    let file_path = archive_file_path(&id);

    if file_path.exists() {
        if let Err(error) = std::fs::remove_file(&file_path) {
            return HttpResponse::InternalServerError()
                .json(ApiResponse::<()>::error(format!("取消收藏失败: {}", error)));
        }
    }

    HttpResponse::Ok().json(ApiResponse::success(true))
}

async fn get_delisted_apps() -> impl Responder {
    let index = fetch_community_delisted_index().await;
    HttpResponse::Ok().json(ApiResponse::success(index))
}

async fn get_community_delisted_app(path: web::Path<String>) -> impl Responder {
    let app_id = path.into_inner();
    match fetch_community_delisted_app(&app_id).await {
        Some(app) => HttpResponse::Ok().json(ApiResponse::success(app)),
        None => HttpResponse::NotFound().json(ApiResponse::<()>::error(
            "社区归档中未找到该应用".to_string(),
        )),
    }
}

async fn get_local_delisted_candidates(data: web::Data<AppState>) -> impl Responder {
    {
        let cache = LOCAL_DELISTED_CANDIDATES_CACHE.read().await;
        if let Some((cached_at, candidates)) = cache.as_ref() {
            if cached_at.elapsed().as_secs() < LOCAL_DELISTED_CANDIDATES_CACHE_TTL_SECS {
                return HttpResponse::Ok().json(ApiResponse::success(candidates.clone()));
            }
        }
    }

    let records = {
        let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
        normalize_download_record_artifact_paths(&db, &data.downloads_dir);
        sync_download_records_from_filesystem(&db, &data.downloads_dir);
        db.get_all_download_records().unwrap_or_default()
    };
    let candidates = build_local_delisted_candidates(records).await;
    {
        let mut cache = LOCAL_DELISTED_CANDIDATES_CACHE.write().await;
        *cache = Some((Instant::now(), candidates.clone()));
    }
    HttpResponse::Ok().json(ApiResponse::success(candidates))
}

async fn prepare_community_contribution(
    admin: AuthenticatedAdmin,
    body: web::Json<PrepareContributionRequest>,
    data: web::Data<AppState>,
) -> impl Responder {
    let app_id = body.app_id.trim().to_string();
    let notes = body.notes.clone();

    if app_id.is_empty() {
        return HttpResponse::BadRequest()
            .json(ApiResponse::<()>::error("app_id 不能为空".to_string()));
    }
    if !is_valid_app_id(&app_id) {
        return invalid_app_id_response();
    }

    let local_path = archive_file_path(&app_id);
    let (app, source) = match load_archive_app_from_path(&local_path) {
        Ok(Some(app)) => (app, "local-archive".to_string()),
        Ok(None) => {
            let records = {
                let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
                normalize_download_record_artifact_paths(&db, &data.downloads_dir);
                sync_download_records_from_filesystem(&db, &data.downloads_dir);
                db.get_all_download_records().unwrap_or_default()
            };
            let candidates = build_local_delisted_candidates(records).await;
            match candidates.iter().find(|candidate| candidate.id == app_id) {
                Some(candidate) => (
                    to_archive_app_from_candidate(candidate),
                    "download-records".to_string(),
                ),
                None => {
                    return HttpResponse::NotFound().json(ApiResponse::<()>::error(
                        "本地归档与待贡献列表中都未找到该应用".to_string(),
                    ))
                }
            }
        }
        Err(error) => {
            return HttpResponse::InternalServerError().json(ApiResponse::<()>::error(error));
        }
    };

    // 构建 icon_asset 路径
    let icon_prefix = if app_id.len() >= 2 {
        &app_id[..2]
    } else {
        &app_id[..]
    };
    let icon_path = Some(format!("assets/icons/{}/{}.png", icon_prefix, app_id));

    // 用 CommunityDelistedAppDetail::from_local_archive() 转换
    // 收集 countries：从下载记录中推断该 app_id 的 regions
    let countries = {
        let records = {
            let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
            db.get_all_download_records().unwrap_or_default()
        };
        let regions: HashSet<String> = records
            .iter()
            .filter(|r| r.app_id == app_id && r.status.eq_ignore_ascii_case("completed"))
            .filter_map(|r| r.account_region.as_ref())
            .map(|r| r.trim().to_uppercase())
            .filter(|r| !r.is_empty())
            .collect();
        if regions.is_empty() {
            vec!["CN".to_string()]
        } else {
            regions.into_iter().collect()
        }
    };

    let mut detail = CommunityDelistedAppDetail::from_local_archive(
        &app,
        app.artist_name.clone(),
        icon_path.clone(),
        notes,
        countries,
    );

    // 版本补全：复用 get_versions 的 timbrd + bilin 聚合（同样的字段兼容：数字/字符串 id、多种 size 字段）
    let region = store_region_or(detail.countries.first().map(String::as_str), "CN");
    let external_versions = fetch_external_version_sources(&app_id, &region).await;

    // 合并版本（本地优先，去重按 version）
    let mut enriched_versions = detail.versions.clone();
    let mut existing_versions: HashSet<String> = enriched_versions
        .iter()
        .map(|v| v.version.clone())
        .collect();

    for item in &external_versions {
        let version = extract_version_label(item);
        if version.is_empty() || existing_versions.contains(&version) {
            continue;
        }
        let external_identifier = extract_version_external_identifier(item);
        let size = extract_version_size(item);
        let released_at = extract_version_created_at(item);
        let source = extract_version_source(item);
        enriched_versions.push(CommunityVersion {
            version_id: if external_identifier > 0 {
                external_identifier.to_string()
            } else {
                String::new()
            },
            version: version.clone(),
            released_at: (!released_at.is_empty()).then_some(released_at),
            size_bytes: (size > 0).then_some(size),
            description: None,
            source: if source.is_empty() {
                "external".to_string()
            } else {
                source
            },
        });
        existing_versions.insert(version);
    }

    // 语义化版本排序（降序）
    enriched_versions.sort_by(|a, b| {
        let parse =
            |v: &str| -> Vec<u32> { v.split('.').filter_map(|p| p.parse::<u32>().ok()).collect() };
        let va = parse(&a.version);
        let vb = parse(&b.version);
        vb.cmp(&va)
    });

    detail.versions = enriched_versions;

    // 检查 GitHub PAT 配置状态
    let github_token_configured = data
        .db
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_github_token(&admin.username)
        .ok()
        .flatten()
        .is_some();

    // 生成 warnings
    let mut warnings: Vec<String> = Vec::new();
    if detail.bundle_id.is_none() {
        warnings.push("缺少 bundle_id，建议手动补充".to_string());
    }
    if detail.artist_name.is_none() {
        warnings.push("缺少 artist_name（开发者名称）".to_string());
    }
    if detail.versions.is_empty() {
        warnings.push("没有版本信息".to_string());
    } else if detail.versions.iter().all(|v| v.size_bytes.is_none()) {
        warnings.push("所有版本均无 size_bytes".to_string());
    }
    if icon_path.is_none() {
        warnings.push("无法构建 icon 路径".to_string());
    }

    // 下载图标并转为 base64 data URL
    let icon_data_url = match app.icon_url.as_deref() {
        Some(url) if url.starts_with("data:") => Some(url.to_string()),
        Some(url) if url.starts_with("http") => match fetch_icon_data_url(url).await {
            Ok(data_url) => Some(data_url),
            Err(message) => {
                warnings.push(message);
                None
            }
        },
        Some(_) => None,
        None => {
            warnings.push("缺少图标 URL".to_string());
            None
        }
    };

    HttpResponse::Ok().json(ApiResponse::success(PrepareContributionResponse {
        app_id: detail.id.clone(),
        source,
        github_token_configured,
        app: detail,
        icon_path,
        icon_data_url,
        warnings,
    }))
}

const MAX_ICON_BYTES: usize = 2 * 1024 * 1024;

/// Fetch an app icon for the contribution preview. The URL comes from user-editable archive
/// data, so only HTTPS images are accepted and the body is size-capped: the response is echoed
/// back to the client and must not become a way to read arbitrary (internal) URLs.
async fn fetch_icon_data_url(url: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("图标地址无效: {}", e))?;
    if parsed.scheme() != "https" {
        return Err("图标地址必须是 https".to_string());
    }
    let response = build_http_client()
        .get(parsed)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("图标下载失败: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("图标下载返回 {}", response.status()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase())
        .unwrap_or_default();
    if !content_type.starts_with("image/") {
        return Err("图标地址返回的不是图片".to_string());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ICON_BYTES as u64)
    {
        return Err("图标文件过大".to_string());
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("图标下载失败: {}", e))?;
        if body.len() + chunk.len() > MAX_ICON_BYTES {
            return Err("图标文件过大".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(format!(
        "data:{};base64,{}",
        content_type,
        base64::engine::general_purpose::STANDARD.encode(&body)
    ))
}

/// Target repository of the community archive (overridable for forks).
fn community_archive_repo() -> (String, String) {
    let owner = std::env::var("IPA_ARCHIVE_OWNER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ruanrrn".to_string());
    let repo = std::env::var("IPA_ARCHIVE_REPO")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "ipa-archive".to_string());
    (owner, repo)
}

// 从 PR title 提取 (\d+) 格式的 app_id
fn extract_app_id_from_pr_title(title: &str) -> Option<String> {
    let start = title.find('(')?;
    let end = title[start + 1..].find(')')?;
    let inner = &title[start + 1..start + 1 + end];
    if inner.chars().all(|c| c.is_ascii_digit()) && !inner.is_empty() {
        Some(inner.to_string())
    } else {
        None
    }
}

async fn get_contributing_ids(
    admin: AuthenticatedAdmin,
    data: web::Data<AppState>,
) -> impl Responder {
    let _ = admin; // 需要认证

    // 1. 已收录：从 community index 获取
    let index = fetch_community_delisted_index().await;
    let archived: Vec<String> = index.apps.iter().map(|app| app.id.clone()).collect();

    // 2. 审核中：GitHub Search API 查 open PR
    let mut in_review: Vec<String> = Vec::new();

    // 获取 GitHub token
    let github_token = {
        let db = data.db.lock().unwrap_or_else(|e| e.into_inner());
        db.get_github_token(&admin.username)
            .ok()
            .flatten()
            .map(|t| t.token)
    };

    if let Some(token) = github_token {
        let (owner, repo) = community_archive_repo();
        let search_url = format!(
            "https://api.github.com/search/issues?q={}&per_page=100",
            urlencoding::encode(&format!("is:pr is:open repo:{}/{}", owner, repo))
        );

        if let Ok(resp) = build_http_client()
            .get(&search_url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "ipatool-community")
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
        {
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if let Some(items) = body["items"].as_array() {
                    for item in items {
                        if let Some(title) = item["title"].as_str() {
                            if let Some(app_id) = extract_app_id_from_pr_title(title) {
                                in_review.push(app_id);
                            }
                        }
                    }
                }
            }
        }
    }

    HttpResponse::Ok().json(ApiResponse::success(ContributingIdsResponse {
        archived,
        in_review,
    }))
}

// 检查更新
async fn check_updates(data: web::Data<AppState>) -> impl Responder {
    match data.download_manager.check_app_updates().await {
        Ok(updates) => {
            let count: usize = updates.len();
            HttpResponse::Ok().json(ApiResponse::success(serde_json::json!({
                "updates": updates,
                "count": count
            })))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(ApiResponse::<String>::error(format!("检查更新失败: {}", e))),
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if handle_reset_admin_password_args(&args)? {
        return Ok(());
    }

    // 初始化数据库
    let project_root = resolve_project_root();
    let data_dir = project_root.join("data");
    let downloads_dir = project_root.join("downloads");
    let db_path = resolve_database_path(&project_root);
    log::info!("Initializing database at: {}", db_path.display());
    std::fs::create_dir_all(&data_dir).ok();
    std::fs::create_dir_all(&downloads_dir).ok();
    let db = Database::new(db_path.to_string_lossy().as_ref()).unwrap_or_else(|e| {
        log::error!("Failed to initialize database: {}", e);
        panic!("Database initialization failed: {}", e);
    });
    init_public_link_key(&db);

    // 将数据库包装在 Arc<Mutex<Database>> 中
    let db_arc = Arc::new(Mutex::new(db));

    // 初始化下载管理器
    let download_manager = Arc::new(DownloadManager::new(
        Arc::clone(&db_arc),
        downloads_dir.clone(),
    ));

    let app_state = web::Data::new(AppState {
        db: db_arc.clone(),
        download_manager: download_manager.clone(),
        job_store: JobStore::new(),
        downloads_dir,
        login_rate_limiter: Arc::new(Mutex::new(LoginRateLimiter::new())),
    });

    // 恢复重启前已登录的 Apple 账号，避免每次重启/升级后登录态丢失。
    restore_account_sessions(&db_arc).await;

    let bind_address = resolve_bind_address();
    log::info!("Starting server at {}", bind_address);

    let server = HttpServer::new(move || {
         App::new()
             .wrap(from_fn(security_headers))
             .app_data(web::JsonConfig::default().limit(2 * 1024 * 1024))
             .app_data(app_state.clone())
             .route("/i/{token}.plist", web::get().to(plist_from_token))
             // 容器/负载均衡健康检查（Dockerfile HEALTHCHECK 访问 /health），无需登录
             .route("/health", web::get().to(health))
             .service(
                 web::scope("/api")
                     // 公开路由：管理员认证
                     .service(
                         web::scope("/auth")
                             .route("/login", web::post().to(admin_login))
                             .route("/logout", web::post().to(logout))
                             .route("/me", web::get().to(me))
                             .route("/change-password", web::post().to(change_password)),
                     )
                     // 公开 OTA / 下载路由（iOS 安装器不会携带后台登录 cookie）
                     .service(
                         web::scope("/public")
                             .route("/download-file", web::get().to(download_file))
                             .route("/manifest", web::get().to(get_manifest))
                             .route("/install", web::get().to(install))
                            .route("/download-records/{id}/file", web::get().to(download_record_file))
                            .route("/ipa-files/{id}/download", web::get().to(download_ipa_file)),
                     )
                     // 公开归档数据（不依赖管理员登录）
                     .route("/archive/delisted", web::get().to(get_delisted_apps))
                     .route("/community/delisted-index", web::get().to(get_delisted_apps))
                     .route("/community/delisted/{id}", web::get().to(get_community_delisted_app))
                     // 需要管理员认证的路由
                     .service(
                         web::scope("")
                             .wrap(from_fn(require_auth))
                             .route("/health", web::get().to(health))
                             .route("/login", web::post().to(apple_login))
                             .route("/accounts", web::get().to(get_account_list))
                             .route("/accounts/{token}", web::delete().to(delete_account))
                             .route("/credentials", web::get().to(get_credentials_list))
                             .route("/versions", web::get().to(get_versions))
                             .route("/download-url", web::get().to(get_download_url))
                             .route("/purchase-status", web::get().to(get_purchase_status))
                             .route("/claim", web::post().to(claim_app))
                             .route("/purchase-status-batch", web::post().to(purchase_status_batch))
                             .route("/confirm-purchase", web::post().to(confirm_purchase))
                             .route("/start-download-direct", web::post().to(start_download_direct))
                             .route("/progress-sse", web::get().to(progress_sse))
                             .route("/job-info", web::get().to(get_job_info))
                             .route("/upload-ipa", web::post().to(upload_ipa))
                             .route("/search", web::get().to(search_app))
                             .route("/app-meta", web::get().to(app_meta))
                             .route("/batch-download", web::post().to(start_batch_download))
                             .route("/batch-tasks", web::get().to(get_batch_tasks))
                             .route("/batch-tasks/{id}", web::get().to(get_batch_task))
                             .route("/batch-tasks/{id}", web::delete().to(delete_batch_task))
                            .route("/download-records", web::get().to(get_download_records))
                            .route("/download-jobs", web::get().to(get_download_records))
                            .route("/download-records", web::delete().to(clear_download_records))
.route("/download-records/{id}", web::delete().to(delete_download_record))
                            .route(
                                "/download-records/{id}/file",
                                web::delete().to(cleanup_download_record_file),
                            )
                            .route("/ipa-files", web::get().to(get_ipa_files))
                            .route("/ipa-files/{id}/download", web::get().to(download_ipa_file))
                             .route("/ipa-files/{id}", web::delete().to(delete_ipa_file))
                             .route("/cleanup-downloads", web::post().to(cleanup_downloads))
                             .route("/subscriptions", web::get().to(get_subscriptions))
                             .route("/subscriptions", web::post().to(add_subscription))
                             .route("/subscriptions", web::delete().to(remove_subscription))
                             .route("/check-updates", web::get().to(check_updates))
                             .route("/archive", web::get().to(get_archive_apps))
                             .route("/archive", web::post().to(add_archive_app))
                             .route(
                                 "/archive/{id}/versions/{version_id}",
                                 web::delete().to(remove_archive_app_version),
                             )
                             .route("/archive/{id}", web::delete().to(remove_archive_app))
                             .route("/github/token", web::get().to(get_github_token))
                             .route("/github/token", web::post().to(save_github_token))
                             .route("/github/token", web::delete().to(delete_github_token))
                             .route("/webdav/config", web::get().to(get_webdav_config))
                             .route("/webdav/config", web::post().to(save_webdav_config))
                             .route("/webdav/config", web::delete().to(delete_webdav_config))
                             .route("/webdav/test", web::post().to(test_webdav_connection))
                             .route("/webdav/browse", web::post().to(browse_webdav_directory))
                             .route("/community/publish", web::post().to(publish_community_archive))
                             .route("/local/delisted-candidates", web::get().to(get_local_delisted_candidates))
                             .route("/community/contributing-ids", web::get().to(get_contributing_ids))
                             .route("/community/prepare-contribution", web::post().to(prepare_community_contribution)),

                     ),
             )
             // 托管前端静态文件
             .service(fs::Files::new("/", "./dist").index_file("index.html"))
    })
    .shutdown_timeout(SHUTDOWN_TIMEOUT_SECS);

    log::info!(
        "Graceful shutdown enabled: max {}s drain on SIGINT/SIGTERM",
        SHUTDOWN_TIMEOUT_SECS
    );

    server.bind(bind_address)?.run().await
}

/// Listen address: `BIND_ADDRESS` (host:port) wins, otherwise `0.0.0.0:$PORT` (default 8080).
/// The install script and Docker image configure the port through `PORT`.
fn resolve_bind_address() -> String {
    if let Some(address) = std::env::var("BIND_ADDRESS")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return address;
    }
    let port = std::env::var("PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(8080);
    format!("0.0.0.0:{}", port)
}

/// 接收到停机信号后，等待进行中的请求（含 SSE / 文件下载）完成的最长时间。
/// 超过后强制断开。8s 是在“快速回收资源”与“大文件下载收尾”之间的折中。
const SHUTDOWN_TIMEOUT_SECS: u64 = 8;
