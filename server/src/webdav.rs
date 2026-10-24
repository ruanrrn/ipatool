//! WebDAV 文件上传。
//!
//! WebDAV 的核心操作就是标准 HTTP：
//!   - PUT   上传/覆盖文件
//!   - MKCOL 创建目录（已存在返回 405，视为成功）
//!
//! 因此不需要引入专门的 WebDAV crate，直接用 reqwest 即可。
//! 下载完成后由 download_manager 调用 `upload_file` 把 IPA 推到远端。

use std::path::Path;
use tokio::fs;
use tokio::io::AsyncReadExt;

/// WebDAV 上传配置（从数据库 WebDAVConfig 映射而来）。
#[derive(Debug, Clone)]
pub struct WebDAVTarget {
    pub url: String,
    pub username: String,
    pub password: String,
    pub remote_path: String,
}

impl WebDAVTarget {
    /// 构建带 Basic Auth 的 reqwest RequestBuilder。
    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .expect("reqwest client build failed");
        let mut req = client.request(method, url);
        if !self.username.is_empty() {
            req = req.basic_auth(&self.username, Some(&self.password));
        }
        req
    }

    /// 拼接最终上传 URL：base_url + remote_path + filename
    fn file_url(&self, filename: &str) -> String {
        let base = self.url.trim_end_matches('/');
        let remote = self.remote_path.trim_matches('/');
        if remote.is_empty() {
            format!("{}/{}", base, filename)
        } else {
            format!("{}/{}/{}", base, remote, filename)
        }
    }

    fn dir_url(&self) -> String {
        let base = self.url.trim_end_matches('/');
        let remote = self.remote_path.trim_matches('/');
        if remote.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, remote)
        }
    }
}

/// WebDAV 上传错误。
#[derive(Debug)]
pub struct WebDAVError(pub String);

impl std::fmt::Display for WebDAVError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for WebDAVError {}

/// 测试 WebDAV 连接：对 remote_path 目录发 PROPFIND（深度 0）。
/// 返回 Ok 表示目录可达且凭据有效。
pub async fn test_connection(target: &WebDAVTarget) -> Result<(), WebDAVError> {
    let url = target.dir_url();

    let resp = target
        .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(
            r#"<?xml version="1.0" encoding="utf-8"?><propfind xmlns="DAV:"><prop><displayname/></prop></propfind>"#,
        )
        .send()
        .await
        .map_err(|e| WebDAVError(format!("连接失败: {}", e)))?;

    let status = resp.status().as_u16();
    // 207 Multi-Status = 成功; 200 某些实现也返回
    if status == 207 || status == 200 {
        return Ok(());
    }
    if status == 401 || status == 403 {
        return Err(WebDAVError("认证失败：用户名或密码错误".to_string()));
    }
    if status == 404 {
        return Err(WebDAVError(format!(
            "目录不存在: {}（HTTP 404）",
            target.remote_path
        )));
    }
    let text = resp.text().await.unwrap_or_default();
    Err(WebDAVError(format!(
        "服务器返回异常状态: {} {}",
        status,
        text.chars().take(200).collect::<String>()
    )))
}

/// 上传本地文件到 WebDAV。
/// 自动创建目标目录（MKCOL），然后 PUT 文件。
/// 返回上传后的完整 URL。
pub async fn upload_file(target: &WebDAVTarget, local_path: &Path) -> Result<String, WebDAVError> {
    let filename = local_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| WebDAVError("无法解析文件名".to_string()))?;

    // 1. 确保目标目录存在（MKCOL，已存在不报错）
    let dir_url = target.dir_url();
    if !target.remote_path.trim_matches('/').is_empty() {
        let mkcol = target
            .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &dir_url)
            .send()
            .await;
        if let Ok(resp) = mkcol {
            let s = resp.status().as_u16();
            // 201 created, 405 method not allowed (已存在), 301 ok — 都视为目录就绪
            if s != 201 && s != 405 && s != 301 && s != 200 {
                log::warn!(
                    "[webdav] MKCOL {} returned status {}, continuing with PUT",
                    dir_url,
                    s
                );
            }
        }
    }

    // 2. 读取文件内容
    let mut file = fs::File::open(local_path)
        .await
        .map_err(|e| WebDAVError(format!("无法读取本地文件: {}", e)))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| WebDAVError(format!("无法获取文件信息: {}", e)))?;
    let file_size = metadata.len();
    let mut buffer = Vec::with_capacity(file_size as usize);
    file.read_to_end(&mut buffer)
        .await
        .map_err(|e| WebDAVError(format!("读取文件内容失败: {}", e)))?;
    drop(file);

    // 3. PUT 上传
    let file_url = target.file_url(filename);
    log::info!(
        "[webdav] uploading {} ({} bytes) -> {}",
        filename,
        file_size,
        file_url
    );

    let resp = target
        .request(reqwest::Method::PUT, &file_url)
        .header("Content-Type", "application/octet-stream")
        .header("Content-Length", file_size)
        .body(buffer)
        .send()
        .await
        .map_err(|e| WebDAVError(format!("上传请求失败: {}", e)))?;

    let status = resp.status().as_u16();
    // 200, 201, 204 = 成功
    if status == 200 || status == 201 || status == 204 {
        log::info!("[webdav] upload complete: {} -> {}", filename, file_url);
        return Ok(file_url);
    }

    if status == 401 || status == 403 {
        return Err(WebDAVError("认证失败：用户名或密码错误".to_string()));
    }
    let body = resp.text().await.unwrap_or_default();
    Err(WebDAVError(format!(
        "上传失败: HTTP {} {}",
        status,
        body.chars().take(300).collect::<String>()
    )))
}

