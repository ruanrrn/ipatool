use crate::{AccountStore, AdminUser, IpaInspection};
use actix_web::{
    cookie::time::Duration as CookieDuration, cookie::Cookie, cookie::SameSite, HttpResponse,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

// ── Session constants ────────────────────────────────────────────────
pub const ADMIN_SESSION_COOKIE: &str = "ipa_admin_session";
pub const SESSION_TTL_DAYS: i64 = 30;
pub const PENDING_MFA_TTL_MINUTES: i64 = 10;

// ── Generic API wrapper ──────────────────────────────────────────────
#[derive(Serialize)]
pub struct ApiResponse<T> {
    pub ok: bool,
    pub data: Option<T>,
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            ok: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(error: String) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error),
        }
    }
}

// ── Query / request structs ──────────────────────────────────────────
#[derive(Deserialize)]
pub struct VersionQuery {
    pub appid: String,
    pub region: Option<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
#[allow(dead_code)]
pub struct DownloadUrlQuery {
    pub token: String,
    pub appid: String,
    pub appVerId: Option<String>,
    #[serde(default)]
    pub autoPurchase: bool,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct PurchaseStatusQuery {
    pub token: String,
    pub appid: String,
    pub appVerId: Option<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct ClaimRequest {
    pub token: String,
    pub appid: String,
    pub appVerId: Option<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct PurchaseStatusBatchRequest {
    pub token: String,
    pub appids: Vec<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct ConfirmPurchaseRequest {
    pub token: String,
    pub appid: String,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct StartDownloadDirectRequest {
    pub token: String,
    pub appid: String,
    pub appVerId: Option<String>,
    pub appName: Option<String>,
    pub bundleId: Option<String>,
    pub appVersion: Option<String>,
    pub artworkUrl: Option<String>,
    pub artistName: Option<String>,
    #[serde(default)]
    pub autoPurchase: bool,
}

#[derive(Deserialize)]
pub struct AppMetaQuery {
    pub appid: String,
    pub region: Option<String>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct JobIdQuery {
    pub jobId: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppleLoginRequest {
    pub email: String,
    pub password: String,
    pub mfa: Option<String>,
    pub save_credentials: Option<bool>,
}

#[derive(Deserialize)]
pub struct AdminLoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
    pub new_username: Option<String>,
}

// ── Auth structs ─────────────────────────────────────────────────────
#[derive(Serialize, Clone)]
pub struct AuthUserPayload {
    pub username: String,
    pub is_default: bool,
}

impl From<&AdminUser> for AuthUserPayload {
    fn from(user: &AdminUser) -> Self {
        Self {
            username: user.username.clone(),
            is_default: user.is_default,
        }
    }
}

// ── AuthenticatedAdmin stays in main.rs to satisfy orphan rule ──
// (FromRequest impl for actix_web trait must be in same crate as the type)

// ── Manifest struct ──────────────────────────────────────────────────
#[derive(Deserialize, Default)]
#[allow(non_snake_case)]
pub struct ManifestQuery {
    pub url: Option<String>,
    pub bundle_id: Option<String>,
    pub bundle_version: Option<String>,
    pub title: Option<String>,
    pub jobId: Option<String>,
}

// ── Download / artifact structs ──────────────────────────────────────
#[derive(Debug, Clone)]
pub struct DownloadArtifact {
    pub id: String,
    pub path: std::path::PathBuf,
    pub file_name: String,
    pub file_size: u64,
    pub modified_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRecordView {
    pub id: Option<i64>,
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version_id: Option<String>,
    pub app_name: String,
    pub app_id: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    pub account_email: String,
    pub account_region: Option<String>,
    pub download_date: Option<String>,
    pub status: String,
    pub file_size: Option<i64>,
    pub file_path: Option<String>,
    pub download_url: Option<String>,
    pub install_url: Option<String>,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub progress: Option<i64>,
    pub error: Option<String>,
    pub package_kind: String,
    pub ota_installable: bool,
    pub install_method: String,
    pub created_at: Option<String>,
    pub file_exists: bool,
    pub inspection: Option<IpaInspection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpaArtifactView {
    pub id: String,
    pub file_name: String,
    pub file_size: u64,
    pub file_path: String,
    pub modified_at: Option<String>,
    pub app_name: String,
    pub app_id: String,
    pub bundle_id: Option<String>,
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version_id: Option<String>,
    pub account_email: Option<String>,
    pub account_region: Option<String>,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub record_id: Option<i64>,
    pub download_url: String,
    pub install_url: Option<String>,
    pub package_kind: String,
    pub ota_installable: bool,
    pub install_method: String,
    pub inspection: Option<IpaInspection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingDownloadResponse {
    pub job_id: String,
    pub record_id: Option<i64>,
    pub app_id: String,
    pub version: String,
    pub app_name: String,
    pub account_email: String,
    pub file_path: String,
    pub file_size: Option<i64>,
    pub download_url: String,
    pub install_url: Option<String>,
    pub package_kind: String,
    pub ota_installable: bool,
    pub install_method: String,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub bundle_id: Option<String>,
    pub reused: bool,
    pub task_dir: String,
}

// ── MFA / purchase structs ───────────────────────────────────────────
#[derive(Clone)]
pub struct PendingMfaSession {
    pub account_store: AccountStore,
    pub password_hash: String,
    pub created_at: chrono::DateTime<Utc>,
}

pub struct PurchaseCacheEntry {
    pub purchased: bool,
    pub needs_purchase: bool,
    pub error: Option<String>,
    pub cached_at: std::time::Instant,
}

// ── Delivery decision ────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct DeliveryDecision {
    pub package_kind: String,
    pub ota_installable: bool,
    pub install_method: String,
}

// ── Helper functions ─────────────────────────────────────────────────
pub fn normalize_mfa_code(mfa: Option<&str>) -> Option<String> {
    mfa.map(|code| code.trim().replace(' ', ""))
        .filter(|code| !code.is_empty())
}

pub fn normalize_region_code(region: &str) -> Option<String> {
    let normalized = region.trim().to_uppercase();
    if normalized.len() >= 2 && normalized.len() <= 3 {
        Some(normalized)
    } else {
        None
    }
}

pub fn is_pending_mfa_expired(created_at: chrono::DateTime<Utc>) -> bool {
    Utc::now().signed_duration_since(created_at) > Duration::minutes(PENDING_MFA_TTL_MINUTES)
}

pub fn session_expires_at() -> String {
    (Utc::now() + Duration::days(SESSION_TTL_DAYS))
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Decide whether the admin session cookie gets the `Secure` attribute.
///
/// `FORCE_SECURE_COOKIE=1|true` / `0|false` force the choice. Otherwise it follows the request
/// scheme (honouring `X-Forwarded-Proto` from a TLS-terminating proxy): browsers refuse to store
/// `Secure` cookies sent over plain HTTP, so always setting it broke logins on
/// `http://<lan-ip>:8080` deployments.
pub fn should_use_secure_cookie(request_is_https: bool) -> bool {
    match std::env::var("FORCE_SECURE_COOKIE").as_deref() {
        Ok("1") | Ok("true") => true,
        Ok("0") | Ok("false") => false,
        _ => request_is_https,
    }
}

pub fn build_session_cookie(token: &str, secure: bool) -> Cookie<'static> {
    Cookie::build(ADMIN_SESSION_COOKIE, token.to_string())
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .max_age(CookieDuration::days(SESSION_TTL_DAYS))
        .secure(secure)
        .finish()
}

pub fn clear_session_cookie(secure: bool) -> Cookie<'static> {
    let mut cookie = Cookie::build(ADMIN_SESSION_COOKIE, "")
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure)
        .finish();
    cookie.make_removal();
    cookie
}

pub fn unauthorized_response() -> HttpResponse {
    HttpResponse::Unauthorized().json(ApiResponse::<String>::error(
        "未登录或登录已过期".to_string(),
    ))
}
