//! Restricted recovery for Apple's empty download response.
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::collections::HashMap;

pub fn needs_download_recovery(map: &HashMap<String, Value>) -> bool {
    if map
        .get("songList")
        .is_some_and(|v| !v.is_array() || v.as_array().is_some_and(|a| !a.is_empty()))
    {
        return false;
    }
    let failure = map.get("failureType").map(scalar).unwrap_or_default();
    if !failure.is_empty() && failure != "5002" {
        return false;
    }
    let message = map
        .get("customerMessage")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    message.is_empty()
        || message == "no longer available"
        || message.ends_with(" no longer available")
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

const DISPATCH_HOST: &str = "downloaddispatch.itunes.apple.com";
const ENT_PATH: &str = "/WebObjects/DownloadDispatch.woa/wa/ent/download";
const MAX_HELPER_OUTPUT: usize = 1024 * 1024;
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
const USER_AGENT: &str =
    "Configurator/2.18 (Macintosh; OS X 15.3.2; 24D81) AppleWebKit/0620.2.4.11.6";

fn payload(guid: &str, appid: &str, version: &str, blob: Option<&str>) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let mut d = plist::Dictionary::new();
    for (k, v) in [
        ("creditDisplay", ""),
        ("guid", guid),
        ("salableAdamId", appid),
    ] {
        d.insert(k.into(), v.into());
    }
    if let Some(blob) = blob {
        let hardware = hex::decode(guid).map_err(|_| "invalid device GUID")?;
        if hardware.len() != 6 {
            return Err("ent/download requires a six-byte device GUID".into());
        }
        // Match upstream hardwareID[2:] (four trailing bytes).
        let mut serial = vec![0x54, 0xc8, 0xb0, 0xa9, 0x88];
        serial.extend_from_slice(&hardware[2..]);
        d.insert(
            "serialNumber".into(),
            base64::engine::general_purpose::STANDARD
                .encode(serial)
                .into(),
        );
        d.insert("kbsync".into(), blob.into());
        d.insert("externalVersionId".into(), version.into());
    } else {
        d.insert(
            "salableAdamId".into(),
            plist::Value::Integer(appid.parse::<u64>().map_err(|_| "invalid app ID")?.into()),
        );
        d.insert("serialNumber".into(), "0".into());
        d.insert("appExtVrsId".into(), version.into());
    }
    let mut out = Vec::new();
    plist::to_writer_xml(&mut out, &d).map_err(|_| "encode download plist")?;
    Ok(out)
}

fn validate(
    mut map: HashMap<String, Value>,
    appid: &str,
    version: &str,
) -> Result<HashMap<String, Value>, String> {
    if map
        .get("failureType")
        .is_some_and(|v| !scalar(v).is_empty())
    {
        // Preserve Apple's structured failure for the caller's auth/license handling.
        // This is terminal for this endpoint: never hide it by trying legacy.
        map.insert("_state".into(), Value::String("failure".into()));
        return Ok(map);
    }
    if map
        .get("customerMessage")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty())
    {
        return Err("download recovery returned an Apple customer error".into());
    }
    let items = map
        .get("songList")
        .and_then(Value::as_array)
        .ok_or("download response must contain exactly one item")?;
    if items.len() != 1 {
        return Err("download response must contain exactly one item".into());
    }
    let metadata = &items[0]["metadata"];
    if scalar(&metadata["itemId"]) != appid
        || scalar(&metadata["softwareVersionExternalIdentifier"]) != version
    {
        return Err("download response does not match requested app or version".into());
    }
    if metadata["softwareVersionBundleId"]
        .as_str()
        .is_none_or(str::is_empty)
    {
        return Err("download response has no bundle identifier".into());
    }
    let url = items[0]["URL"]
        .as_str()
        .ok_or("download response has no package URL")?;
    if url.is_empty() {
        return Err("download response has no package URL".into());
    }
    map.insert("_state".into(), Value::String("success".into()));
    Ok(map)
}

