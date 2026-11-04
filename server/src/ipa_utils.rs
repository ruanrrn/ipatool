use crate::signature::{read_bounded, MAX_ICON_BYTES, MAX_PLIST_BYTES};
use std::path::Path;

/// 完整的 IPA 元数据结构体
#[derive(Debug, Clone, Default)]
pub struct IpaMetadata {
    pub item_id: Option<String>,
    pub item_name: Option<String>,
    pub bundle_display_name: Option<String>,
    pub bundle_id: Option<String>,
    pub bundle_short_version: Option<String>,
    pub bundle_version: Option<String>,
    pub artist_name: Option<String>,
    pub artist_id: Option<String>,
    pub genre: Option<String>,
    pub genre_id: Option<String>,
    pub release_date: Option<String>,
    pub copyright: Option<String>,
    pub icon_url: Option<String>,
    pub has_iap: Option<bool>,
    pub min_os_version: Option<String>,
    pub primary_icon_bytes: Option<Vec<u8>>,
}

/// 从指定的 IPA 文件路径提取基本元信息（name, bundle_id, size）
/// 返回 (name, bundle_id, size_in_bytes)
pub fn extract_metadata_from_ipa(ipa_path: &Path) -> Option<(String, Option<String>, Option<i64>)> {
    let file = std::fs::File::open(ipa_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;

    // 找到 app bundle name
    let app_bundle_name = find_app_bundle_in_zip(&mut zip)?;

    // 读取 Info.plist
    let info_path = format!("Payload/{}/Info.plist", app_bundle_name);
    let info_plist = read_plist_from_zip(&mut zip, &info_path)?;

    let name = plist_string_value(&info_plist, "CFBundleDisplayName")
        .or_else(|| plist_string_value(&info_plist, "CFBundleName"))
        .unwrap_or_default();
    let bundle_id = plist_string_value(&info_plist, "CFBundleIdentifier");
    let size = std::fs::metadata(ipa_path).ok().map(|m| m.len() as i64);

    if name.is_empty() {
        return None;
    }

    Some((name, bundle_id, size))
}

/// 从 IPA 的 iTunesMetadata.plist 提取完整元数据
pub fn extract_itunes_metadata_from_ipa(ipa_path: &Path) -> Option<IpaMetadata> {
    let file = std::fs::File::open(ipa_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;

    let mut metadata = IpaMetadata::default();

    // 尝试读取 iTunesMetadata.plist（根目录级别）
    if let Some(itunes_plist) = read_plist_from_zip(&mut zip, "iTunesMetadata.plist") {
        metadata.item_id = plist_string_value(&itunes_plist, "itemId");
        metadata.item_name = plist_string_value(&itunes_plist, "itemName");
        metadata.bundle_display_name = plist_string_value(&itunes_plist, "bundleDisplayName");
        metadata.bundle_id = plist_string_value(&itunes_plist, "softwareVersionBundleId");
        metadata.bundle_short_version =
            plist_string_value(&itunes_plist, "bundleShortVersionString");
        metadata.bundle_version = plist_string_value(&itunes_plist, "bundleVersion");
        metadata.artist_name = plist_string_value(&itunes_plist, "artistName");
        metadata.artist_id = plist_string_value(&itunes_plist, "artistId");
        metadata.genre = plist_string_value(&itunes_plist, "genre");
        metadata.genre_id = plist_string_value(&itunes_plist, "genreId");
        metadata.release_date = plist_string_value(&itunes_plist, "releaseDate");
        metadata.copyright = plist_string_value(&itunes_plist, "copyright");
        metadata.icon_url = plist_string_value(&itunes_plist, "softwareIcon57x57URL");
        metadata.has_iap = plist_bool_value(&itunes_plist, "hasOrEverHasHadIAP");
    }

    // 从 Info.plist 补充 MinimumOSVersion
    if let Some(app_bundle_name) = find_app_bundle_in_zip(&mut zip) {
        let info_path = format!("Payload/{}/Info.plist", app_bundle_name);
        if let Some(info_plist) = read_plist_from_zip(&mut zip, &info_path) {
            metadata.min_os_version = plist_string_value(&info_plist, "MinimumOSVersion");

            // 如果 iTunesMetadata 没有提供 bundle_id，从 Info.plist 补充
            if metadata.bundle_id.is_none() {
                metadata.bundle_id = plist_string_value(&info_plist, "CFBundleIdentifier");
            }
            if metadata.bundle_display_name.is_none() {
                metadata.bundle_display_name =
                    plist_string_value(&info_plist, "CFBundleDisplayName")
                        .or_else(|| plist_string_value(&info_plist, "CFBundleName"));
            }
            if metadata.bundle_short_version.is_none() {
                metadata.bundle_short_version =
                    plist_string_value(&info_plist, "CFBundleShortVersionString");
            }
            if metadata.bundle_version.is_none() {
                metadata.bundle_version = plist_string_value(&info_plist, "CFBundleVersion");
            }
        }

        // 提取最大的 AppIcon PNG
        metadata.primary_icon_bytes = extract_largest_app_icon_from_zip(&mut zip, &app_bundle_name);
    }

    // 至少要有一些有用的信息才算成功
    if metadata.bundle_id.is_none()
        && metadata.item_name.is_none()
        && metadata.bundle_display_name.is_none()
    {
        return None;
    }

    Some(metadata)
}

/// 从 IPA 内提取最大的 AppIcon PNG bytes
/// 优先查找 60x60@3x (180x180)，然后 60x60@2x (120x120)，依此类推
pub fn extract_largest_app_icon_from_ipa(ipa_path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(ipa_path).ok()?;
    let mut zip = zip::ZipArchive::new(file).ok()?;
    let app_bundle_name = find_app_bundle_in_zip(&mut zip)?;
    extract_largest_app_icon_from_zip(&mut zip, &app_bundle_name)
}

/// 从 ZIP archive 中提取最大的 AppIcon PNG bytes
fn extract_largest_app_icon_from_zip<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    app_bundle_name: &str,
) -> Option<Vec<u8>> {
    let icon_prefix = format!("Payload/{}/AppIcon60x60/", app_bundle_name);

    // 优先级排序：@3x 最大，@2x 次之，@1x 最小
    let priorities: &[&str] = &[
        "@3x~ipad.png",
        "@3x~iphone.png",
        "@3x.png",
        "@2x~ipad.png",
        "@2x~iphone.png",
        "@2x.png",
        "@1x~ipad.png",
        "@1x~iphone.png",
        "@1x.png",
        ".png",
    ];

    for suffix in priorities {
        let icon_path = format!("{}{}", icon_prefix, suffix);
        if let Ok(entry) = zip.by_name(&icon_path) {
            let size = entry.size();
            if let Ok(data) = read_bounded(entry, size, MAX_ICON_BYTES) {
                // 验证是 PNG (以 0x89504E47 开头)
                if data.len() > 8 && data[0..4] == [0x89, 0x50, 0x4E, 0x47] {
                    return Some(data);
                }
            }
        }
    }

    // 如果 AppIcon60x60 找不到，尝试搜索所有 icon 文件
    let mut best_icon: Option<Vec<u8>> = None;
    let mut best_size: usize = 0;

    for i in 0..zip.len() {
        let entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        if name.contains("AppIcon") && name.ends_with(".png") && entry.is_file() {
            let size = entry.size();
            let data = match read_bounded(entry, size, MAX_ICON_BYTES) {
                Ok(data) => data,
                Err(_) => continue,
            };
            if data.len() > best_size && data.len() > 8 && data[0..4] == [0x89, 0x50, 0x4E, 0x47] {
                best_size = data.len();
                best_icon = Some(data);
            }
        }
    }

    best_icon
}

/// 在 ZIP archive 中找到 .app bundle 目录名
pub fn find_app_bundle_in_zip<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Option<String> {
    for i in 0..zip.len() {
        let zip_name = zip.by_index(i).ok()?.name().to_string();
        if zip_name.starts_with("Payload/") && zip_name.ends_with(".app/") {
            let bundle = zip_name
                .strip_prefix("Payload/")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap_or(&zip_name)
                .to_string();
            return Some(bundle);
        }
    }
    None
}

/// 从 ZIP archive 中读取 plist 并解析为 Value
pub fn read_plist_from_zip<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    path: &str,
) -> Option<plist::Value> {
    let file = zip.by_name(path).ok()?;
    let size = file.size();
    let content = read_bounded(file, size, MAX_PLIST_BYTES).ok()?;
    plist::from_bytes(&content).ok()
}

