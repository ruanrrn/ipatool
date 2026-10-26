use crate::apple_auth::{AccountStore, AuthInfo, Store};
use crate::SignatureClient;
use reqwest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::fs::{self};
use tokio::io::AsyncWriteExt;

const CHUNK_SIZE: usize = 5 * 1024 * 1024;
const MAX_RETRIES: usize = 5;
const RETRY_DELAY: u64 = 3000;
const MAX_CONCURRENT_CHUNKS: usize = 8;
/// Upper bound for one ranged request (5 MB chunk), i.e. a ~17 KB/s floor before retrying.
const CHUNK_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

fn parse_content_range(value: &str) -> Option<(u64, u64, Option<u64>)> {
    let value = value.trim().strip_prefix("bytes ")?;
    let (range, total) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let total = if total == "*" {
        None
    } else {
        Some(total.parse().ok()?)
    };

    Some((start.parse().ok()?, end.parse().ok()?, total))
}

async fn create_download_cache_dir(
    download_dir: &Path,
) -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let cache_root = download_dir.join("cache");
    fs::create_dir_all(&cache_root).await?;

    for attempt in 0..100u32 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let cache_dir = cache_root.join(format!(
            "download-{}-{}-{}",
            std::process::id(),
            now,
            attempt
        ));

        match fs::create_dir(&cache_dir).await {
            Ok(()) => return Ok(cache_dir),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }

    Err("无法创建唯一下载缓存目录".into())
}

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub phase: String,
    pub message: String,
    pub progress: Option<f64>,
    pub file_size: Option<u64>,
    pub downloaded: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub ok: bool,
    pub file: Option<String>,
    pub metadata: Option<DownloadMetadata>,
    pub error: Option<String>,
    pub needs_reauth: bool,
    pub needs_purchase: bool,
}

#[derive(Clone)]
pub struct DownloadParams<'a, S: AppleAuthService> {
    pub store: &'a S,
    pub email: &'a str,
    pub appid: &'a str,
    pub app_ver_id: Option<&'a str>,
    pub download_path: &'a str,
    pub auto_purchase: bool,
    pub token: Option<&'a str>,
    pub progress_callback: Option<std::sync::Arc<dyn Fn(DownloadProgress) + Send + Sync>>,
}