fn offer_version(value: &Value, appid: &str) -> Result<Option<String>, String> {
    let Some(offers) = value["results"][appid]["offers"].as_array() else {
        return Ok(None);
    };
    let Some(offer) = offers.first() else {
        return Ok(None);
    };
    let mut version = scalar(&offer["version"]["externalId"]);
    if version.is_empty() {
        if let Some(params) = offer["buyParams"].as_str() {
            version = reqwest::Url::parse(&format!("https://example.invalid/?{params}"))
                .map_err(|_| "invalid MDM buy params")?
                .query_pairs()
                .find(|(k, _)| k == "appExtVrsId")
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
        }
    }
    if version.is_empty() {
        return Err("MDM offer has no external version ID".into());
    }
    numeric_id(&version, "external version ID")?;
    Ok(Some(version))
}

fn numeric_id(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || !value.bytes().all(|c| c.is_ascii_digit())
        || value.parse::<u64>().ok().filter(|v| *v > 0).is_none()
    {
        return Err(format!("invalid {name}"));
    }
    Ok(())
}

fn parse_helper(bytes: &[u8]) -> Result<String, String> {
    use base64::Engine;
    if bytes.len() > MAX_HELPER_OUTPUT {
        return Err("kbsync helper output exceeds limit".into());
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "invalid kbsync helper JSON")?;
    let blob = value["kbsync"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("kbsync helper returned no blob")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .map_err(|_| "kbsync helper returned invalid base64")?;
    if decoded.is_empty() {
        return Err("kbsync helper returned an empty blob".into());
    }
    Ok(blob.into())
}

async fn bounded_response(mut response: reqwest::Response) -> Result<Vec<u8>, String> {
    let url = response.url();
    let target = format!("{}{}", url.host_str().unwrap_or("unknown"), url.path());
    if response.status() != reqwest::StatusCode::OK {
        return Err(format!(
            "Apple HTTP {} {target}",
            response.status().as_u16()
        ));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("read Apple response {target}"))?
    {
        if data.len() + chunk.len() > MAX_RESPONSE {
            return Err(format!("Apple response exceeds limit {target}"));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

// Store client MUST use Policy::none(): reqwest has no per-request override.
async fn dispatch(
    client: &reqwest::Client,
    url: &str,
    headers: HeaderMap,
    guid: &str,
    appid: &str,
    version: &str,
    blob: Option<&str>,
) -> Result<HashMap<String, Value>, String> {
    let mut safe = HeaderMap::new();
    for key in ["x-dsid", "icloud-dsid"] {
        if let Some(v) = headers.get(key) {
            safe.insert(reqwest::header::HeaderName::from_static(key), v.clone());
        }
    }
    if blob.is_some() {
        for key in ["x-token", "x-apple-store-front"] {
            if let Some(v) = headers.get(key) {
                safe.insert(reqwest::header::HeaderName::from_static(key), v.clone());
            }
        }
    }
    safe.insert(reqwest::header::USER_AGENT, USER_AGENT.parse().unwrap());
    safe.insert(
        reqwest::header::CONTENT_TYPE,
        if blob.is_some() {
            "application/x-www-form-urlencoded; charset=utf-8"
        } else {
            "application/x-apple-plist"
        }
        .parse()
        .unwrap(),
    );
    let body = payload(guid, appid, version, blob)?;
    let response = client
        .post(url)
        .headers(safe)
        .body(body)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| "Apple download recovery request failed".to_string())?;
    let data = bounded_response(response).await?;
    let map = parse_download(&data)?;
    validate(map, appid, version)
}

fn parse_download(data: &[u8]) -> Result<HashMap<String, Value>, String> {
    fn json(value: plist::Value) -> Value {
        use base64::Engine;
        match value {
            plist::Value::String(v) => Value::String(v),
            plist::Value::Boolean(v) => Value::Bool(v),
            plist::Value::Integer(v) => Value::String(v.to_string()),
            plist::Value::Real(v) => Value::String(v.to_string()),
            plist::Value::Data(v) => {
                Value::String(base64::engine::general_purpose::STANDARD.encode(v))
            }
            plist::Value::Date(v) => Value::String(format!("{v:?}")),
            plist::Value::Array(v) => Value::Array(v.into_iter().map(json).collect()),
            plist::Value::Dictionary(v) => {
                Value::Object(v.into_iter().map(|(k, v)| (k, json(v))).collect())
            }
            _ => Value::Null,
        }
    }
    let value: plist::Value =
        plist::from_bytes(data).map_err(|_| "invalid Apple download recovery plist")?;
    match json(value) {
        Value::Object(map) => Ok(map.into_iter().collect()),
        _ => Err("Apple download recovery plist is not a dictionary".into()),
    }
}

fn helper_path() -> Result<std::path::PathBuf, String> {
    let path = match std::env::var_os("IPA_KBSYNC_HELPER") {
        Some(path) => std::path::PathBuf::from(path),
        None => std::env::current_exe()
            .map_err(|_| "cannot locate server executable")?
            .parent()
            .ok_or("cannot locate server directory")?
            .join("ipatool-kbsync"),
    };
    if !path.is_absolute() {
        return Err("IPA_KBSYNC_HELPER must be an absolute executable path".into());
    }
    Ok(path)
}

async fn run_helper(
    path: &std::path::Path,
    guid: &str,
    dsid: &str,
    deadline: std::time::Duration,
) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    if !path.is_absolute() {
        return Err("kbsync helper path must be absolute".into());
    }
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|_| "kbsync helper unavailable")?;
    if !meta.is_file() {
        return Err("kbsync helper is not an executable file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            return Err("kbsync helper is not executable".into());
        }
    }
    let mut child = tokio::process::Command::new(path)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "cannot start kbsync helper")?;
    let operation = async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("kbsync helper stdin unavailable")?;
        let mut input = serde_json::to_vec(&serde_json::json!({"guid":guid,"dsid":dsid}))
            .map_err(|_| "encode kbsync helper input")?;
        input.push(b'\n');
        stdin
            .write_all(&input)
            .await
            .map_err(|_| "write kbsync helper input")?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or("kbsync helper stdout unavailable")?;
        let mut data = Vec::new();
        stdout
            .take((MAX_HELPER_OUTPUT + 1) as u64)
            .read_to_end(&mut data)
            .await
            .map_err(|_| "read kbsync helper output")?;
        if data.len() > MAX_HELPER_OUTPUT {
            return Err("kbsync helper output exceeds limit".into());
        }
        let status = child.wait().await.map_err(|_| "wait for kbsync helper")?;
        if !status.success() {
            return Err("kbsync helper exited unsuccessfully".into());
        }
        parse_helper(&data)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| "kbsync helper timed out".to_string())?
}

