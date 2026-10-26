use crate::sap::{self, SapSigner};
use base64::Engine as _;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

// Match ipatool reference: Configurator/2.17 + macOS 15.2
const USER_AGENT: &str =
    "Configurator/2.17 (Macintosh; OS X 15.2; 24C5089c) AppleWebKit/0620.1.16.11.6";
const DEFAULT_AUTH_ENDPOINT: &str = "https://auth.itunes.apple.com/auth/v1/native/fast/";
const NATIVE_AUTH_HOST: &str = "auth.itunes.apple.com";
const AUTH_REQUEST_ATTEMPTS: usize = 3;
const AUTH_RETRY_DELAY: Duration = Duration::from_secs(10);
const MAX_AUTH_RETRY_DELAY: Duration = Duration::from_secs(30);

/// Apple only answers the auth endpoints with a trailing slash: the bare legacy path
/// returns an empty or HTML response, the bare native path a 301 without Location.
fn normalize_auth_url(raw: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return raw.to_string();
    };
    let host = url.host_str().unwrap_or("").to_string();
    if is_buy_host(&host)
        && url
            .path()
            .trim_end_matches('/')
            .eq("/WebObjects/MZFinance.woa/wa/authenticate")
    {
        url.set_path("/WebObjects/MZFinance.woa/wa/authenticate/");
        return url.to_string();
    }
    if host != NATIVE_AUTH_HOST {
        return raw.to_string();
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/fast") {
        path.push_str("/fast");
    }
    url.set_path(&format!("{}/", path));
    url.to_string()
}

fn is_buy_host(host: &str) -> bool {
    host == "buy.itunes.apple.com"
        || host
            .strip_suffix("-buy.itunes.apple.com")
            .and_then(|pod| pod.strip_prefix('p'))
            .is_some_and(|digits| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
}

/// Only send credentials to Apple's HTTPS auth hosts.
fn approved_auth_endpoint(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    let host = url.host_str()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !(host == NATIVE_AUTH_HOST || is_buy_host(host))
    {
        return None;
    }
    Some(url.to_string())
}

fn is_transient_auth_status(status: u16) -> bool {
    matches!(status, 204 | 404 | 429) || status / 100 == 5
}

fn retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let wait = date.signed_duration_since(chrono::Utc::now());
    Some(wait.to_std().unwrap_or(Duration::ZERO))
}

fn auth_failure(failure_type: &str, message: &str) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    m.insert("_state".to_string(), Value::String("failure".to_string()));
    m.insert(
        "failureType".to_string(),
        Value::String(failure_type.to_string()),
    );
    m.insert(
        "customerMessage".to_string(),
        Value::String(message.to_string()),
    );
    m
}

fn storefront_region(code: &str) -> Option<&'static str> {
    match code {
        "143441" => Some("US"),
        "143465" => Some("CN"),
        "143462" => Some("JP"),
        "143444" => Some("GB"),
        "143443" => Some("DE"),
        "143442" => Some("FR"),
        "143455" => Some("CA"),
        "143460" => Some("AU"),
        _ => None,
    }
}

pub fn region_to_storefront_code(region: &str) -> Option<&'static str> {
    match region.trim().to_uppercase().as_str() {
        "AE" => Some("143481"),
        "AG" => Some("143540"),
        "AI" => Some("143538"),
        "AL" => Some("143575"),
        "AM" => Some("143524"),
        "AO" => Some("143564"),
        "AR" => Some("143505"),
        "AT" => Some("143445"),
        "AU" => Some("143460"),
        "AZ" => Some("143568"),
        "BB" => Some("143541"),
        "BD" => Some("143490"),
        "BE" => Some("143446"),
        "BG" => Some("143526"),
        "BH" => Some("143559"),
        "BM" => Some("143542"),
        "BN" => Some("143560"),
        "BO" => Some("143556"),
        "BR" => Some("143503"),
        "BS" => Some("143539"),
        "BW" => Some("143525"),
        "BY" => Some("143565"),
        "BZ" => Some("143555"),
        "CA" => Some("143455"),
        "CH" => Some("143459"),
        "CI" => Some("143527"),
        "CL" => Some("143483"),
        "CN" => Some("143465"),
        "CO" => Some("143501"),
        "CR" => Some("143495"),
        "CY" => Some("143557"),
        "CZ" => Some("143489"),
        "DE" => Some("143443"),
        "DK" => Some("143458"),
        "DM" => Some("143545"),
        "DO" => Some("143508"),
        "DZ" => Some("143563"),
        "EC" => Some("143509"),
        "EE" => Some("143518"),
        "EG" => Some("143516"),
        "ES" => Some("143454"),
        "FI" => Some("143447"),
        "FR" => Some("143442"),
        "GB" | "UK" => Some("143444"),
        "GD" => Some("143546"),
        "GH" => Some("143573"),
        "GR" => Some("143448"),
        "GT" => Some("143504"),
        "GY" => Some("143553"),
        "HK" => Some("143463"),
        "HN" => Some("143510"),
        "HR" => Some("143494"),
        "HU" => Some("143482"),
        "ID" => Some("143476"),
        "IE" => Some("143449"),
        "IL" => Some("143491"),
        "IN" => Some("143467"),
        "IS" => Some("143558"),
        "IT" => Some("143450"),
        "JM" => Some("143511"),
        "JO" => Some("143528"),
        "JP" => Some("143462"),
        "KE" => Some("143529"),
        "KN" => Some("143548"),
        "KR" => Some("143466"),
        "KW" => Some("143493"),
        "KY" => Some("143544"),
        "KZ" => Some("143517"),
        "LB" => Some("143497"),
        "LC" => Some("143549"),
        "LI" => Some("143522"),
        "LK" => Some("143486"),
        "LT" => Some("143520"),
        "LU" => Some("143451"),
        "LV" => Some("143519"),
        "MD" => Some("143523"),
        "MG" => Some("143531"),
        "MK" => Some("143530"),
        "ML" => Some("143532"),
        "MN" => Some("143592"),
        "MO" => Some("143515"),
        "MS" => Some("143547"),
        "MT" => Some("143521"),
        "MU" => Some("143533"),
        "MV" => Some("143488"),
        "MX" => Some("143468"),
        "MY" => Some("143473"),
        "NE" => Some("143534"),
        "NG" => Some("143561"),
        "NI" => Some("143512"),
        "NL" => Some("143452"),
        "NO" => Some("143457"),
        "NP" => Some("143484"),
        "NZ" => Some("143461"),
        "OM" => Some("143562"),
        "PA" => Some("143485"),
        "PE" => Some("143507"),
        "PH" => Some("143474"),
        "PK" => Some("143477"),
        "PL" => Some("143478"),
        "PT" => Some("143453"),
        "PY" => Some("143513"),
        "QA" => Some("143498"),
        "RO" => Some("143487"),
        "RS" => Some("143500"),
        "RU" => Some("143469"),
        "SA" => Some("143479"),
        "SE" => Some("143456"),
        "SG" => Some("143464"),
        "SI" => Some("143499"),
        "SK" => Some("143496"),
        "SN" => Some("143535"),
        "SR" => Some("143554"),
        "SV" => Some("143506"),
        "TC" => Some("143552"),
        "TH" => Some("143475"),
        "TN" => Some("143536"),
        "TR" => Some("143480"),
        "TT" => Some("143551"),
        "TW" => Some("143470"),
        "TZ" => Some("143572"),
        "UA" => Some("143492"),
        "UG" => Some("143537"),
        "US" => Some("143441"),
        "UY" => Some("143514"),
        "UZ" => Some("143566"),
        "VC" => Some("143550"),
        "VE" => Some("143502"),
        "VG" => Some("143543"),
        "VN" => Some("143471"),
        "YE" => Some("143571"),
        "ZA" => Some("143472"),
        _ => None,
    }
}