/// 从数据库配置构建 WebDAVTarget。
/// 如果未配置或 url 为空返回 None。
pub fn target_from_config(config: &crate::WebDAVConfig) -> Option<WebDAVTarget> {
    if config.url.trim().is_empty() {
        return None;
    }
    Some(WebDAVTarget {
        url: config.url.clone(),
        username: config.username.clone(),
        password: config.password.clone(),
        remote_path: config.remote_path.clone(),
    })
}

/// 目录条目
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// 列出 WebDAV 目录内容。
/// 对目标 URL 发 PROPFIND Depth:1，解析返回的 XML 提取 href 和资源类型。
/// 过滤掉自身（href == dir_url 或 href == dir_url/），只返回子条目。
pub async fn list_directory(target: &WebDAVTarget) -> Result<Vec<DirEntry>, WebDAVError> {
    let dir_url = target.dir_url();

    let resp = target
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
            &dir_url,
        )
        .header("Depth", "1")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(
            r#"<?xml version="1.0" encoding="utf-8"?><propfind xmlns="DAV:"><prop><resourcetype/></prop></propfind>"#,
        )
        .send()
        .await
        .map_err(|e| WebDAVError(format!("PROPFIND 请求失败: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 207 && status != 200 {
        let body = resp.text().await.unwrap_or_default();
        return Err(WebDAVError(format!(
            "服务器返回异常状态: {} {}",
            status,
            body.chars().take(200).collect::<String>()
        )));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| WebDAVError(format!("读取响应失败: {}", e)))?;

    parse_propfind_response(&body, &dir_url)
}

/// 解析 PROPFIND 响应的 XML，提取目录条目。
fn parse_propfind_response(xml: &str, base_url: &str) -> Result<Vec<DirEntry>, WebDAVError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.trim_text(true);

    let base_path = extract_url_path(base_url);

    let mut entries: Vec<DirEntry> = Vec::new();
    let mut current_href: Option<String> = None;
    let mut is_collection = false;
    let mut in_propstat = false;
    let mut in_prop = false;
    let mut in_href = false;
    let mut in_status = false;
    let mut status_is_200 = false;
    let mut prop_has_collection = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = e.name();
                match name.local_name().as_ref() {
                    b"response" => {
                        current_href = None;
                        is_collection = false;
                        in_propstat = false;
                        in_prop = false;
                        status_is_200 = false;
                        prop_has_collection = false;
                    }
                    b"propstat" => {
                        in_propstat = true;
                        prop_has_collection = false;
                        status_is_200 = false;
                    }
                    b"prop" => {
                        if in_propstat {
                            in_prop = true;
                        }
                    }
                    b"collection" => {
                        if in_prop {
                            prop_has_collection = true;
                        }
                    }
                    b"href" => {
                        in_href = true;
                    }
                    b"status" if in_propstat => {
                        in_status = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    if in_href {
                        current_href = Some(text.to_string());
                    } else if in_status {
                        status_is_200 = text.contains("200 OK") || text.contains("200");
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                match name.local_name().as_ref() {
                    b"propstat" => {
                        if prop_has_collection && (status_is_200 || !in_status) {
                            is_collection = true;
                        }
                        in_propstat = false;
                        in_prop = false;
                        prop_has_collection = false;
                    }
                    b"response" => {
                        if let Some(ref href) = current_href {
                            let href_path = extract_url_path(href);

                            // 跳过自身目录
                            if href_path == base_path {
                                continue;
                            }

                            let name = href_path
                                .rsplit_once('/')
                                .map(|(_, n)| n)
                                .unwrap_or(&href_path)
                                .to_string();

                            if name.is_empty() {
                                continue;
                            }

                            let rel_path =
                                if let Some(stripped) = href_path.strip_prefix(&base_path) {
                                    let rest = stripped.trim_start_matches('/');
                                    if rest.is_empty() {
                                        name.clone()
                                    } else {
                                        rest.to_string()
                                    }
                                } else {
                                    name.clone()
                                };

                            let is_dir = is_collection || href.ends_with('/');

                            entries.push(DirEntry {
                                name,
                                path: rel_path,
                                is_dir,
                            });
                        }
                    }
                    b"href" => {
                        in_href = false;
                    }
                    b"status" => {
                        in_status = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(WebDAVError(format!("XML 解析错误: {}", e)));
            }
            _ => {}
        }
        buf.clear();
    }

    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

fn extract_url_path(raw: &str) -> String {
    let path = if let Ok(parsed) = reqwest::Url::parse(raw) {
        parsed.path().to_string()
    } else {
        raw.split('?').next().unwrap_or(raw).to_string()
    };
    let unescaped = urlencoding::decode(&path).unwrap_or(std::borrow::Cow::Borrowed(&path));
    unescaped.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_propfind_extracts_directories_and_files() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/photos/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/ipa%20files/</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype><D:collection/></D:resourcetype>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/app.ipa</D:href>
    <D:propstat>
      <D:prop>
        <D:resourcetype/>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>
</D:multistatus>"#;

        let entries = parse_propfind_response(xml, "https://example.com/dav/").unwrap();
        // Self (/dav/) should be skipped
        assert_eq!(entries.len(), 3);

        // Directories first, sorted
        assert!(entries[0].is_dir);
        assert_eq!(entries[0].name, "ipa files");
        assert_eq!(entries[0].path, "ipa files");

        assert!(entries[1].is_dir);
        assert_eq!(entries[1].name, "photos");
        assert_eq!(entries[1].path, "photos");

        assert!(!entries[2].is_dir);
        assert_eq!(entries[2].name, "app.ipa");
        assert_eq!(entries[2].path, "app.ipa");
    }
}