type BlobSlot =
    std::sync::Arc<tokio::sync::Mutex<Option<(std::time::Instant, Result<String, String>)>>>;

async fn shared_blob(
    slot: &BlobSlot,
    path: &std::path::Path,
    guid: &str,
    dsid: &str,
    deadline: std::time::Duration,
) -> Result<String, String> {
    let mut guard = slot.lock().await;
    if let Some((at, value)) = guard.as_ref() {
        if value.is_ok() || at.elapsed() < std::time::Duration::from_secs(5) {
            return value.clone();
        }
    }
    let result = run_helper(path, guid, dsid, deadline).await;
    *guard = Some((std::time::Instant::now(), result.clone()));
    result
}

async fn lookup_version(
    public: &reqwest::Client,
    base: &str,
    appid: &str,
    region: &str,
) -> Result<String, String> {
    for catalog in ["enterprisestore", "iphone", "ipad"] {
        let response = public
            .get(base)
            .query(&[
                ("version", "2"),
                ("id", appid),
                ("p", "mdm-lockup"),
                ("caller", "MDM"),
                ("platform", catalog),
                ("cc", &region.to_ascii_lowercase()),
                ("l", "en"),
            ])
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|_| "public MDM version lookup failed")?;
        let bytes = bounded_response(response).await?;
        let data: Value =
            serde_json::from_slice(&bytes).map_err(|_| "invalid public MDM response")?;
        if let Some(version) = offer_version(&data, appid)? {
            return Ok(version);
        }
    }
    Err("public MDM catalogs returned no iOS version".into())
}