pub fn region_to_storefront_header(region: &str) -> Option<String> {
    let clean = region.trim().to_uppercase();
    match clean.as_str() {
        "CN" => Some("143465-19,29".to_string()),
        "US" => Some("143441-1,29".to_string()),
        "JP" => Some("143462-9,29".to_string()),
        "GB" | "UK" => Some("143444-2,29".to_string()),
        "DE" => Some("143443-2,29".to_string()),
        "FR" => Some("143442-3,29".to_string()),
        "CA" => Some("143455-6,29".to_string()),
        "AU" => Some("143460-14,29".to_string()),
        "HK" => Some("143463-20,29".to_string()),
        "TW" => Some("143470-20,29".to_string()),
        "KR" => Some("143466-19,29".to_string()),
        "SG" => Some("143464-2,29".to_string()),
        other => {
            if let Some(code) = region_to_storefront_code(other) {
                Some(format!("{}-1,29", code))
            } else if clean.chars().all(|c| c.is_ascii_digit()) && !clean.is_empty() {
                Some(format!("{}-1,29", clean))
            } else {
                None
            }
        }
    }
}

fn normalize_region_candidate(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let alpha: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_uppercase();
    if (2..=3).contains(&alpha.len()) {
        return Some(alpha);
    }

    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if let Some(region) = storefront_region(&digits) {
        return Some(region.to_string());
    }

    None
}

fn first_region_from_object(object: &serde_json::Map<String, Value>) -> Option<String> {
    [
        "countryCode",
        "countryISOCode",
        "country",
        "storefront",
        "storeFront",
        "storefrontId",
        "storeFrontId",
    ]
    .into_iter()
    .find_map(|key| object.get(key))
    .and_then(|value| match value {
        Value::String(s) => normalize_region_candidate(s),
        Value::Number(n) => normalize_region_candidate(&n.to_string()),
        _ => None,
    })
}

fn extract_region_from_headers(headers: &header::HeaderMap) -> Option<String> {
    [
        "x-set-apple-store-front",
        "x-apple-store-front",
        "x-apple-storefront",
    ]
    .into_iter()
    .find_map(|name| headers.get(name))
    .and_then(|value| value.to_str().ok())
    .and_then(normalize_region_candidate)
}

fn extract_account_metadata(
    result: &HashMap<String, Value>,
) -> (Option<String>, Option<String>, Option<String>) {
    let account_info = result
        .get("accountInfo")
        .and_then(|value| value.as_object());

    let email = account_info
        .and_then(|account| account.get("appleId"))
        .and_then(|value| value.as_str())
        .map(str::to_string);

    let address = account_info
        .and_then(|account| account.get("address"))
        .and_then(|value| value.as_object());

    let display_name = address
        .map(|address| {
            [
                address.get("firstName").and_then(|value| value.as_str()),
                address.get("lastName").and_then(|value| value.as_str()),
            ]
            .into_iter()
            .flatten()
            .filter(|part| !part.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
        })
        .filter(|name| !name.is_empty());

    let region = address
        .and_then(first_region_from_object)
        .or_else(|| account_info.and_then(first_region_from_object));

    (display_name, email, region)
}

/// Best-effort normalize Apple's XML-ish plist responses.
/// Some endpoints wrap a plist inside <Document> or return a bare <dict>.
fn normalize_apple_plist_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if let (Some(start), Some(end)) = (trimmed.find("<plist"), trimmed.rfind("</plist>")) {
        let end = end + "</plist>".len();
        return trimmed[start..end].trim().to_string();
    }

    if let (Some(start), Some(end)) = (trimmed.find("<dict"), trimmed.rfind("</dict>")) {
        let end = end + "</dict>".len();
        return trimmed[start..end].trim().to_string();
    }

    if trimmed.contains("<key>") {
        return format!("<dict>{}</dict>", trimmed);
    }

    trimmed.to_string()
}

/// Parse Apple's XML plist response into a HashMap<String, Value>.
/// Apple's auth/bag endpoints return XML plist (not JSON), so we need to convert.
fn parse_apple_plist_response(body: &str) -> Result<HashMap<String, Value>, String> {
    let normalized = normalize_apple_plist_body(body);

    let parsed: Result<plist::Value, _> = plist::from_bytes(normalized.as_bytes());
    match parsed {
        Ok(plist_val) => {
            let mut map = HashMap::new();
            if let Some(dict) = plist_val.as_dictionary() {
                for (key, val) in dict {
                    let json_val = plist_value_to_json(val);
                    map.insert(key.clone(), json_val);
                }
            }
            Ok(map)
        }
        Err(e) => {
            // Fallback: try JSON parse (in case Apple ever returns JSON)
            if body.trim().starts_with('{') {
                match serde_json::from_str::<HashMap<String, Value>>(body) {
                    Ok(m) => return Ok(m),
                    Err(je) => log::warn!("JSON fallback also failed: {}", je),
                }
            }

            // Neither plist nor JSON — log raw body for debugging
            log::error!(
                "Apple returned unparseable response ({} bytes): {:.300}",
                body.len(),
                body
            );
            Err(format!(
                "Failed to parse Apple response: {} (body_len={})",
                e,
                body.len()
            ))
        }
    }
}