/// 从 plist Value 中提取字符串值
pub fn plist_string_value(plist: &plist::Value, key: &str) -> Option<String> {
    match plist {
        plist::Value::Dictionary(dict) => dict.get(key).and_then(|v| match v {
            plist::Value::String(s) => Some(s.to_string()),
            _ => None,
        }),
        _ => None,
    }
}

/// 从 plist Value 中提取布尔值
fn plist_bool_value(plist: &plist::Value, key: &str) -> Option<bool> {
    match plist {
        plist::Value::Dictionary(dict) => dict.get(key).and_then(|v| match v {
            plist::Value::Boolean(b) => Some(*b),
            plist::Value::Integer(i) => Some(i.as_unsigned() != Some(0)),
            _ => None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn oversized_icon_is_skipped_and_normal_icon_is_preserved() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        writer
            .start_file("Payload/Test.app/AppIcon60x60/@3x.png", options)
            .unwrap();
        let chunk = [0u8; 8192];
        for _ in 0..(MAX_ICON_BYTES / chunk.len() as u64 + 1) {
            writer.write_all(&chunk).unwrap();
        }
        let png = b"\x89PNG\r\n\x1a\nnormal-icon";
        writer
            .start_file("Payload/Test.app/AppIcon60x60/@2x.png", options)
            .unwrap();
        writer.write_all(png).unwrap();
        let mut zip =
            zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert_eq!(
            extract_largest_app_icon_from_zip(&mut zip, "Test.app").unwrap(),
            png
        );
    }
}