struct RecoveryRequest<'a> {
    client: &'a reqwest::Client,
    headers: HeaderMap,
    guid: &'a str,
    appid: &'a str,
    version: Option<&'a str>,
    region: &'a str,
}
struct Targets<'a> {
    ent: &'a str,
    redownload: &'a str,
    mdm: &'a str,
}
type BlobCache = std::sync::Mutex<HashMap<(String, String), BlobSlot>>;
static BLOBS: std::sync::OnceLock<BlobCache> = std::sync::OnceLock::new();

fn blob_slot(dsid: &str, guid: &str) -> Result<BlobSlot, String> {
    let mut cache = BLOBS
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "kbsync cache unavailable")?;
    let key = (dsid.to_string(), guid.to_string());
    if let Some(slot) = cache.get(&key) {
        return Ok(slot.clone());
    }
    if cache.len() >= 128 {
        // Never evict a slot held by an active request, which would break singleflight.
        let old = cache
            .iter()
            .find(|(_, v)| std::sync::Arc::strong_count(v) == 1)
            .map(|(k, _)| k.clone());
        if let Some(key) = old {
            cache.remove(&key);
        } else {
            return Err("kbsync cache is busy".into());
        }
    }
    let slot = std::sync::Arc::new(tokio::sync::Mutex::new(None));
    cache.insert(key, slot.clone());
    Ok(slot)
}

async fn recover_using(
    request: RecoveryRequest<'_>,
    targets: Targets<'_>,
    helper: &std::path::Path,
    helper_timeout: std::time::Duration,
) -> Result<HashMap<String, Value>, String> {
    let RecoveryRequest {
        client,
        headers,
        guid,
        appid,
        version,
        region,
    } = request;
    numeric_id(appid, "app ID")?;
    if hex::decode(guid).ok().filter(|v| v.len() == 6).is_none() {
        return Err("download recovery requires a six-byte device GUID".into());
    }
    let dsid = headers
        .get("x-dsid")
        .and_then(|v| v.to_str().ok())
        .ok_or("download recovery requires X-Dsid")?
        .to_string();
    numeric_id(&dsid, "account DSID")?;
    if let Some(version) = version {
        numeric_id(version, "external version ID")?;
    }
    if region.len() != 2 || !region.bytes().all(|c| c.is_ascii_alphabetic()) {
        return Err("invalid MDM storefront region".into());
    }
    let slot = blob_slot(&dsid, guid)?;
    // Public catalog has no cookie store, account default headers, or redirect replay.
    let public = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| "build public MDM client")?;
    let resolve = async {
        if let Some(version) = version {
            Ok(version.to_string())
        } else {
            lookup_version(&public, targets.mdm, appid, region).await
        }
    };
    let generate = async {
        if headers.get("x-token").is_none() || headers.get("x-apple-store-front").is_none() {
            return Err("ent/download requires token and original storefront".to_string());
        }
        shared_blob(&slot, helper, guid, &dsid, helper_timeout).await
    };
    // MDM (at most 3 x 10s) overlaps slow guest generation rather than extending it.
    let (version, blob) = tokio::join!(resolve, generate);
    let version = version?;
    match blob {
        Ok(blob) => {
            let url = format!("{}?guid={guid}", targets.ent);
            let result = dispatch(client, &url, headers, guid, appid, &version, Some(&blob)).await;
            if result.is_err()
                || result
                    .as_ref()
                    .is_ok_and(|map| map.get("_state").and_then(Value::as_str) == Some("failure"))
            {
                let mut guard = slot.lock().await;
                if guard
                    .as_ref()
                    .is_some_and(|(_, v)| v.as_ref().is_ok_and(|b| b == &blob))
                {
                    *guard = None;
                }
            }
            // Never hide real ent HTTP/auth/license errors behind the known-broken legacy route.
            result
        }
        Err(helper_error) => {
            let url = format!("{}?guid={guid}", targets.redownload);
            dispatch(client, &url, headers, guid, appid, &version, None)
                .await
                .map_err(|e| format!("{helper_error}; legacy recovery failed: {e}"))
        }
    }
}

