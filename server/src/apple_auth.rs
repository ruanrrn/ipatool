use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

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

        Store { client, guid }
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
        headers.insert(
            "User-Agent",
            // Match ipatool reference: Configurator/2.17 + macOS 15.2
            "Configurator/2.17 (Macintosh; OS X 15.2; 24C5089c) AppleWebKit/0620.1.16.11.6"
                .parse()
                .unwrap(),
        );
        headers
    }

    fn form_headers() -> header::HeaderMap {
        let mut headers = Self::base_headers();
        headers.insert(
            "Content-Type",
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
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

    fn resolve_redirect_url(current_url: &str, location: &str, guid: &str) -> Option<String> {
        let candidate = match reqwest::Url::parse(location) {
            Ok(url) => url,
            Err(_) => {
                let base = reqwest::Url::parse(current_url).ok()?;
                base.join(location).ok()?
            }
        };

        Some(Self::ensure_guid_query(candidate.as_str(), guid))
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

        let endpoint = parsed
            .get("urlBag")
            .and_then(|v| v.as_object())
            .and_then(|obj| obj.get("authenticateAccount"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "bag missing urlBag.authenticateAccount".to_string())?;

        Ok(endpoint.to_string())
    }

    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
        mfa: Option<&str>,
    ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
        // Prefer bag.xml-derived auth endpoint (more stable across regions/redirects).
        let fallback = format!(
            "https://auth.itunes.apple.com/auth/v1/native/fast?guid={}",
            self.guid
        );

        let endpoint = match self.resolve_auth_endpoint().await {
            Ok(ep) => ep,
            Err(e) => {
                log::warn!(
                    "Apple bag endpoint resolve failed (guid={}): {}. Falling back to {}",
                    self.guid,
                    e,
                    fallback
                );
                fallback
            }
        };

        let mut url = Self::ensure_guid_query(&endpoint, &self.guid);
        let mut last_result: Option<HashMap<String, Value>> = None;

        // Best-effort region inference: some Apple flows only expose storefront headers
        // on intermediate 302 responses. Keep the last seen value across redirects.
        let mut inferred_region: Option<String> = None;

        for attempt in 1..=4u32 {
            // Exponential backoff between retries: 1s, 2s, 4s
            if attempt > 1 {
                let delay_secs = 1u64 << (attempt - 2); // 1, 2, 4
                log::debug!(
                    "Apple auth retry backoff: {}s before attempt {}",
                    delay_secs,
                    attempt
                );
                tokio::time::sleep(std::time::Duration::from_secs(delay_secs)).await;
            }

            let combined_password = format!("{}{}", password, mfa.unwrap_or("").replace(' ', ""));

            let auth_body = build_xml_plist_body(&[
                ("appleId", email.to_string()),
                ("attempt", attempt.to_string()),
                ("createSession", "true".to_string()),
                ("guid", self.guid.clone()),
                ("password", combined_password),
                ("rmp", "0".to_string()),
                ("why", "signIn".to_string()),
            ])
            .map_err(|e| format!("build auth plist failed: {}", e))?;

            log::info!(
                "Apple auth attempt {}/4: url={}, has_mfa={}, guid={}",
                attempt,
                url,
                mfa.is_some(),
                self.guid
            );

            let response = self
                .client
                .post(&url)
                .headers(Self::form_headers())
                .body(auth_body)
                .send()
                .await?;

            let status = response.status();
            let response_region = extract_region_from_headers(response.headers());
            if let Some(region) = response_region.clone() {
                inferred_region = Some(region);
            }

            // Handle 302 redirect — follow to new URL and retry
            if status == StatusCode::FOUND || status == StatusCode::MOVED_PERMANENTLY {
                if let Some(location) = response.headers().get("location") {
                    if let Ok(loc_str) = location.to_str() {
                        if let Some(redirect_url) =
                            Self::resolve_redirect_url(&url, loc_str, &self.guid)
                        {
                            url = redirect_url;
                            log::info!("Apple auth redirect -> {}", url);
                            continue;
                        }
                    }
                }

                let mut redirect_failure = HashMap::new();
                redirect_failure.insert("_state".to_string(), Value::String("failure".to_string()));
                redirect_failure.insert(
                    "failureType".to_string(),
                    Value::String("RedirectError".to_string()),
                );
                redirect_failure.insert(
                    "customerMessage".to_string(),
                    Value::String("Apple 登录重定向异常，请重新开始登录流程".to_string()),
                );
                return Ok(redirect_failure);
            }

            let body_text = response.text().await.unwrap_or_default();

            log::info!(
                "Apple auth response: status={}, body_len={}",
                status,
                body_text.len()
            );

            // Detect non-plist error responses before parsing
            if status == StatusCode::TOO_MANY_REQUESTS {
                log::warn!(
                    "Apple auth rate limited (attempt {}/4): {}",
                    attempt,
                    body_text.trim()
                );
                let mut m = HashMap::new();
                m.insert("_state".to_string(), Value::String("failure".to_string()));
                m.insert(
                    "failureType".to_string(),
                    Value::String("RateLimited".to_string()),
                );
                m.insert(
                    "customerMessage".to_string(),
                    Value::String("Apple 频率限制 (429)，请等待 15-30 分钟后再试".to_string()),
                );
                // Don't retry on 429 — each retry burns more quota
                return Ok(m);
            }

            // Parse plist response
            let mut result = match parse_apple_plist_response(&body_text) {
                Ok(m) => m,
                Err(e) => {
                    log::error!(
                        "Failed to parse Apple response ({} bytes, attempt {}): {}",
                        body_text.len(),
                        attempt,
                        e
                    );
                    let mut m = HashMap::new();
                    m.insert("_state".to_string(), Value::String("failure".to_string()));
                    m.insert(
                        "failureType".to_string(),
                        Value::String("ParseError".to_string()),
                    );
                    let hint = if body_text.contains("Rate limit") {
                        "Apple 频率限制 (429)，请等待 15-30 分钟后再试"
                    } else if body_text.len() < 200 {
                        &body_text
                    } else {
                        "无法解析 Apple 的响应，请稍后重试"
                    };
                    m.insert(
                        "customerMessage".to_string(),
                        Value::String(hint.to_string()),
                    );
                    return Ok(m);
                }
            };

            if let Some(region) = inferred_region.clone().or(response_region.clone()) {
                log::info!("Apple auth inferred region from headers: {}", region);
                result.insert("region".to_string(), Value::String(region));
            }

            let failure_type = result
                .get("failureType")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Match ipatool reference behavior:
            // attempt 1 may return InvalidCredentials (-5000) even for correct passwords.
            // auto-retry once (or until attempt limit).
            if attempt == 1 && failure_type == "-5000" {
                log::info!("Apple auth: InvalidCredentials on attempt 1, auto-retrying");
                last_result = Some(result);
                continue;
            }

            last_result = Some(result);
            break;
        }

        let result = last_result.unwrap_or_default();
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
    /// 记录上次成功认证的时间，用于自动刷新判断
    pub last_authenticated_at: std::time::Instant,
}

impl AccountStore {
    pub fn new(email: &str) -> Self {
        AccountStore {
            store: Store::new(),
            account_email: email.to_string(),
            auth_info: None,
            last_authenticated_at: std::time::Instant::now(),
        }
    }

    /// 从持久化的登录态恢复账号（服务重启后使用）。
    pub fn restore(email: &str, guid: String, auth_info: AuthInfo) -> Self {
        AccountStore {
            store: Store::with_guid(guid),
            account_email: email.to_string(),
            auth_info: Some(auth_info),
            last_authenticated_at: std::time::Instant::now(),
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
            self.last_authenticated_at = std::time::Instant::now();
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

    /// 标记认证时间为"刚刚"，供外部刷新成功后调用
    pub fn touch_authenticated(&mut self) {
        self.last_authenticated_at = std::time::Instant::now();
    }
}