/// Convert a plist::Value to serde_json::Value
fn plist_value_to_json(val: &plist::Value) -> Value {
    match val {
        plist::Value::String(s) => Value::String(s.clone()),
        plist::Value::Boolean(b) => Value::Bool(*b),
        plist::Value::Integer(i) => Value::String(i.to_string()),
        plist::Value::Real(f) => Value::String(format!("{}", f)),
        plist::Value::Data(d) => {
            use base64::Engine;
            Value::String(base64::engine::general_purpose::STANDARD.encode(d))
        }
        plist::Value::Date(d) => Value::String(format!("{:?}", d)),
        plist::Value::Array(arr) => Value::Array(arr.iter().map(plist_value_to_json).collect()),
        plist::Value::Dictionary(dict) => {
            let m: std::collections::HashMap<String, Value> = dict
                .iter()
                .map(|(k, v)| (k.clone(), plist_value_to_json(v)))
                .collect();
            Value::Object(m.into_iter().collect())
        }
        _ => Value::Null,
    }
}

fn build_xml_plist_body(fields: &[(&str, String)]) -> Result<Vec<u8>, String> {
    let mut dict = plist::Dictionary::new();
    for (k, v) in fields {
        dict.insert((*k).to_string(), plist::Value::String(v.clone()));
    }

    let mut buf = Vec::new();
    plist::to_writer_xml(&mut buf, &plist::Value::Dictionary(dict))
        .map_err(|e| format!("failed to encode plist body: {}", e))?;
    Ok(buf)
}

fn build_download_plist_body(
    appid: &str,
    guid: &str,
    version: Option<&str>,
) -> Result<Vec<u8>, String> {
    let id: i64 = appid
        .parse()
        .map_err(|_| "Apple app ID must be a positive integer")?;
    if id <= 0 {
        return Err("Apple app ID must be a positive integer".into());
    }
    let mut dict = plist::Dictionary::new();
    dict.insert("creditDisplay".into(), plist::Value::String(String::new()));
    dict.insert("guid".into(), plist::Value::String(guid.into()));
    dict.insert("salableAdamId".into(), plist::Value::Integer(id.into()));
    dict.insert("serialNumber".into(), plist::Value::String("0".into()));
    if let Some(version) = version {
        dict.insert(
            "externalVersionId".into(),
            plist::Value::String(version.into()),
        );
    }
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &plist::Value::Dictionary(dict)).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    pub ds_person_id: Option<String>,
    pub password_token: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub region: Option<String>,
}

/// Exact upstream session-expiry codes; never infer ownership from these failures.
pub fn is_session_auth_failure(result: &HashMap<String, Value>) -> bool {
    match result.get("failureType") {
        Some(Value::String(code)) => matches!(code.as_str(), "2034" | "2042"),
        Some(Value::Number(code)) => matches!(code.as_u64(), Some(2034 | 2042)),
        _ => false,
    }
}

#[derive(Serialize, Deserialize, Default)]
struct SessionSnapshot {
    #[serde(default)]
    cookies: Vec<Value>,
    #[serde(default)]
    pod: Option<String>,
}

fn valid_pod(pod: &str) -> bool {
    !pod.is_empty() && pod.len() <= 10 && pod.bytes().all(|c| c.is_ascii_digit())
}

#[derive(Debug, Clone)]
pub struct Store {
    pub client: Client,
    // All Store clones (including AccountStore clones) share the live session.
    cookies: Arc<reqwest_cookie_store::CookieStoreMutex>,
    pod: Arc<std::sync::Mutex<Option<String>>>,
    pub guid: String,
    /// SAP 签名会话：每次新登录重新建立，验证码步骤沿用同一个。
    signer: Arc<tokio::sync::Mutex<Option<SapSigner>>>,
}

impl Store {
    pub fn new() -> Self {
        Self::with_guid(Self::generate_guid())
    }

    /// 使用指定 GUID 创建 Store。passwordToken 与登录时的 GUID 绑定，
    /// 恢复会话时必须沿用原 GUID（容器重建后 MAC 地址可能变化）。
    pub fn with_guid(guid: String) -> Self {
        // IMPORTANT: Apple auth flow can return 302 redirects that must be handled
        // by retrying POST to the redirect location. Disable automatic redirect following.
        let cookies = Arc::new(reqwest_cookie_store::CookieStoreMutex::default());
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .cookie_provider(Arc::clone(&cookies))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        Store {
            client,
            cookies,
            pod: Arc::new(std::sync::Mutex::new(None)),
            guid,
            signer: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// Export the complete cookie/pod session as JSON. Treat the result as a secret.
    /// Include non-persistent session cookies: cookie_store's save_json omits them.
    pub fn export_session_cookies(&self) -> Result<String, String> {
        let cookies = self
            .cookies
            .lock()
            .map_err(|_| "Apple cookie jar lock poisoned")?;
        let pod = self.pod.lock().map_err(|_| "Apple pod lock poisoned")?;
        let records = cookies
            .iter_unexpired()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Cannot serialize Apple cookies")?;
        serde_json::to_string(&SessionSnapshot {
            cookies: records,
            pod: pod.clone(),
        })
        .map_err(|_| "Cannot serialize Apple session".to_string())
    }

    /// Replace the shared live session after validating the entire snapshot.
    /// Empty input and missing fields support older persisted token-only sessions.
    /// GUID and AuthInfo must be persisted separately by the caller.
    pub fn import_session_cookies(&self, serialized: &str) -> Result<(), String> {
        let snapshot: SessionSnapshot = if serialized.trim().is_empty() {
            SessionSnapshot::default()
        } else {
            serde_json::from_str(serialized).map_err(|_| "Invalid Apple session JSON")?
        };
        if snapshot.pod.as_deref().is_some_and(|pod| !valid_pod(pod)) {
            return Err("Invalid Apple session pod".into());
        }
        let mut lines = String::new();
        for cookie in snapshot.cookies {
            lines.push_str(&serde_json::to_string(&cookie).map_err(|_| "Invalid Apple cookie")?);
            lines.push('\n');
        }
        let restored = reqwest_cookie_store::CookieStore::load_json(std::io::Cursor::new(lines))
            .map_err(|_| "Invalid Apple session cookies")?;
        // Lock order matches export; no partially imported state on validation/lock failure.
        let mut cookies = self
            .cookies
            .lock()
            .map_err(|_| "Apple cookie jar lock poisoned")?;
        let mut pod = self.pod.lock().map_err(|_| "Apple pod lock poisoned")?;
        *cookies = restored;
        *pod = snapshot.pod;
        Ok(())
    }

    pub fn session_pod(&self) -> Option<String> {
        self.pod.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn observe_session_response(&self, url: &reqwest::Url, headers: &header::HeaderMap) {
        if approved_auth_endpoint(url.as_str()).is_none() {
            return;
        }
        let header_pod = headers
            .get("pod")
            .and_then(|v| v.to_str().ok())
            .filter(|pod| valid_pod(pod));
        let host_pod = url
            .host_str()
            .and_then(|host| host.strip_suffix("-buy.itunes.apple.com"))
            .and_then(|host| host.strip_prefix('p'))
            .filter(|pod| valid_pod(pod));
        if let Some(pod) = header_pod.or(host_pod) {
            *self.pod.lock().unwrap_or_else(|e| e.into_inner()) = Some(pod.to_string());
        }
    }

    fn store_endpoint(&self, action: &str) -> String {
        let pod = self.session_pod().unwrap_or_else(|| "25".into());
        let mut url = reqwest::Url::parse(&format!(
            "https://p{pod}-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/{action}"
        ))
        .expect("validated Apple pod");
        url.query_pairs_mut().append_pair("guid", &self.guid);
        url.to_string()
    }

    fn generate_guid() -> String {
        // Match ipatool reference: use MAC address (AABBCCDDEEFF) as GUID.
        // Best-effort on Linux: read /sys/class/net/*/address.
        if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name == "lo" {
                    continue;
                }

                let addr_path = entry.path().join("address");
                if let Ok(addr) = std::fs::read_to_string(addr_path) {
                    let mac = addr.trim();
                    if mac.len() >= 17 && mac.contains(':') {
                        let guid = mac.replace(':', "").to_uppercase();
                        if guid.len() == 12 {
                            return guid;
                        }
                    }
                }
            }
        }

        // Fallback: random but keep length compatible (12 hex chars)
        let raw = uuid::Uuid::new_v4()
            .to_string()
            .to_uppercase()
            .replace('-', "");
        raw.chars().take(12).collect()
    }

    fn base_headers() -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();
        headers.insert("User-Agent", USER_AGENT.parse().unwrap());
        headers
    }