/// Recover only after `needs_download_recovery` accepts the primary response.
/// `client` must be the Store's shared-cookie client with redirect Policy::none().
/// Helper input is private stdin JSON; blob and subprocess output are never logged.
/// IPA_KBSYNC_TIMEOUT_SECS defaults to 180 (1..=600); HTTP adds at most 30 seconds.
/// No environment variable can replace the production Apple endpoints.
pub async fn recover(
    client: &reqwest::Client,
    headers: HeaderMap,
    guid: &str,
    appid: &str,
    version: Option<&str>,
    region: &str,
) -> Result<HashMap<String, Value>, String> {
    let helper = helper_path()?;
    let seconds = match std::env::var("IPA_KBSYNC_TIMEOUT_SECS") {
        Ok(value) => value
            .parse::<u64>()
            .ok()
            .filter(|n| (1..=600).contains(n))
            .ok_or("IPA_KBSYNC_TIMEOUT_SECS must be between 1 and 600")?,
        Err(std::env::VarError::NotPresent) => 180,
        Err(_) => return Err("invalid IPA_KBSYNC_TIMEOUT_SECS".into()),
    };
    let deadline = std::time::Duration::from_secs(seconds);
    let ent = format!("https://{DISPATCH_HOST}{ENT_PATH}");
    let redownload = format!("https://{DISPATCH_HOST}/r/redownload");
    tokio::time::timeout(
        deadline.max(std::time::Duration::from_secs(30)) + std::time::Duration::from_secs(31),
        recover_using(
            RecoveryRequest {
                client,
                headers,
                guid,
                appid,
                version,
                region,
            },
            Targets {
                ent: &ent,
                redownload: &redownload,
                mdm:
                    "https://uclient-api.itunes.apple.com/WebObjects/MZStorePlatform.woa/wa/lookup",
            },
            &helper,
            deadline,
        ),
    )
    .await
    .map_err(|_| "Apple download recovery timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> HashMap<String, Value> {
        serde_json::from_value(value).unwrap()
    }

    #[cfg(unix)]
    fn script(body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::var_os("TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("kbsync-test-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, format!("#!/usr/bin/env python3\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_receives_private_json_on_stdin_not_arguments() {
        let path = script("import json,sys\nv=json.load(sys.stdin)\nassert sys.argv==[sys.argv[0]]\nassert v=={'guid':'001122334455','dsid':'123'}\nprint(json.dumps({'kbsync':'YmxvYg=='}))");
        assert_eq!(
            run_helper(
                &path,
                "001122334455",
                "123",
                std::time::Duration::from_secs(3)
            )
            .await
            .unwrap(),
            "YmxvYg=="
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn helper_deadline_and_stdout_limit_are_enforced_without_secret_errors() {
        let path = script("import time\ntime.sleep(10)");
        let start = std::time::Instant::now();
        assert!(run_helper(
            &path,
            "001122334455",
            "123",
            std::time::Duration::from_millis(50)
        )
        .await
        .unwrap_err()
        .contains("timed out"));
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        std::fs::remove_file(path).unwrap();
        let path = script("import sys\nsys.stdout.write('SENSITIVE' * 200000)");
        let error = run_helper(
            &path,
            "001122334455",
            "123",
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap_err();
        assert!(error.contains("limit"));
        assert!(!error.contains("SENSITIVE"));
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shared_generation_is_singleflight_and_cached() {
        let path = script("import time,json,sys\nwith open(sys.argv[0]+'.count','a') as f: f.write('run\\n')\ntime.sleep(0.15)\nprint(json.dumps({'kbsync':'YmxvYg=='}))");
        let slot = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let (a, b) = tokio::join!(
            shared_blob(
                &slot,
                &path,
                "001122334455",
                "123",
                std::time::Duration::from_secs(3)
            ),
            shared_blob(
                &slot,
                &path,
                "001122334455",
                "123",
                std::time::Duration::from_secs(3)
            )
        );
        assert_eq!(a.unwrap(), "YmxvYg==");
        assert_eq!(b.unwrap(), "YmxvYg==");
        let count = path.with_file_name(format!(
            "{}.count",
            path.file_name().unwrap().to_str().unwrap()
        ));
        assert_eq!(std::fs::read_to_string(&count).unwrap(), "run\n");
        std::fs::remove_file(count).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            shared_blob(
                &slot,
                &path,
                "001122334455",
                "123",
                std::time::Duration::from_secs(3)
            )
            .await
            .unwrap(),
            "YmxvYg=="
        );
    }

    #[test]
    fn helper_default_is_next_to_server_binary() {
        if std::env::var_os("IPA_KBSYNC_HELPER").is_none() {
            assert_eq!(
                helper_path().unwrap(),
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("ipatool-kbsync")
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recovery_selects_ent_first_and_keeps_explicit_fixed_version() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let helper = script("import json\nprint(json.dumps({'kbsync':'YmxvYg=='}))");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            loop {
                let mut chunk = [0; 8192];
                let n = socket.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                buf.extend_from_slice(&chunk[..n]);
                if let Some(end) = buf.windows(4).position(|s| s == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]);
                    let size: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|s| s.parse().unwrap())
                        })
                        .unwrap();
                    if buf.len() >= end + 4 + size {
                        break;
                    }
                }
            }
            let end = buf.windows(4).position(|s| s == b"\r\n\r\n").unwrap();
            let head = String::from_utf8_lossy(&buf[..end]);
            assert!(head.starts_with("POST /ent?guid="));
            let d: plist::Dictionary = plist::from_bytes(&buf[end + 4..]).unwrap();
            assert_eq!(d["externalVersionId"].as_string(), Some("456"));
            assert_eq!(d["kbsync"].as_string(), Some("YmxvYg=="));
            let value = map(
                json!({"songList":[{"URL":"https://example.test/app","metadata":{"itemId":123,"softwareVersionExternalIdentifier":456,"softwareVersionBundleId":"com.example.app"}}]}),
            );
            let mut body = Vec::new();
            plist::to_writer_xml(&mut body, &value).unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-dsid", "98765001".parse().unwrap());
        headers.insert("x-token", "secret".parse().unwrap());
        headers.insert("x-apple-store-front", "143441-1,29".parse().unwrap());
        let ent = format!("http://{addr}/ent");
        let result = recover_using(
            RecoveryRequest {
                client: &client,
                headers,
                guid: "001122334455",
                appid: "123",
                version: Some("456"),
                region: "US",
            },
            Targets {
                ent: &ent,
                redownload: "http://127.0.0.1:1/legacy",
                mdm: "http://127.0.0.1:1/mdm",
            },
            &helper,
            std::time::Duration::from_secs(3),
        )
        .await
        .unwrap();
        assert_eq!(result["_state"], "success");
        server.await.unwrap();
        std::fs::remove_file(helper).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unavailable_helper_uses_dsid_cookies_only_but_ent_errors_never_trigger_legacy() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for preferred in [false, true] {
            let helper = script("import json\nprint(json.dumps({'kbsync':'YmxvYg=='}))");
            if !preferred {
                std::fs::remove_file(&helper).unwrap();
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0; 8192];
                let n = socket.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]);
                assert!(request.starts_with(if preferred {
                    "POST /ent?guid="
                } else {
                    "POST /legacy?guid="
                }));
                assert!(request.contains("x-dsid:"));
                assert!(request.contains("cookie: session=owned"));
                assert_eq!(request.contains("x-token: secret"), preferred);
                assert_eq!(request.contains("x-apple-store-front:"), preferred);
                assert!(!request.contains("evil-cookie"));
                socket
                    .write_all(
                        b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await
                    .unwrap();
            });
            let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
            jar.add_cookie_str(
                "session=owned; Path=/",
                &reqwest::Url::parse(&format!("http://{addr}")).unwrap(),
            );
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .cookie_provider(jar)
                .build()
                .unwrap();
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-dsid",
                if preferred { "98765003" } else { "98765002" }
                    .parse()
                    .unwrap(),
            );
            headers.insert("x-token", "secret".parse().unwrap());
            headers.insert("x-apple-store-front", "143441-1,29".parse().unwrap());
            headers.insert("cookie", "evil-cookie=secret".parse().unwrap());
            let ent = format!("http://{addr}/ent");
            let legacy = format!("http://{addr}/legacy");
            let error = recover_using(
                RecoveryRequest {
                    client: &client,
                    headers,
                    guid: "001122334455",
                    appid: "123",
                    version: Some("456"),
                    region: "US",
                },
                Targets {
                    ent: &ent,
                    redownload: &legacy,
                    mdm: "http://127.0.0.1:1/mdm",
                },
                &helper,
                std::time::Duration::from_secs(3),
            )
            .await
            .unwrap_err();
            assert!(error.contains("403"));
            assert_eq!(error.contains("legacy recovery failed"), !preferred);
            server.await.unwrap();
            if preferred {
                std::fs::remove_file(helper).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn mdm_lookup_uses_public_catalog_queries_without_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for catalog in ["enterprisestore", "iphone", "ipad"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0; 8192];
                let n = socket.read(&mut buf).await.unwrap();
                let request = String::from_utf8_lossy(&buf[..n]);
                assert!(request.contains(&format!("platform={catalog}")));
                assert!(request.contains("cc=us"));
                assert!(request.contains("caller=MDM"));
                assert!(request.contains("p=mdm-lockup"));
                assert!(request.contains("version=2"));
                assert!(request.contains("id=123"));
                assert!(!request.contains("cookie:"));
                assert!(!request.contains("x-token:"));
                assert!(!request.contains("x-dsid:"));
                let body = if catalog == "ipad" {
                    r#"{"results":{"123":{"offers":[{"version":{"externalId":456}}]}}}"#
                } else {
                    r#"{"results":{}}"#
                };
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let public = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        assert_eq!(
            lookup_version(&public, &format!("http://{addr}/lookup"), "123", "US")
                .await
                .unwrap(),
            "456"
        );
        server.await.unwrap();
    }

    #[test]
    fn ent_payload_matches_upstream_xml_types_and_serial() {
        use base64::Engine;
        let body = payload("001122334455", "123", "456", Some("YmxvYg==")).unwrap();
        let p: plist::Value = plist::from_bytes(&body).unwrap();
        let d = p.as_dictionary().unwrap();
        assert_eq!(d["salableAdamId"].as_string(), Some("123"));
        assert_eq!(d["externalVersionId"].as_string(), Some("456"));
        assert_eq!(d["kbsync"].as_string(), Some("YmxvYg=="));
        // Actual upstream hardwareID[2:] means FOUR trailing bytes, not three.
        assert_eq!(
            d["serialNumber"].as_string().unwrap(),
            base64::engine::general_purpose::STANDARD
                .encode([0x54, 0xc8, 0xb0, 0xa9, 0x88, 0x22, 0x33, 0x44, 0x55])
        );
        let p: plist::Value =
            plist::from_bytes(&payload("001122334455", "123", "456", None).unwrap()).unwrap();
        let d = p.as_dictionary().unwrap();
        assert_eq!(d["appExtVrsId"].as_string(), Some("456"));
        assert_eq!(d["serialNumber"].as_string(), Some("0"));
        assert!(!d.contains_key("kbsync"));
    }

    #[test]
    fn response_binary_sinf_is_base64_compatible_with_existing_download_consumer() {
        let response=br#"<?xml version="1.0"?><plist version="1.0"><dict><key>sinfs</key><array><dict><key>sinf</key><data>AAEC/w==</data></dict></array></dict></plist>"#;
        let map = parse_download(response).unwrap();
        assert_eq!(map["sinfs"][0]["sinf"], "AAEC/w==");
    }

    #[test]
    fn validation_requires_one_exact_app_version_and_package_url() {
        let good = json!({"songList":[{"URL":"https://example.test/app", "metadata":{"itemId":123,"softwareVersionExternalIdentifier":456,"softwareVersionBundleId":"com.example.app"}}]});
        assert!(validate(map(good.clone()), "123", "456").is_ok());
        for code in ["2034", "2042", "5002"] {
            let failure =
                map(json!({"failureType":code,"customerMessage":"Original Apple diagnostic"}));
            let result = validate(failure.clone(), "123", "456").unwrap();
            assert_eq!(result["failureType"], failure["failureType"]);
            assert_eq!(result["customerMessage"], failure["customerMessage"]);
            assert_eq!(result["_state"], "failure");
        }
        for bad in [
            json!({}),
            json!({"songList":[]}),
            json!({"customerMessage":"No Longer Available"}),
        ] {
            assert!(validate(map(bad), "123", "456").is_err());
        }
        assert!(validate(map(good.clone()), "124", "456").is_err());
        assert!(validate(map(good.clone()), "123", "457").is_err());
        let mut bad = good.clone();
        bad["songList"][0]["URL"] = json!("");
        assert!(validate(map(bad), "123", "456").is_err());
        let mut bad = good.clone();
        bad["songList"]
            .as_array_mut()
            .unwrap()
            .push(good["songList"][0].clone());
        assert!(validate(map(bad), "123", "456").is_err());
    }

    #[test]
    fn mdm_supports_string_number_and_buy_params_external_id() {
        for offer in [
            json!({"version":{"externalId":456}}),
            json!({"version":{"externalId":"456"}}),
            json!({"buyParams":"foo=bar&appExtVrsId=456"}),
        ] {
            assert_eq!(
                offer_version(&json!({"results":{"123":{"offers":[offer]}}}), "123").unwrap(),
                Some("456".into())
            );
        }
        assert_eq!(offer_version(&json!({"results":{}}), "123").unwrap(), None);
        assert!(offer_version(&json!({"results":{"123":{"offers":[{}]}}}), "123").is_err());
    }

    #[tokio::test]
    async fn dispatch_preserves_protocol_and_rejects_redirect_without_leaking_query() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let n = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8_lossy(&bytes[..n]).to_string();
            assert!(request.contains("x-token: secret"));
            assert!(request.contains("x-apple-store-front: 143441-1,29"));
            assert!(request.contains("application/x-www-form-urlencoded; charset=utf-8"));
            assert!(request.contains("Configurator/2.18"));
            socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/stolen\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-token", "secret".parse().unwrap());
        headers.insert("x-apple-store-front", "143441-1,29".parse().unwrap());
        let error = dispatch(
            &client,
            &format!("http://{addr}/ent/download?guid=SECRET"),
            headers,
            "001122334455",
            "123",
            "456",
            Some("YmxvYg=="),
        )
        .await
        .unwrap_err();
        assert!(error.contains("302"));
        assert!(error.contains("127.0.0.1/ent/download"));
        assert!(!error.contains("SECRET"));
        assert!(!error.contains("stolen"));
        server.await.unwrap();
    }

    #[test]
    fn helper_output_requires_bounded_nonempty_base64_json() {
        assert_eq!(
            parse_helper(br#"{"kbsync":"YmxvYg=="}"#).unwrap(),
            "YmxvYg=="
        );
        for bad in [
            br#"{"kbsync":""}"#.as_slice(),
            br#"{"kbsync":"not base64"}"#,
            br#"{}"#,
            b"garbage",
        ] {
            assert!(parse_helper(bad).is_err());
        }
    }

    #[test]
    fn empty_and_observed_availability_are_recoverable() {
        for value in [
            json!({}),
            json!({"songList": []}),
            json!({"failureType":"5002"}),
            json!({"customerMessage":"This item is no longer available"}),
        ] {
            assert!(needs_download_recovery(&map(value)));
        }
    }

    #[test]
    fn auth_license_and_other_messages_are_not_recoverable() {
        for value in [
            json!({"failureType":"2034"}),
            json!({"failureType":"5002","customerMessage":"Please sign in"}),
            json!({"customerMessage":"Your session has expired"}),
            json!({"customerMessage":"No longer available","failureType":"2034"}),
            json!({"songList":[{"URL":"https://example.test/app"}]}),
            json!({"songList":{}}),
        ] {
            assert!(!needs_download_recovery(&map(value)));
        }
    }
}