impl<'a, S: AppleAuthService> DownloadParams<'a, S> {
    pub fn on_progress(&self, progress: DownloadProgress) {
        if let Some(callback) = &self.progress_callback {
            callback(progress);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadata {
    pub bundle_display_name: String,
    pub bundle_short_version_string: String,
    pub bundle_id: String,
    pub artwork_url: String,
    pub artist_name: String,
}

fn get_value_from_map<'a>(
    map: &'a std::collections::HashMap<String, Value>,
    key: &str,
) -> Option<&'a Value> {
    map.get(key)
}

fn is_session_error(result: &std::collections::HashMap<String, Value>) -> bool {
    let error_msg = get_value_from_map(result, "failureType")
        .or(get_value_from_map(result, "customerMessage"))
        .or(get_value_from_map(result, "message"))
        .and_then(|v: &Value| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    let session_error_patterns = [
        "session expired",
        "session invalid",
        "invalid session",
        "unauthorized",
        "authentication failed",
        "token expired",
        "invalid token",
        "not authenticated",
    ];

    session_error_patterns
        .iter()
        .any(|pattern| error_msg.contains(pattern))
}

pub fn get_license_error_message(result: &std::collections::HashMap<String, Value>) -> String {
    let customer_message = get_value_from_map(result, "customerMessage")
        .and_then(|v: &Value| v.as_str())
        .unwrap_or("");

    let failure_type = get_value_from_map(result, "failureType")
        .and_then(|v: &Value| v.as_str())
        .unwrap_or("");

    let error_msg = format!("{} {}", customer_message, failure_type).to_lowercase();

    let license_error_map = [
        ("license not found", "您尚未购买此应用，正在尝试免费获取..."),
        ("not found", "未找到此应用，请检查 App ID 是否正确"),
        ("not purchased", "您尚未购买此应用"),
        ("未购买", "您尚未购买此应用"),
        ("未找到", "未找到此应用"),
        ("unauthorized", "无权下载此应用"),
        ("invalid request", "无效的请求"),
        ("item not found", "未找到此应用"),
        ("store front mismatch", "账号区域与应用不匹配"),
        ("store front error", "账号区域错误，请切换账号区域"),
    ];

    for (key, message) in &license_error_map {
        if error_msg.contains(key) {
            return message.to_string();
        }
    }

    if !customer_message.trim().is_empty() {
        return customer_message.to_string();
    }
    if !failure_type.trim().is_empty() {
        return failure_type.to_string();
    }

    "下载失败：Apple 未返回具体错误信息".to_string()
}

async fn download_chunk(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    output: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let expected_len = end
        .checked_sub(start)
        .and_then(|value| value.checked_add(1))
        .ok_or("无效的分块范围")?;

    let mut last_error: Box<dyn std::error::Error + Send + Sync> = "下载重试次数耗尽".into();
    for attempt in 0..MAX_RETRIES {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(RETRY_DELAY * attempt as u64)).await;
        }

        // Transport errors (reset, timeout, truncated body) are retried like a bad status: with
        // hundreds of chunks per IPA a single transient failure must not abort the whole job.
        let response = match client
            .get(url)
            .header("Range", format!("bytes={}-{}", start, end))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                last_error = format!("分块请求失败: {}", error).into();
                continue;
            }
        };

        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            last_error = format!("无法获取区块: 期望 206，实际 {}", response.status()).into();
            continue;
        }

        let content_range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .ok_or("分块响应缺少 Content-Range")?
            .to_str()?;

        let (range_start, range_end, total_len) = parse_content_range(content_range)
            .ok_or_else(|| format!("无效的 Content-Range: {}", content_range))?;
        if range_start != start || range_end != end {
            return Err(format!(
                "分块 Content-Range 不匹配: 期望 {}-{}，实际 {}-{}",
                start, end, range_start, range_end
            )
            .into());
        }
        if let Some(total_len) = total_len {
            if total_len <= end {
                return Err(format!(
                    "分块 Content-Range 总大小无效: total={} end={}",
                    total_len, end
                )
                .into());
            }
        }

        if let Some(content_length) = response.content_length() {
            if content_length != expected_len {
                return Err(format!(
                    "分块 Content-Length 不匹配: 期望 {}，实际 {}",
                    expected_len, content_length
                )
                .into());
            }
        }

        let bytes = match response.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                last_error = format!("分块读取失败: {}", error).into();
                continue;
            }
        };
        if bytes.len() as u64 != expected_len {
            last_error = format!(
                "分块大小不匹配: 期望 {}，实际 {}",
                expected_len,
                bytes.len()
            )
            .into();
            continue;
        }

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)
            .await?;

        file.write_all(&bytes).await?;
        file.flush().await?;

        return Ok(());
    }

    Err(last_error)
}

fn get_artwork_from_map(metadata: &serde_json::Map<String, Value>) -> String {
    let url_60 = metadata.get("artworkUrl60").and_then(|v| v.as_str());
    let url_512 = metadata.get("artworkUrl512").and_then(|v| v.as_str());
    let url_100 = metadata.get("artworkUrl100").and_then(|v| v.as_str());
    url_60.or(url_512).or(url_100).unwrap_or("").to_string()
}

pub fn sanitize_ipa_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '@') {
                c
            } else {
                '_'
            }
        })
        .collect();

    if cleaned.is_empty() {
        return "download.ipa".to_string();
    }

    const MAX_FILENAME_BYTES: usize = 255;
    if cleaned.len() <= MAX_FILENAME_BYTES {
        return cleaned;
    }

    // 超长文件名处理：保留 ".ipa" 后缀和 @bundle_id 部分，截断中间
    let extension = ".ipa";
    let bundle_id_suffix = cleaned.rfind('@').map(|pos| &cleaned[pos..]);
    let base_len = if let Some(suffix) = bundle_id_suffix {
        cleaned.len() - suffix.len()
    } else {
        cleaned.len() - extension.len()
    };

    if base_len == 0 {
        return format!("download_{}.ipa", hex::encode(&cleaned.as_bytes()[..8]));
    }

    let max_base_len = MAX_FILENAME_BYTES - extension.len();
    let truncated = if base_len > max_base_len {
        let suffix = bundle_id_suffix.unwrap_or(extension);
        let prefix_budget = max_base_len - suffix.len();
        if prefix_budget > 0 {
            format!("{}...{}", &cleaned[..prefix_budget.min(50)], suffix)
        } else {
            format!("download_{}.ipa", hex::encode(&cleaned.as_bytes()[..8]))
        }
    } else {
        cleaned
    };

    truncated
}