    fn apple_plist_headers() -> header::HeaderMap {
        let mut headers = Self::base_headers();
        headers.insert("Content-Type", "application/x-apple-plist".parse().unwrap());
        headers.insert(
            header::ACCEPT,
            "application/x-apple-plist, text/xml, application/xml, */*"
                .parse()
                .unwrap(),
        );
        headers
    }

    fn ensure_guid_query(endpoint: &str, guid: &str) -> String {
        if endpoint.contains("guid=") {
            return endpoint.to_string();
        }
        if endpoint.contains('?') {
            format!("{}&guid={}", endpoint, guid)
        } else {
            format!("{}?guid={}", endpoint, guid)
        }
    }

    async fn resolve_auth_endpoint(&self) -> Result<String, String> {
        let bag_url = format!("https://init.itunes.apple.com/bag.xml?guid={}", self.guid);
        let response = self
            .client
            .get(&bag_url)
            .headers(Self::base_headers())
            .header("Accept", "application/xml")
            .send()
            .await
            .map_err(|e| format!("bag request failed: {}", e))?;

        let status = response.status();
        let body_text = response
            .text()
            .await
            .map_err(|e| format!("bag read body failed: {}", e))?;

        if status != StatusCode::OK {
            return Err(format!(
                "bag returned non-200 status={} body_len={}",
                status,
                body_text.len()
            ));
        }

        let parsed = parse_apple_plist_response(&body_text)?;

        // Newer bags put authenticateAccount at the root; older ones inside urlBag.
        let endpoint = parsed
            .get("authenticateAccount")
            .and_then(|v| v.as_str())
            .or_else(|| {
                parsed
                    .get("urlBag")
                    .and_then(|v| v.as_object())
                    .and_then(|obj| obj.get("authenticateAccount"))
                    .and_then(|v| v.as_str())
            })
            .ok_or_else(|| "bag missing authenticateAccount".to_string())?;

        Ok(endpoint.to_string())
    }

    /// 发送一次带 SAP 签名的登录请求；Apple 返回临时状态（204/404/429/5xx）时
    /// 按 10s、20s 退避重试，最多 3 次，并遵守不超过 30s 的 Retry-After。
    /// 每次重发都对请求体重新签名。
    async fn send_signed(
        &self,
        url: &str,
        body: &[u8],
        signer: &mut SapSigner,
    ) -> Result<reqwest::Response, String> {
        let mut statuses: Vec<u16> = Vec::new();
        loop {
            let signature = signer
                .sign(body)
                .map_err(|e| format!("Apple 登录签名失败：{}", e))?;
            let response = self
                .client
                .post(url)
                .headers(Self::base_headers())
                .header("Content-Type", "application/x-apple-plist")
                .header(
                    "X-Apple-ActionSignature",
                    base64::engine::general_purpose::STANDARD.encode(signature),
                )
                .body(body.to_vec())
                .send()
                .await
                .map_err(|e| {
                    if e.is_timeout() {
                        "连接 Apple 超时，请检查网络环境".to_string()
                    } else {
                        format!("连接 Apple 失败：{}", e)
                    }
                })?;

            let status = response.status().as_u16();
            if !is_transient_auth_status(status) {
                return Ok(response);
            }

            statuses.push(status);
            if statuses.len() == AUTH_REQUEST_ATTEMPTS {
                if status == 429 {
                    return Err("Apple 频率限制 (429)，请等待 15-30 分钟后再试".to_string());
                }
                let list = statuses
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "Apple 暂时无法处理登录请求（已尝试 {} 次：HTTP {}），请稍后再试",
                    AUTH_REQUEST_ATTEMPTS, list
                ));
            }

