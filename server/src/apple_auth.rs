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
                "Apple returned unparseable response ({} bytes): {:300}",
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthInfo {
    pub ds_person_id: Option<String>,
    pub password_token: Option<String>,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub region: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Store {
    pub client: Client,
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
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .cookie_store(true)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        Store {
            client,
            guid,
            signer: Arc::new(tokio::sync::Mutex::new(None)),
        }
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

    pub async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "https://p25-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/buyProduct?guid={}",
            self.guid
        );

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

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .body(purchase_body)
            .send()
            .await?;

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
        let url = format!(
            "https://p25-buy.itunes.apple.com/WebObjects/MZFinance.woa/wa/volumeStoreDownloadProduct?guid={}",
            self.guid
        );

        let mut download_data = vec![
            ("creditDisplay", "".to_string()),
            ("guid", self.guid.clone()),
            ("salableAdamId", app_identifier.to_string()),
        ];
        if let Some(ver_id) = app_ver_id {
            download_data.push(("externalVersionId", ver_id.to_string()));
        }

        let download_body = build_xml_plist_body(&download_data)
            .map_err(|e| format!("failed to build download plist body: {}", e))?;

        let mut headers = Self::apple_plist_headers();
        if let Some(ds_id) = &auth_info.ds_person_id {
            headers.insert("X-Dsid", ds_id.parse().unwrap());
            headers.insert("iCloud-DSID", ds_id.parse().unwrap());
        }
        if let Some(token) = &auth_info.password_token {
            headers.insert("X-Token", token.parse().unwrap());
        }

        let response = self
            .client
            .post(&url)
            .headers(headers)
            .body(download_body)
            .send()
            .await?;

        let body_text = response.text().await?;
        let mut result = parse_apple_plist_response(&body_text)
            .map_err(|e| format!("failed to parse download response: {}", e))?;
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
}