pub fn canonical_ipa_filename(
    display_name: &str,
    version: &str,
    bundle_id: Option<&str>,
) -> String {
    let safe_name = display_name.trim();
    let safe_version = version.trim();
    let safe_bundle_id = bundle_id.unwrap_or("").trim();

    let raw = if safe_bundle_id.is_empty() {
        format!("{}-{}.ipa", safe_name, safe_version)
    } else {
        format!("{}-{}@{}.ipa", safe_name, safe_version, safe_bundle_id)
    };

    sanitize_ipa_filename(&raw)
}

fn get_state(app: &std::collections::HashMap<String, Value>) -> Option<&Value> {
    app.get("_state")
}

fn get_song_list(app: &std::collections::HashMap<String, Value>) -> Option<&Value> {
    app.get("songList")
}

fn summarize_song_list_sinfs(song_list: &serde_json::Map<String, Value>) -> String {
    let sinfs = match song_list.get("sinfs").and_then(|value| value.as_array()) {
        Some(sinfs) => sinfs,
        None => return "sinfs=missing".to_string(),
    };

    if sinfs.is_empty() {
        return "sinfs=0".to_string();
    }

    let mut parts = Vec::with_capacity(sinfs.len());
    for (index, sinf) in sinfs.iter().enumerate() {
        let id = sinf
            .get("id")
            .and_then(|value| value.as_i64())
            .map(|value| value.to_string())
            .or_else(|| {
                sinf.get("id")
                    .and_then(|value| value.as_u64())
                    .map(|value| value.to_string())
            })
            .or_else(|| {
                sinf.get("id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string())
            })
            .unwrap_or_else(|| format!("idx{}", index));
        let b64_len = sinf
            .get("sinf")
            .and_then(|value| value.as_str())
            .map(|value| value.len())
            .unwrap_or(0);
        parts.push(format!("id={} len={}", id, b64_len));
    }

    format!("sinfs={} [{}]", sinfs.len(), parts.join(", "))
}

pub async fn download_ipa_with_account<S: AppleAuthService>(
    params: DownloadParams<'_, S>,
) -> Result<DownloadResult, Box<dyn std::error::Error + Send + Sync>> {
    params.on_progress(DownloadProgress {
        phase: "auth".to_string(),
        message: "[auth] 查询下载信息".to_string(),
        progress: None,
        file_size: None,
        downloaded: None,
    });

    let auth_info = AuthInfo {
        ds_person_id: None,
        password_token: None,
        display_name: None,
        email: Some(params.email.to_string()),
        region: None,
    };

    let mut app = params
        .store
        .download_product(params.appid, params.app_ver_id, &auth_info)
        .await?;

    let state = get_state(&app);
    if state != Some(&Value::String("success".to_string())) && is_session_error(&app) {
        params.on_progress(DownloadProgress {
            phase: "session".to_string(),
            message: "[session] 检测到会话失效，尝试刷新...".to_string(),
            progress: None,
            file_size: None,
            downloaded: None,
        });

        return Ok(DownloadResult {
            ok: false,
            file: None,
            metadata: None,
            error: Some("会话已失效，请重新登录".to_string()),
            needs_reauth: true,
            needs_purchase: false,
        });
    }

    let error_msg = get_value_from_map(&app, "failureType")
        .or(get_value_from_map(&app, "customerMessage"))
        .or(get_value_from_map(&app, "message"))
        .and_then(|v: &Value| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    let is_license_error = error_msg.contains("license")
        || error_msg.contains("not found")
        || error_msg.contains("未购买")
        || error_msg.contains("未授权");

    let current_state = get_state(&app);
    if current_state != Some(&Value::String("success".to_string())) && is_license_error {
        if params.auto_purchase {
            params.on_progress(DownloadProgress {
                phase: "auth".to_string(),
                message: "[purchase] 正在购买应用...".to_string(),
                progress: None,
                file_size: None,
                downloaded: None,
            });

            let license_result = params
                .store
                .ensure_license(params.appid, params.app_ver_id, &auth_info)
                .await?;

            if get_state(&license_result) != Some(&Value::String("success".to_string())) {
                let error_msg = get_license_error_message(&license_result);
                return Ok(DownloadResult {
                    ok: false,
                    file: None,
                    metadata: None,
                    error: Some(error_msg),
                    needs_reauth: false,
                    needs_purchase: true,
                });
            }

            params.on_progress(DownloadProgress {
                phase: "auth".to_string(),
                message: "[purchase] 购买成功，重新查询下载信息".to_string(),
                progress: None,
                file_size: None,
                downloaded: None,
            });

            app = params
                .store
                .download_product(params.appid, params.app_ver_id, &auth_info)
                .await?;

            if get_state(&app) != Some(&Value::String("success".to_string())) {
                let error_msg = get_license_error_message(&app);
                return Ok(DownloadResult {
                    ok: false,
                    file: None,
                    metadata: None,
                    error: Some(error_msg),
                    needs_reauth: false,
                    needs_purchase: true,
                });
            }
        } else {
            let error_msg = get_license_error_message(&app);
            return Ok(DownloadResult {
                ok: false,
                file: None,
                metadata: None,
                error: Some(error_msg),
                needs_reauth: false,
                needs_purchase: true,
            });
        }
    }

    if get_state(&app) != Some(&Value::String("success".to_string())) {
        let error_msg = get_value_from_map(&app, "customerMessage")
            .and_then(|v: &Value| v.as_str())
            .unwrap_or("下载失败")
            .to_string();

        return Ok(DownloadResult {
            ok: false,
            file: None,
            metadata: None,
            error: Some(error_msg),
            needs_reauth: false,
            needs_purchase: false,
        });
    }

    let song_list_value = get_song_list(&app)
        .and_then(|v| v.as_array())
        .and_then(|v| v.first())
        .ok_or("Invalid song list")?;

    let song_list = song_list_value
        .as_object()
        .ok_or("Invalid song list format")?;

    eprintln!(
        "[download] appid={} version_id={} {}",
        params.appid,
        params.app_ver_id.unwrap_or("latest"),
        summarize_song_list_sinfs(song_list)
    );

    let file_url = song_list
        .get("URL")
        .and_then(|v| v.as_str())
        .ok_or("Invalid file URL")?;

    let metadata = song_list
        .get("metadata")
        .and_then(|v| v.as_object())
        .ok_or("Invalid metadata")?;

    let download_dir = Path::new(params.download_path);
    fs::create_dir_all(download_dir).await?;

    let bundle_display_name = metadata
        .get("bundleDisplayName")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");

    let bundle_short_version = metadata
        .get("bundleShortVersionString")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0");

    let bundle_id = metadata
        .get("bundleId")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // OpenList 经验：iOS 16+ OTA 原生安装依赖文件名携带 @bundle_id。
    // App Store 下载得到的 IPA 已包含账号/授权信息，不在这里改写包体或补签。
    let output_file_name = canonical_ipa_filename(
        bundle_display_name,
        bundle_short_version,
        if bundle_id.is_empty() {
            None
        } else {
            Some(bundle_id)
        },
    );
    let output_file_path = download_dir.join(output_file_name);
    // Chunks are merged and patched in a sibling ".partial" file that is only renamed to the
    // final .ipa name once packaging succeeded, so a failed run never leaves a broken IPA that
    // the artifact scanner would later list as a completed download.
    let partial_file_path = output_file_path.with_extension("ipa.partial");
    let cache_dir = create_download_cache_dir(download_dir).await?;

    let outcome = fetch_and_package(
        &params,
        file_url,
        song_list_value,
        &cache_dir,
        &partial_file_path,
        &output_file_path,
    )
    .await;

    let _ = fs::remove_dir_all(&cache_dir).await;
    let downloaded = match outcome {
        Ok(downloaded) => downloaded,
        Err(error) => {
            let _ = fs::remove_file(&partial_file_path).await;
            return Err(error);
        }
    };

    let metadata_info = DownloadMetadata {
        bundle_display_name: bundle_display_name.to_string(),
        bundle_short_version_string: bundle_short_version.to_string(),
        bundle_id: bundle_id.to_string(),
        artwork_url: get_artwork_from_map(metadata),
        artist_name: metadata
            .get("artistName")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    };

    params.on_progress(DownloadProgress {
        phase: "done".to_string(),
        message: format!("[done] 产物：{}", output_file_path.to_string_lossy()),
        progress: Some(100.0),
        file_size: Some(downloaded),
        downloaded: Some(downloaded),
    });

    Ok(DownloadResult {
        ok: true,
        file: Some(output_file_path.to_string_lossy().into_owned()),
        metadata: Some(metadata_info),
        error: None,
        needs_reauth: false,
        needs_purchase: false,
    })
}

/// Download `file_url` in parallel ranged chunks, merge them into `partial_file_path`, inject
/// metadata/sinfs and finally move the result to `output_file_path`. Returns the byte count.
async fn fetch_and_package<S: AppleAuthService>(
    params: &DownloadParams<'_, S>,
    file_url: &str,
    song_list_value: &Value,
    cache_dir: &Path,
    partial_file_path: &Path,
    output_file_path: &Path,
) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    // One client per download: chunk requests reuse pooled keep-alive connections instead of
    // paying a TLS handshake per 5 MB chunk, and stalled connections are bounded by timeouts.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(CHUNK_REQUEST_TIMEOUT)
        .build()?;

    let response = client.get(file_url).send().await?;

    if !response.status().is_success() {
        return Err(format!("无法获取文件: {}", response.status()).into());
    }

    let file_size = response.content_length().unwrap_or(0);
    drop(response);
    if file_size == 0 {
        return Err("文件大小为 0，下载失败".into());
    }
    let num_chunks = file_size.div_ceil(CHUNK_SIZE as u64) as usize;

    params.on_progress(DownloadProgress {
        phase: "download-start".to_string(),
        message: format!(
            "[download] 开始：{:.2}MB，分块={}",
            file_size as f64 / 1024.0 / 1024.0,
            num_chunks
        ),
        progress: Some(0.0),
        file_size: Some(file_size),
        downloaded: Some(0),
    });

    let downloaded_total = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let progress_callback = params.progress_callback.clone();

    use futures::stream::{self, StreamExt};
    let mut chunk_stream = stream::iter(0..num_chunks)
        .map(|i| {
            let client = client.clone();
            let downloaded_total = downloaded_total.clone();
            let progress_callback = progress_callback.clone();

            async move {
                let start = (i * CHUNK_SIZE) as u64;
                let end = std::cmp::min(start + CHUNK_SIZE as u64 - 1, file_size - 1);
                let temp_output = cache_dir.join(format!("part{}", i));

                download_chunk(&client, file_url, start, end, &temp_output).await?;

                let chunk_bytes = end - start + 1;
                let prev =
                    downloaded_total.fetch_add(chunk_bytes, std::sync::atomic::Ordering::Relaxed);
                let current = prev + chunk_bytes;

                if let Some(callback) = &progress_callback {
                    let percent = ((current as f64 / file_size as f64) * 100.0).min(100.0);
                    callback(DownloadProgress {
                        phase: "download-progress".to_string(),
                        message: format!(
                            "[download] 进度 {:.2}MB / {:.2}MB",
                            current as f64 / 1024.0 / 1024.0,
                            file_size as f64 / 1024.0 / 1024.0
                        ),
                        progress: Some(percent),
                        file_size: Some(file_size),
                        downloaded: Some(current),
                    });
                }

                Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
            }
        })
        .buffer_unordered(MAX_CONCURRENT_CHUNKS);

    while let Some(result) = chunk_stream.next().await {
        result.map_err(|e| format!("分块下载失败: {}", e))?;
    }
    drop(chunk_stream);

    let downloaded = downloaded_total.load(std::sync::atomic::Ordering::Relaxed);

    params.on_progress(DownloadProgress {
        phase: "merge".to_string(),
        message: "[merge] 合并分块...".to_string(),
        progress: None,
        file_size: None,
        downloaded: None,
    });

    {
        let cache_dir = cache_dir.to_path_buf();
        let partial_file_path = partial_file_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            merge_chunks(&cache_dir, &partial_file_path, num_chunks, file_size)
        })
        .await
        .map_err(|e| format!("合并分块任务异常: {}", e))??;
    }

    params.on_progress(DownloadProgress {
        phase: "package".to_string(),
        message: "[package] 注入真实 iTunesMetadata / sinf（若 Apple 响应提供）".to_string(),
        progress: None,
        file_size: None,
        downloaded: None,
    });

    // ZIP rewriting is blocking file IO + CPU; keep it off the async worker so other requests
    // served by the same worker are not stalled for the duration of a multi-GB rewrite.
    let signature_result = {
        let song_list_value = song_list_value.clone();
        let email = params.email.to_string();
        let partial_file_path = partial_file_path.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || {
            let mut signature_client = SignatureClient::new(&song_list_value, &email)?;
            signature_client.load_file(&partial_file_path)?;
            signature_client.append_metadata();
            let signature_result = signature_client.append_signatures()?;
            signature_client.write()?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(signature_result)
        })
        .await
        .map_err(|e| format!("注入签名任务异常: {}", e))??
    };

    if !signature_result.applied_paths.is_empty() {
        params.on_progress(DownloadProgress {
            phase: "package".to_string(),
            message: format!(
                "[package] 已补齐 .sinf：{}",
                signature_result.applied_paths.join(", ")
            ),
            progress: None,
            file_size: None,
            downloaded: None,
        });
    } else if let Some(warning) = signature_result.warning {
        params.on_progress(DownloadProgress {
            phase: "package".to_string(),
            message: format!("[package] {}", warning),
            progress: None,
            file_size: None,
            downloaded: None,
        });
    }

    params.on_progress(DownloadProgress {
        phase: "finalize".to_string(),
        message: format!(
            "[finalize] 生成 OTA 文件名：{}",
            output_file_path
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("download.ipa")
        ),
        progress: None,
        file_size: None,
        downloaded: None,
    });

    fs::rename(partial_file_path, output_file_path).await?;
    Ok(downloaded)
}