            let mut delay = std::cmp::min(
                AUTH_RETRY_DELAY * 2u32.pow(statuses.len() as u32 - 1),
                MAX_AUTH_RETRY_DELAY,
            );
            if let Some(requested) = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(retry_after)
            {
                if requested > MAX_AUTH_RETRY_DELAY {
                    return Err(format!(
                        "Apple 要求 {} 秒后再试，请稍后重新登录",
                        requested.as_secs()
                    ));
                }
                delay = std::cmp::max(requested, Duration::from_secs(1));
            }
            log::warn!(
                "Apple auth transient HTTP {} (attempt {}/{}), retrying in {}s",
                status,
                statuses.len(),
                AUTH_REQUEST_ATTEMPTS,
                delay.as_secs()
            );
            tokio::time::sleep(delay).await;
        }
    }

    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
        mfa: Option<&str>,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let endpoint = match self.resolve_auth_endpoint().await {
            Ok(ep) => normalize_auth_url(&ep),
            Err(e) => {
                log::warn!(
                    "Apple bag endpoint resolve failed (guid={}): {}. Falling back to {}",
                    self.guid,
                    e,
                    DEFAULT_AUTH_ENDPOINT
                );
                DEFAULT_AUTH_ENDPOINT.to_string()
            }
        };
        let mut url = match approved_auth_endpoint(&endpoint) {
            Some(url) => Self::ensure_guid_query(&url, &self.guid),
            None => {
                log::warn!("Unapproved auth endpoint {}, using default", endpoint);
                Self::ensure_guid_query(DEFAULT_AUTH_ENDPOINT, &self.guid)
            }
        };

        // Apple 要求登录请求带 SAP 签名。签名会话在整个登录过程中复用，
        // 包括第二步提交验证码（PENDING_MFA 中暂存的 Store 共享同一个 signer）。
        let mut signer_guard = self.signer.lock().await;
        if signer_guard.is_none() {
            match sap::connect(&self.client, USER_AGENT).await {
                Ok(signer) => *signer_guard = Some(signer),
                Err(e) => {
                    log::error!("SAP setup failed: {}", e);
                    return Ok(auth_failure(
                        "SapSetupError",
                        &format!("Apple 登录签名初始化失败：{}", e),
                    ));
                }
            }
        }
        let signer = signer_guard.as_mut().expect("signer initialized above");

        let code = mfa
            .map(|code| code.replace(' ', ""))
            .filter(|code| !code.is_empty());
        let auth_body = build_xml_plist_body(&[
            ("appleId", email.to_string()),
            (
                "attempt",
                if code.is_some() { "2" } else { "4" }.to_string(),
            ),
            ("guid", self.guid.clone()),
            (
                "password",
                format!("{}{}", password, code.as_deref().unwrap_or("")),
            ),
            ("rmp", "0".to_string()),
            ("why", "signIn".to_string()),
        ])
        .map_err(|e| format!("build auth plist failed: {}", e))?;

        // Best-effort region inference: some Apple flows only expose storefront headers
        // on intermediate redirect responses. Keep the last seen value across redirects.
        let mut inferred_region: Option<String> = None;
        let mut last_failure: Option<HashMap<String, Value>> = None;
        let mut result: Option<HashMap<String, Value>> = None;
        let mut attempt = 0;
        let mut redirects = 0;

        while attempt < 2 && redirects <= 3 {
            attempt += 1;
            log::info!(
                "Apple auth attempt {}: url={}, has_mfa={}, guid={}",
                attempt,
                url,
                code.is_some(),
                self.guid
            );

            let response = match self.send_signed(&url, &auth_body, signer).await {
                Ok(response) => response,
                Err(message) => {
                    log::warn!("Apple auth request failed: {}", message);
                    last_failure = Some(auth_failure("RequestError", &message));
                    if message.contains("429") || message.contains("要求") {
                        break;
                    }
                    continue;
                }
            };

            let status = response.status();
            self.observe_session_response(response.url(), response.headers());
            if let Some(region) = extract_region_from_headers(response.headers()) {
                inferred_region = Some(region);
            }

            // The native /fast host answers with 301 as well as 302; follow every redirect status.
            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                let redirect = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|location| reqwest::Url::parse(&url).ok()?.join(location).ok())
                    .and_then(|target| {
                        approved_auth_endpoint(&normalize_auth_url(target.as_str()))
                    });
                match redirect {
                    Some(target) => {
                        self.observe_session_response(
                            &reqwest::Url::parse(&target)?,
                            &header::HeaderMap::new(),
                        );
                        url = Self::ensure_guid_query(&target, &self.guid);
                        log::info!("Apple auth redirect ({}) -> {}", status, url);
                        attempt -= 1;
                        redirects += 1;
                    }
                    None => {
                        log::warn!("Apple auth redirect ({}) without usable Location", status);
                        last_failure = Some(auth_failure(
                            "RedirectError",
                            "Apple 登录重定向异常，请稍后重试",
                        ));
                    }
                }
                continue;
            }

            let body_text = response.text().await.unwrap_or_default();
            log::info!(
                "Apple auth response: status={}, body_len={}",
                status,
                body_text.len()
            );

            if status != StatusCode::OK || body_text.trim().is_empty() {
                last_failure = Some(auth_failure(
                    "UnexpectedResponse",
                    &format!(
                        "Apple 未返回有效的登录结果（HTTP {}），请稍后重试",
                        status.as_u16()
                    ),
                ));
                continue;
            }

            match parse_apple_plist_response(&body_text) {
                Ok(parsed) => {
                    let failure_type = parsed
                        .get("failureType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    // Apple may answer the first sign-in with InvalidCredentials (-5000)
                    // even for correct passwords; retry once.
                    if attempt == 1 && failure_type == "-5000" {
                        log::info!("Apple auth: InvalidCredentials on attempt 1, retrying");
                        last_failure = Some(parsed);
                        continue;
                    }
                    result = Some(parsed);
                    break;
                }
                Err(e) => {
                    log::error!("Failed to parse Apple response: {}", e);
                    last_failure = Some(auth_failure(
                        "ParseError",
                        "无法解析 Apple 的响应，请稍后重试",
                    ));
                }
            }
        }

        let Some(mut result) = result else {
            let failure = last_failure.unwrap_or_else(|| {
                auth_failure("UnknownError", "登录失败，Apple 未返回具体错误信息")
            });
            log::warn!(
                "Apple auth failure: type='{}', msg='{}'",
                failure
                    .get("failureType")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
                failure
                    .get("customerMessage")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
            );
            let mut failure = failure;
            failure.insert("_state".to_string(), Value::String("failure".to_string()));
            return Ok(failure);
        };

        if let Some(region) = inferred_region.clone() {
            log::info!("Apple auth inferred region from headers: {}", region);
            result.insert("region".to_string(), Value::String(region));
        }
        // Some failures only carry the text in dialog.explanation.
        if !result.contains_key("customerMessage") {
            if let Some(explanation) = result
                .get("dialog")
                .and_then(|v| v.as_object())
                .and_then(|dialog| dialog.get("explanation"))
                .cloned()
            {
                result.insert("customerMessage".to_string(), explanation);
            }
        }

        let mut final_result = result.clone();

        // Check for success
        let has_success = result.contains_key("dsPersonId") || result.contains_key("passwordToken");

        if has_success {
            let (display_name, account_email, region) = extract_account_metadata(&result);
            final_result.insert("_state".to_string(), Value::String("success".to_string()));
            if let Some(display_name) = display_name {
                final_result.insert("displayName".to_string(), Value::String(display_name));
            }
            if let Some(account_email) = account_email {
                final_result.insert("email".to_string(), Value::String(account_email));
            }
            if let Some(region) = region {
                final_result.insert("region".to_string(), Value::String(region));
            } else if let Some(region) = inferred_region.clone() {
                final_result.insert("region".to_string(), Value::String(region));
            }
            log::info!("Apple auth SUCCESS for {}", email);
        } else {
            final_result.insert("_state".to_string(), Value::String("failure".to_string()));
            let ft = result
                .get("failureType")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let cm = result
                .get("customerMessage")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            log::warn!("Apple auth failure: type='{}', msg='{}'", ft, cm);
        }

        Ok(final_result)
    }

    /// Preserve POST body and authentication across Apple's Store pod redirects.
    async fn post_store_plist(
        &self,
        endpoint: &str,
        headers: header::HeaderMap,
        body: Vec<u8>,
    ) -> Result<reqwest::Response, Box<dyn std::error::Error + Send + Sync>> {
        let mut url = reqwest::Url::parse(endpoint)?;
        for hop in 0..=5 {
            let response = self
                .client
                .post(url.clone())
                .headers(headers.clone())
                .body(body.clone())
                .send()
                .await?;
            self.observe_session_response(response.url(), response.headers());
            if !response.status().is_redirection() {
                if !response.status().is_success() {
                    return Err(format!(
                        "Apple Store 请求失败（HTTP {}）",
                        response.status().as_u16()
                    )
                    .into());
                }
                return Ok(response);
            }
            if hop == 5 {
                return Err("Apple Store 重定向次数过多".into());
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Apple Store 重定向缺少 Location")?;
            let target = url.join(location)?;
            let approved = target.scheme() == "https"
                && target.username().is_empty()
                && target.password().is_none()
                && target.port().is_none()
                && target.host_str().is_some_and(is_buy_host);
            let local_test = cfg!(test)
                && url.host_str() == Some("127.0.0.1")
                && target.origin() == url.origin();
            if !approved && !local_test {
                return Err("Apple Store 重定向目标不可信".into());
            }
            self.observe_session_response(&target, &header::HeaderMap::new());
            log::info!(
                "Apple Store pod redirect: status={}, host={}",
                response.status().as_u16(),
                target.host_str().unwrap_or("")
            );
            url = target;
        }
        unreachable!()
    }

    pub async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let url = self.store_endpoint("buyProduct");

        let mut purchase_data = vec![
            ("appExtVrsId", "0".to_string()),
            ("hasAskedToFulfillPreorder", "true".to_string()),
            ("buyWithoutAuthorization", "true".to_string()),
            ("hasDoneAgeCheck", "true".to_string()),
            ("guid", self.guid.clone()),
            ("needDiv", "0".to_string()),
            ("origPage", format!("Software-{}", app_identifier)),
            ("origPageLocation", "Buy".to_string()),
            ("price", "0".to_string()),
            ("pricingParameters", "STDQ".to_string()),
            ("productType", "C".to_string()),
            ("salableAdamId", app_identifier.to_string()),
        ];
        if let Some(ver_id) = app_ver_id {
            for (k, v) in purchase_data.iter_mut() {
                if *k == "appExtVrsId" {
                    *v = ver_id.to_string();
                }
            }
            purchase_data.push(("externalVersionId", ver_id.to_string()));
        }

        let purchase_body = build_xml_plist_body(&purchase_data)
            .map_err(|e| format!("failed to build purchase plist body: {}", e))?;

        let mut headers = Self::apple_plist_headers();
        if let Some(ds_id) = &auth_info.ds_person_id {
            headers.insert("X-Dsid", ds_id.parse().unwrap());
            headers.insert("iCloud-DSID", ds_id.parse().unwrap());
        }
        if let Some(token) = &auth_info.password_token {
            headers.insert("X-Token", token.parse().unwrap());
        }
        if let Some(region) = &auth_info.region {
            if let Some(sf) = region_to_storefront_header(region) {
                if let Ok(hdr_val) = sf.parse() {
                    headers.insert("X-Apple-Store-Front", hdr_val);
                }
            }
        }

        let response = self.post_store_plist(&url, headers, purchase_body).await?;

        let body_text = response.text().await?;
        let mut result = parse_apple_plist_response(&body_text)
            .map_err(|e| format!("failed to parse purchase response: {}", e))?;
        let has_failure = result
            .get("failureType")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        result.insert(
            "_state".to_string(),
            Value::String(if has_failure { "failure" } else { "success" }.to_string()),
        );
        Ok(result)
    }

    pub async fn download_product(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let url = self.store_endpoint("volumeStoreDownloadProduct");

        let download_body = build_download_plist_body(app_identifier, &self.guid, app_ver_id)
            .map_err(|e| format!("failed to build download plist body: {}", e))?;

        let mut headers = Self::apple_plist_headers();
        if let Some(ds_id) = &auth_info.ds_person_id {
            headers.insert("X-Dsid", ds_id.parse().unwrap());
            headers.insert("iCloud-DSID", ds_id.parse().unwrap());
        }
        if let Some(token) = &auth_info.password_token {
            headers.insert("X-Token", token.parse().unwrap());
        }
        if let Some(region) = &auth_info.region {
            if let Some(sf) = region_to_storefront_header(region) {
                if let Ok(hdr_val) = sf.parse() {
                    headers.insert("X-Apple-Store-Front", hdr_val);
                }
            }
        }

        let response = self
            .post_store_plist(&url, headers.clone(), download_body)
            .await?;

        let body_text = response.text().await?;
        let mut result = parse_apple_plist_response(&body_text)
            .map_err(|e| format!("failed to parse download response: {}", e))?;
        if crate::apple_download_recovery::needs_download_recovery(&result) {
            log::info!("Apple legacy download returned no usable item; requesting device-bound DownloadDispatch authorization");
            result = crate::apple_download_recovery::recover(
                &self.client,
                headers,
                &self.guid,
                app_identifier,
                app_ver_id,
                auth_info
                    .region
                    .as_deref()
                    .ok_or("下载恢复需要账号商店区域，请重新登录")?,
            )
            .await
            .map_err(|e| format!("Apple 下载授权恢复失败: {e}"))?;
        }
        let has_failure = result
            .get("failureType")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_song_list = result
            .get("songList")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        result.insert(
            "_state".to_string(),
            Value::String(
                if has_failure || !has_song_list {
                    "failure"
                } else {
                    "success"
                }
                .to_string(),
            ),
        );
        Ok(result)
    }
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct AccountStore {
    pub store: Store,
    pub account_email: String,
    pub auth_info: Option<AuthInfo>,
    /// 上次成功登录的时间（Unix 秒），用于展示"登录于 X 天前"
    pub authenticated_at: i64,
}

impl AccountStore {
    pub fn new(email: &str) -> Self {
        AccountStore {
            store: Store::new(),
            account_email: email.to_string(),
            auth_info: None,
            authenticated_at: chrono::Utc::now().timestamp(),
        }
    }

    /// 从持久化的登录态恢复账号（服务重启后使用）。
    pub fn restore(email: &str, guid: String, auth_info: AuthInfo, authenticated_at: i64) -> Self {
        AccountStore {
            store: Store::with_guid(guid),
            account_email: email.to_string(),
            auth_info: Some(auth_info),
            authenticated_at,
        }
    }

    pub async fn authenticate(
        &mut self,
        password: &str,
        mfa: Option<&str>,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let result = self
            .store
            .authenticate(&self.account_email, password, mfa)
            .await?;

        // 提取认证信息
        if result.get("_state").and_then(|v| v.as_str()) == Some("success") {
            let auth_info = AuthInfo {
                ds_person_id: result
                    .get("dsPersonId")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                password_token: result
                    .get("passwordToken")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                display_name: result
                    .get("displayName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                email: Some(self.account_email.clone()),
                region: result
                    .get("region")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            };
            self.auth_info = Some(auth_info);
            self.authenticated_at = chrono::Utc::now().timestamp();
        }

        Ok(result)
    }

    pub async fn download_product(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let auth_info = self.auth_info.as_ref().ok_or("Not authenticated")?;
        self.store
            .download_product(app_identifier, app_ver_id, auth_info)
            .await
    }

    pub async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let auth_info = self.auth_info.as_ref().ok_or("Not authenticated")?;
        self.store
            .ensure_license(app_identifier, app_ver_id, auth_info)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie_header(store: &Store, url: &str) -> String {
        use reqwest::cookie::CookieStore;
        store
            .cookies
            .cookies(&reqwest::Url::parse(url).unwrap())
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default()
    }

    #[test]
    fn session_roundtrip_preserves_session_cookies_and_scope() {
        use reqwest::cookie::CookieStore;
        let store = Store::with_guid("TESTGUID".into());
        let url = reqwest::Url::parse("https://p28-buy.itunes.apple.com/auth").unwrap();
        let headers: Vec<header::HeaderValue> = [
            "session=one; Domain=itunes.apple.com; Path=/; Secure; HttpOnly",
            "host=two; Path=/private; Secure; Max-Age=3600",
            "expired=gone; Path=/; Max-Age=0",
        ]
        .iter()
        .map(|v| v.parse().unwrap())
        .collect();
        store.cookies.set_cookies(&mut headers.iter(), &url);
        store.observe_session_response(&url, &header::HeaderMap::new());
        let restored = Store::with_guid("TESTGUID".into());
        restored
            .import_session_cookies(&store.export_session_cookies().unwrap())
            .unwrap();
        assert!(
            cookie_header(&restored, "https://p28-buy.itunes.apple.com/private/item")
                .contains("host=two")
        );
        assert!(
            !cookie_header(&restored, "https://p28-buy.itunes.apple.com/privateer")
                .contains("host=two")
        );
        assert!(
            !cookie_header(&restored, "https://p29-buy.itunes.apple.com/private/item")
                .contains("host=two")
        );
        assert!(
            cookie_header(&restored, "https://p29-buy.itunes.apple.com/").contains("session=one")
        );
        assert!(
            cookie_header(&restored, "http://p28-buy.itunes.apple.com/private/item").is_empty()
        );
        assert!(cookie_header(&restored, "https://evil.example/").is_empty());
        assert!(!restored
            .export_session_cookies()
            .unwrap()
            .contains("expired"));
        assert_eq!(restored.session_pod().as_deref(), Some("28"));
        let snapshot: Value =
            serde_json::from_str(&restored.export_session_cookies().unwrap()).unwrap();
        assert!(snapshot["cookies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["raw_cookie"].as_str().unwrap_or("").contains("HttpOnly")));
    }

    #[test]
    fn session_import_is_atomic_and_legacy_empty_is_supported() {
        let store = Store::new();
        store.import_session_cookies("").unwrap();
        store.import_session_cookies("{}").unwrap();
        store
            .import_session_cookies(r#"{"pod":"28","cookies":[]}"#)
            .unwrap();
        let before = store.export_session_cookies().unwrap();
        for bad in [
            "not-json",
            r#"{"pod":"28.evil.example","cookies":[]}"#,
            r#"{"pod":"29","cookies":[{}]}"#,
        ] {
            assert!(store.import_session_cookies(bad).is_err());
            assert_eq!(store.export_session_cookies().unwrap(), before);
        }
        let old: AuthInfo = serde_json::from_str(r#"{"ds_person_id":"123","password_token":"test","display_name":null,"email":null,"region":"US"}"#).unwrap();
        assert_eq!(old.region.as_deref(), Some("US"));
    }

    #[test]
    fn pod_endpoint_tracks_headers_hosts_and_clones_without_accepting_untrusted_hosts() {
        let store = Store::with_guid("ABCD".into());
        assert_eq!(
            store.store_endpoint("buyProduct"),
            "https://p25-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/buyProduct?guid=ABCD"
        );
        let clone = store.clone();
        let mut headers = header::HeaderMap::new();
        headers.insert("pod", "49".parse().unwrap());
        store.observe_session_response(
            &reqwest::Url::parse(DEFAULT_AUTH_ENDPOINT).unwrap(),
            &headers,
        );
        assert!(clone
            .store_endpoint("volumeStoreDownloadProduct")
            .contains("p49-buy.itunes.apple.com"));
        store.observe_session_response(
            &reqwest::Url::parse("https://p28-buy.itunes.apple.com/redirect").unwrap(),
            &header::HeaderMap::new(),
        );
        assert_eq!(clone.session_pod().as_deref(), Some("28"));
        headers.insert("pod", "99".parse().unwrap());
        store.observe_session_response(
            &reqwest::Url::parse("https://evil.example/").unwrap(),
            &headers,
        );
        assert_eq!(clone.session_pod().as_deref(), Some("28"));
        store
            .import_session_cookies(r#"{"pod":"71","cookies":[]}"#)
            .unwrap();
        assert_eq!(clone.session_pod().as_deref(), Some("71"));
    }

    #[test]
    fn auth_failure_codes_are_exact_not_message_substrings() {
        for code in [serde_json::json!("2042"), serde_json::json!(2034)] {
            assert!(is_session_auth_failure(&HashMap::from([(
                "failureType".into(),
                code
            )])));
        }
        for code in ["12042", "20420", "1008", "2042 other"] {
            assert!(!is_session_auth_failure(&HashMap::from([(
                "failureType".into(),
                serde_json::json!(code)
            )])));
        }
        assert!(!is_session_auth_failure(&HashMap::from([(
            "customerMessage".into(),
            serde_json::json!("2042 Sign In")
        )])));
    }

    #[tokio::test]
    async fn response_cookies_survive_redirect_and_reopen_on_real_client() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for step in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 2048];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(bytes).unwrap().to_lowercase();
                if step > 0 {
                    assert!(
                        request.contains("cookie: apple-session=test-only"),
                        "{request}"
                    );
                }
                let reply = if step == 0 {
                    "HTTP/1.1 302 Found\r\nLocation: /store/pod\r\nSet-Cookie: apple-session=test-only; Path=/store; HttpOnly\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK"
                };
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        let store = Store::new();
        store
            .post_store_plist(
                &format!("http://{address}/auth"),
                Store::apple_plist_headers(),
                vec![],
            )
            .await
            .unwrap();
        let restored = Store::with_guid(store.guid.clone());
        restored
            .import_session_cookies(&store.export_session_cookies().unwrap())
            .unwrap();
        restored
            .post_store_plist(
                &format!("http://{address}/store/reopened"),
                Store::apple_plist_headers(),
                vec![],
            )
            .await
            .unwrap();
        server.await.unwrap();
    }

    #[test]
    fn import_does_not_revive_expired_cookie_or_extend_absolute_expiry() {
        use reqwest::cookie::CookieStore;
        let store = Store::new();
        let url = reqwest::Url::parse("https://p28-buy.itunes.apple.com/").unwrap();
        let header = "persistent=test-only; Max-Age=3600; Secure"
            .parse()
            .unwrap();
        store
            .cookies
            .set_cookies(&mut std::iter::once(&header), &url);
        let serialized = store.export_session_cookies().unwrap();
        let mut snapshot: Value = serde_json::from_str(&serialized).unwrap();
        let expiration = snapshot["cookies"][0]["expires"].clone();
        let restored = Store::new();
        restored.import_session_cookies(&serialized).unwrap();
        let exported: Value =
            serde_json::from_str(&restored.export_session_cookies().unwrap()).unwrap();
        assert_eq!(exported["cookies"][0]["expires"], expiration);
        snapshot["cookies"][0]["expires"] = serde_json::json!({"AtUtc":"2000-01-01T00:00:00Z"});
        restored
            .import_session_cookies(&snapshot.to_string())
            .unwrap();
        assert!(cookie_header(&restored, url.as_str()).is_empty());
    }

    #[test]
    fn normalizes_auth_urls_with_trailing_slash() {
        assert_eq!(
            normalize_auth_url(
                "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate"
            ),
            "https://buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
        );
        assert_eq!(
            normalize_auth_url(
                "https://p71-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate?guid=AB"
            ),
            "https://p71-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/?guid=AB"
        );
        assert_eq!(
            normalize_auth_url("https://auth.itunes.apple.com/auth/v1/native"),
            "https://auth.itunes.apple.com/auth/v1/native/fast/"
        );
        assert_eq!(
            normalize_auth_url("https://auth.itunes.apple.com/auth/v1/native/fast"),
            "https://auth.itunes.apple.com/auth/v1/native/fast/"
        );
    }

    #[test]
    fn only_approves_apple_https_auth_hosts() {
        assert!(approved_auth_endpoint(DEFAULT_AUTH_ENDPOINT).is_some());
        assert!(approved_auth_endpoint(
            "https://p25-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/authenticate/"
        )
        .is_some());
        assert!(approved_auth_endpoint("http://buy.itunes.apple.com/").is_none());
        assert!(approved_auth_endpoint("https://evil.example.com/").is_none());
        assert!(approved_auth_endpoint("https://pX-buy.itunes.apple.com/").is_none());
        assert!(approved_auth_endpoint("https://buy.itunes.apple.com:8443/").is_none());
    }

    #[test]
    fn treats_only_temporary_statuses_as_transient() {
        for status in [204, 404, 429, 500, 503] {
            assert!(is_transient_auth_status(status));
        }
        for status in [200, 301, 302, 403] {
            assert!(!is_transient_auth_status(status));
        }
        assert_eq!(retry_after("12"), Some(Duration::from_secs(12)));
    }

    #[tokio::test]
    async fn store_post_preserves_body_and_headers_across_pod_redirect() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for step in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 2048];
                    let n = socket.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(split) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..split]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() >= split + 4 + length {
                            break;
                        }
                    }
                }
                let request = String::from_utf8(bytes).unwrap();
                assert!(request.starts_with("POST "));
                assert!(request
                    .to_lowercase()
                    .contains("x-apple-store-front: 143441-1,29"));
                assert!(request.to_lowercase().contains("x-token: test-only"));
                assert!(request.ends_with("<plist>same-body</plist>"));
                if step == 0 {
                    socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /pod28?Pod=28\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                } else {
                    assert!(request.starts_with("POST /pod28?Pod=28 "));
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        let store = Store::new();
        let mut headers = Store::apple_plist_headers();
        headers.insert("X-Apple-Store-Front", "143441-1,29".parse().unwrap());
        headers.insert("X-Token", "test-only".parse().unwrap());
        let response = store
            .post_store_plist(
                &format!("http://{address}/pod25"),
                headers,
                b"<plist>same-body</plist>".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.unwrap(), "OK");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn store_post_rejects_untrusted_redirect_and_http_failure() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (reply, expected) in [
            ("HTTP/1.1 302 Found\r\nLocation: https://evil.example/steal\r\nContent-Length: 0\r\n\r\n", "不可信"),
            ("HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n", "Location"),
            ("HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n", "503"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let task = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending a request");
                socket.write_all(reply.as_bytes()).await.unwrap();
            });
            let error = Store::new().post_store_plist(&format!("http://{address}/store"), Store::apple_plist_headers(), vec![]).await.unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            task.await.unwrap();
        }
    }

    #[test]
    fn legacy_download_payload_preserves_apple_field_types() {
        let bytes =
            build_download_plist_body("1596063349", "AABBCCDDEEFF", Some("893559076")).unwrap();
        let value: plist::Value = plist::from_bytes(&bytes).unwrap();
        let dict = value.as_dictionary().unwrap();
        assert_eq!(dict["salableAdamId"].as_signed_integer(), Some(1596063349));
        assert_eq!(dict["serialNumber"].as_string(), Some("0"));
        assert_eq!(dict["externalVersionId"].as_string(), Some("893559076"));
        assert!(build_download_plist_body("not-an-id", "AABBCCDDEEFF", None).is_err());
    }

    #[test]
    fn maps_regions_to_storefront_headers() {
        assert_eq!(
            region_to_storefront_header("CN"),
            Some("143465-19,29".to_string())
        );
        assert_eq!(
            region_to_storefront_header("US"),
            Some("143441-1,29".to_string())
        );
        assert_eq!(
            region_to_storefront_header("jp"),
            Some("143462-9,29".to_string())
        );
        assert_eq!(
            region_to_storefront_header("DE"),
            Some("143443-2,29".to_string())
        );
        assert_eq!(
            region_to_storefront_header("AE"),
            Some("143481-1,29".to_string())
        );
    }
}