/// Concatenate the downloaded part files into `output`, verifying every part and the final size.
/// `std::io::copy` between files uses `copy_file_range` on Linux, so no data passes through
/// user space.
fn merge_chunks(
    cache_dir: &Path,
    output: &Path,
    num_chunks: usize,
    file_size: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut final_file = std::fs::File::create(output)?;

    for i in 0..num_chunks {
        let temp_output = cache_dir.join(format!("part{}", i));
        let expected_chunk_size =
            std::cmp::min(CHUNK_SIZE as u64, file_size - (i * CHUNK_SIZE) as u64);
        let mut part = std::fs::File::open(&temp_output)?;
        let part_size = part.metadata()?.len();
        if part_size != expected_chunk_size {
            return Err(format!(
                "分块文件大小不匹配: part{} 期望 {}，实际 {}",
                i, expected_chunk_size, part_size
            )
            .into());
        }
        std::io::copy(&mut part, &mut final_file)?;
        drop(part);
        std::fs::remove_file(&temp_output)?;
    }
    final_file.sync_all()?;
    drop(final_file);

    let merged_size = std::fs::metadata(output)?.len();
    if merged_size != file_size {
        return Err(format!(
            "合并文件大小不匹配: 期望 {}，实际 {}",
            file_size, merged_size
        )
        .into());
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait AppleAuthService {
    async fn download_product(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>;

    async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>;
}

#[async_trait::async_trait]
impl AppleAuthService for Store {
    async fn download_product(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>
    {
        Store::download_product(self, app_identifier, app_ver_id, auth_info).await
    }

    async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>
    {
        Store::ensure_license(self, app_identifier, app_ver_id, auth_info).await
    }
}

#[async_trait::async_trait]
impl AppleAuthService for AccountStore {
    async fn download_product(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        _auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>
    {
        AccountStore::download_product(self, app_identifier, app_ver_id).await
    }

    async fn ensure_license(
        &self,
        app_identifier: &str,
        app_ver_id: Option<&str>,
        _auth_info: &AuthInfo,
    ) -> Result<std::collections::HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>>
    {
        AccountStore::ensure_license(self, app_identifier, app_ver_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use std::collections::HashMap;
    use std::io::{Cursor, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;

    struct SongListStore {
        song: Value,
    }

    #[async_trait::async_trait]
    impl AppleAuthService for SongListStore {
        async fn download_product(
            &self,
            _: &str,
            _: Option<&str>,
            _: &AuthInfo,
        ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(HashMap::from([
                ("_state".to_string(), Value::String("success".to_string())),
                (
                    "songList".to_string(),
                    Value::Array(vec![self.song.clone()]),
                ),
            ]))
        }

        async fn ensure_license(
            &self,
            _: &str,
            _: Option<&str>,
            _: &AuthInfo,
        ) -> Result<HashMap<String, Value>, Box<dyn std::error::Error + Send + Sync>> {
            unreachable!("download succeeds without purchase")
        }
    }

    fn sample_ipa(binary: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
        let stored: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let plist = |entries: Vec<(&str, plist::Value)>| {
            let mut dict = plist::Dictionary::new();
            for (key, value) in entries {
                dict.insert(key.to_string(), value);
            }
            let mut bytes = Vec::new();
            plist::to_writer_xml(&mut bytes, &plist::Value::Dictionary(dict)).unwrap();
            bytes
        };
        zip.add_directory("Payload/Test.app/", stored).unwrap();
        zip.start_file("Payload/Test.app/Info.plist", stored)
            .unwrap();
        zip.write_all(&plist(vec![(
            "CFBundleExecutable",
            plist::Value::String("Test".into()),
        )]))
        .unwrap();
        zip.start_file("Payload/Test.app/SC_Info/Manifest.plist", stored)
            .unwrap();
        zip.write_all(&plist(vec![(
            "SinfPaths",
            plist::Value::Array(vec![plist::Value::String("SC_Info/Test.sinf".into())]),
        )]))
        .unwrap();
        zip.start_file("Payload/Test.app/Test", stored).unwrap();
        zip.write_all(binary).unwrap();
        zip.finish().unwrap();
        out
    }

    /// Minimal HTTP/1.1 server with Range support. `respond` may override the response for a
    /// given (request index, range); every connection is closed after one response.
    async fn serve_file<F>(body: Arc<Vec<u8>>, respond: F) -> String
    where
        F: Fn(usize, Option<(u64, u64)>) -> Option<String> + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let respond = Arc::new(respond);
        let counter = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                let respond = respond.clone();
                let index = counter.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let read = socket.read(&mut buffer).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..read]);
                    }
                    let request = String::from_utf8_lossy(&request).to_lowercase();
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: bytes="))
                        .and_then(|value| value.trim().split_once('-'))
                        .and_then(|(start, end)| Some((start.parse().ok()?, end.parse().ok()?)));
                    let (head, payload): (String, &[u8]) = match respond(index, range) {
                        Some(head) => (head, &[]),
                        None => match range {
                            Some((start, end)) => (
                                format!(
                                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    start,
                                    end,
                                    body.len(),
                                    end - start + 1
                                ),
                                &body[start as usize..=end as usize],
                            ),
                            None => (
                                format!(
                                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    body.len()
                                ),
                                &body[..],
                            ),
                        },
                    };
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(payload).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        format!("http://{}/app.ipa", address)
    }

    fn song_for(url: &str) -> Value {
        serde_json::json!({
            "URL": url,
            "metadata": {
                "bundleDisplayName": "Test",
                "bundleShortVersionString": "1.2.3",
                "bundleId": "com.example.test",
                "artistName": "Tester"
            },
            "sinfs": [{
                "id": 0,
                "sinf": base64::engine::general_purpose::STANDARD.encode(b"main-sinf")
            }]
        })
    }

    fn temp_download_dir(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{}-{}-{}",
            prefix,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftover_files(dir: &Path) -> Vec<String> {
        let mut names = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current).unwrap().flatten() {
                if entry.path().is_dir() {
                    stack.push(entry.path());
                } else {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        names
    }

    #[tokio::test]
    async fn download_merges_chunks_retries_and_injects_signatures() {
        // > 2 chunks of incompressible data so ranged downloads and merging are exercised.
        let mut binary = vec![0u8; CHUNK_SIZE * 2 + 12345];
        let mut seed = 0x2545_f491_u32;
        for byte in binary.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            *byte = (seed >> 24) as u8;
        }
        let ipa = Arc::new(sample_ipa(&binary));
        // The first ranged request fails once; the chunk must be retried, not fail the job.
        let failed_once = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let url = serve_file(ipa.clone(), move |_, range| {
            (range.is_some() && !failed_once.swap(true, Ordering::SeqCst)).then(|| {
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            })
        })
        .await;

        let dir = temp_download_dir("ipa-download-ok");
        let download_path = dir.to_string_lossy().into_owned();
        let store = SongListStore {
            song: song_for(&url),
        };
        let max_percent = Arc::new(std::sync::Mutex::new(0f64));
        let progress_max = max_percent.clone();
        let params = DownloadParams {
            store: &store,
            email: "tester@example.com",
            appid: "123",
            app_ver_id: None,
            download_path: &download_path,
            auto_purchase: false,
            token: None,
            progress_callback: Some(Arc::new(move |progress: DownloadProgress| {
                if let Some(percent) = progress.progress {
                    let mut max = progress_max.lock().unwrap();
                    *max = max.max(percent);
                }
            })),
        };

        let result = download_ipa_with_account(params).await.unwrap();
        assert!(result.ok, "{:?}", result.error);
        let file = result.file.unwrap();
        assert!(file.ends_with("Test-1.2.3@com.example.test.ipa"), "{file}");
        assert_eq!(*max_percent.lock().unwrap(), 100.0);
        // Only the final IPA remains: no .partial, no part files, no rewrite temp file.
        assert_eq!(
            leftover_files(&dir),
            vec!["Test-1.2.3@com.example.test.ipa".to_string()]
        );

        let mut zip = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let mut sinf = Vec::new();
        zip.by_name("Payload/Test.app/SC_Info/Test.sinf")
            .unwrap()
            .read_to_end(&mut sinf)
            .unwrap();
        assert_eq!(sinf, b"main-sinf");
        let mut metadata = Vec::new();
        zip.by_name("iTunesMetadata.plist")
            .unwrap()
            .read_to_end(&mut metadata)
            .unwrap();
        assert!(String::from_utf8_lossy(&metadata).contains("tester@example.com"));
        let mut copied = Vec::new();
        zip.by_name("Payload/Test.app/Test")
            .unwrap()
            .read_to_end(&mut copied)
            .unwrap();
        assert!(copied == binary);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn failed_download_leaves_no_partial_or_ipa_behind() {
        let ipa = Arc::new(sample_ipa(&[1u8; 4096]));
        // A mismatching Content-Range is a hard error (no retry).
        let url = serve_file(ipa.clone(), |_, range| {
            range.map(|_| {
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-2/3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string()
            })
        })
        .await;

        let dir = temp_download_dir("ipa-download-fail");
        let download_path = dir.to_string_lossy().into_owned();
        let store = SongListStore {
            song: song_for(&url),
        };
        let params = DownloadParams {
            store: &store,
            email: "tester@example.com",
            appid: "123",
            app_ver_id: None,
            download_path: &download_path,
            auto_purchase: false,
            token: None,
            progress_callback: None,
        };

        assert!(download_ipa_with_account(params).await.is_err());
        assert!(
            leftover_files(&dir).is_empty(),
            "{:?}",
            leftover_files(&dir)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
