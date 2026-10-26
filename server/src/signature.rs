use base64::Engine;
use plist::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, Write};
use std::path::Path;
use zip::ZipArchive;

const MH_MAGIC: u32 = 0xfeedface;
const MH_CIGAM: u32 = 0xcefaedfe;
const MH_MAGIC_64: u32 = 0xfeedfacf;
const MH_CIGAM_64: u32 = 0xcffaedfe;
const FAT_MAGIC: u32 = 0xcafebabe;
const FAT_CIGAM: u32 = 0xbebafeca;
const FAT_MAGIC_64: u32 = 0xcafebabf;
const FAT_CIGAM_64: u32 = 0xbfbafeca;
const LC_ENCRYPTION_INFO: u32 = 0x21;
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureMetadata {
    pub bundle_display_name: Option<String>,
    pub bundle_short_version_string: Option<String>,
    pub bundle_id: Option<String>,
    pub artwork_url: Option<String>,
    pub artist_name: Option<String>,
    pub apple_id: Option<String>,
    pub user_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sinf {
    pub id: String,
    pub sinf: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestTargets {
    #[serde(rename = "sinfPaths")]
    pub sinf_paths: Vec<String>,
    #[serde(rename = "sinfReplicationPaths")]
    pub sinf_replication_paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpaInspection {
    pub has_sc_info_manifest: bool,
    pub has_embedded_mobileprovision: bool,
    pub declared_sinf_paths: Vec<String>,
    pub present_sinf_paths: Vec<String>,
    pub missing_sinf_paths: Vec<String>,
    pub encrypted_binaries: Vec<String>,
    pub direct_install_ok: bool,
    pub blocked_reason: Option<String>,
    pub recommended_action: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Default)]
pub struct SignatureReplacement {
    pub path: String,
    pub signature_index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SignatureApplyResult {
    pub applied_paths: Vec<String>,
    pub replacements: Vec<SignatureReplacement>,
    pub warning: Option<String>,
}

/// Injects iTunesMetadata.plist and .sinf files into a downloaded IPA.
///
/// Changes are staged in memory and applied by `write`, which streams the archive into a
/// sibling temp file: untouched entries are raw-copied (no decompress/recompress), so memory
/// use stays bounded regardless of IPA size and original compression is preserved.
#[derive(Debug, Clone)]
pub struct SignatureClient {
    filename: String,
    raw_metadata: Option<serde_json::Map<String, serde_json::Value>>,
    metadata: SignatureMetadata,
    signatures: Vec<Sinf>,
    email: String,
    staged_entries: Vec<(String, Vec<u8>)>,
}

fn normalize_sinf_id(value: &serde_json::Value, fallback_index: usize) -> String {
    value
        .as_i64()
        .map(|v| v.to_string())
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .or_else(|| value.as_str().map(|v| v.to_string()))
        .unwrap_or_else(|| fallback_index.to_string())
}

fn parse_sinf_entry(value: &serde_json::Value, fallback_index: usize) -> Option<Sinf> {
    let sinf = value.get("sinf")?.as_str()?.trim().to_string();
    if sinf.is_empty() {
        return None;
    }
    Some(Sinf {
        id: normalize_sinf_id(
            value.get("id").unwrap_or(&serde_json::Value::Null),
            fallback_index,
        ),
        sinf,
    })
}

fn ordered_unique_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for path in paths {
        if seen.insert(path.clone()) {
            ordered.push(path);
        }
    }
    ordered
}

fn find_app_bundle_name<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // name_for_index only touches the central directory; by_index would seek to and parse
    // every local header, which is slow for IPAs with tens of thousands of entries.
    for i in 0..zip.len() {
        let Some(zip_name) = zip.name_for_index(i) else {
            continue;
        };
        if zip_name.starts_with("Payload/") && zip_name.ends_with(".app/") {
            let bundle = zip_name
                .strip_prefix("Payload/")
                .and_then(|s| s.strip_suffix('/'))
                .unwrap_or(zip_name)
                .to_string();
            return Ok(bundle);
        }
    }
    Err("Could not find app bundle".into())
}

fn read_zip_entry_bytes<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    path: &str,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let mut file = zip.by_name(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(content)
}

fn read_zip_plist<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    path: &str,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let content = read_zip_entry_bytes(zip, path)?;
    Ok(plist::from_bytes(&content)?)
}

fn manifest_targets_from_value(manifest: &Value) -> ManifestTargets {
    let mut targets = ManifestTargets::default();
    if let Value::Dictionary(dict) = manifest {
        targets.sinf_paths = dict
            .get("SinfPaths")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_string().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        targets.sinf_replication_paths = dict
            .get("SinfReplicationPaths")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_string().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    }
    targets
}

fn read_manifest_targets<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    app_bundle_name: &str,
) -> Result<Option<ManifestTargets>, Box<dyn std::error::Error + Send + Sync>> {
    let manifest_path = format!("Payload/{}/SC_Info/Manifest.plist", app_bundle_name);
    let manifest = match read_zip_plist(zip, &manifest_path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(Some(manifest_targets_from_value(&manifest)))
}

fn read_bundle_executable<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    app_bundle_name: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let info_path = format!("Payload/{}/Info.plist", app_bundle_name);
    let info = match read_zip_plist(zip, &info_path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    let executable = match info {
        Value::Dictionary(dict) => dict
            .get("CFBundleExecutable")
            .and_then(|value| value.as_string())
            .map(|value| value.to_string()),
        _ => None,
    };

    Ok(executable)
}

fn read_bundle_identifier<R: Read + Seek>(
    zip: &mut ZipArchive<R>,
    app_bundle_name: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let info_path = format!("Payload/{}/Info.plist", app_bundle_name);
    let info = match read_zip_plist(zip, &info_path) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };

    let bundle_id = match info {
        Value::Dictionary(dict) => dict
            .get("CFBundleIdentifier")
            .and_then(|value| value.as_string())
            .map(|value| value.to_string()),
        _ => None,
    };

    Ok(bundle_id)
}

pub fn read_bundle_identifier_from_ipa(
    path: &Path,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut zip = read_zip(&path.to_string_lossy())?;
    let app_bundle_name = find_app_bundle_name(&mut zip)?;
    read_bundle_identifier(&mut zip, &app_bundle_name)
}

fn decode_signatures(
    signatures: &[Sinf],
) -> Result<Vec<Vec<u8>>, Box<dyn std::error::Error + Send + Sync>> {
    signatures
        .iter()
        .map(|signature| Ok(base64::engine::general_purpose::STANDARD.decode(&signature.sinf)?))
        .collect()
}

fn sinf_basename(path: &str) -> Option<String> {
    let file_name = path.rsplit('/').next()?;
    Some(file_name.trim_end_matches(".sinf").to_string())
}

fn build_injection_plan(
    signatures: &[Sinf],
    manifest_targets: &ManifestTargets,
) -> Result<SignatureApplyResult, Box<dyn std::error::Error + Send + Sync>> {
    let primary_paths = ordered_unique_paths(manifest_targets.sinf_paths.iter().cloned());
    let replication_paths = ordered_unique_paths(
        manifest_targets
            .sinf_replication_paths
            .iter()
            .filter(|path| !primary_paths.contains(path))
            .cloned(),
    );
    let target_paths = ordered_unique_paths(
        primary_paths
            .iter()
            .cloned()
            .chain(replication_paths.iter().cloned()),
    );

    if target_paths.is_empty() {
        return Ok(SignatureApplyResult {
            applied_paths: Vec::new(),
            replacements: Vec::new(),
            warning: Some("包内未声明需要补齐的 .sinf 目标".to_string()),
        });
    }

    if signatures.is_empty() {
        return Ok(SignatureApplyResult {
            applied_paths: Vec::new(),
            replacements: Vec::new(),
            warning: Some(format!(
                "包内声明了 {} 个 .sinf 目标，但 Apple 下载响应未返回任何真实 sinf",
                target_paths.len()
            )),
        });
    }

    if signatures.len() == 1 {
        let replacements = target_paths
            .iter()
            .cloned()
            .map(|path| SignatureReplacement {
                path,
                signature_index: 0,
            })
            .collect::<Vec<_>>();
        return Ok(SignatureApplyResult {
            applied_paths: target_paths,
            replacements,
            warning: None,
        });
    }

    if signatures.len() == target_paths.len() {
        let replacements = target_paths
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, path)| SignatureReplacement {
                path,
                signature_index: index,
            })
            .collect::<Vec<_>>();
        return Ok(SignatureApplyResult {
            applied_paths: target_paths,
            replacements,
            warning: None,
        });
    }

    if signatures.len() == primary_paths.len() && !replication_paths.is_empty() {
        let mut primary_name_to_index = std::collections::HashMap::new();
        for (index, path) in primary_paths.iter().enumerate() {
            let Some(base) = sinf_basename(path) else {
                return Ok(SignatureApplyResult {
                    applied_paths: Vec::new(),
                    replacements: Vec::new(),
                    warning: Some("存在无法解析 basename 的主 .sinf 路径，跳过注入".to_string()),
                });
            };
            if primary_name_to_index.insert(base, index).is_some() {
                return Ok(SignatureApplyResult {
                    applied_paths: Vec::new(),
                    replacements: Vec::new(),
                    warning: Some(
                        "主 SinfPaths 出现重复 basename，无法安全映射 replication 目标".to_string(),
                    ),
                });
            }
        }

        let mut replacements = primary_paths
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, path)| SignatureReplacement {
                path,
                signature_index: index,
            })
            .collect::<Vec<_>>();

        for path in replication_paths.iter().cloned() {
            let Some(base) = sinf_basename(&path) else {
                return Ok(SignatureApplyResult {
                    applied_paths: Vec::new(),
                    replacements: Vec::new(),
                    warning: Some(format!(
                        "replication 目标 {} 无法解析 basename，跳过注入",
                        path
                    )),
                });
            };
            let Some(signature_index) = primary_name_to_index.get(&base).copied() else {
                return Ok(SignatureApplyResult {
                    applied_paths: Vec::new(),
                    replacements: Vec::new(),
                    warning: Some(format!(
                        "replication 目标 {} 找不到同 basename 的主 SinfPath，跳过注入",
                        path
                    )),
                });
            };
            replacements.push(SignatureReplacement {
                path,
                signature_index,
            });
        }

        let applied_paths = replacements.iter().map(|item| item.path.clone()).collect();
        return Ok(SignatureApplyResult {
            applied_paths,
            replacements,
            warning: None,
        });
    }

    Ok(SignatureApplyResult {
        applied_paths: Vec::new(),
        replacements: Vec::new(),
        warning: Some(format!(
            "Apple 返回 sinf 数量 ({}) 与包内声明目标数量 ({}) 不匹配，跳过注入",
            signatures.len(),
            target_paths.len()
        )),
    })
}

/// Copy every entry of `source` into `writer` except the staged paths, then append the staged
/// entries. Untouched files are raw-copied so their compressed bytes, CRC and permissions are
/// preserved and nothing is decompressed into memory.
fn rewrite_archive<R: Read + Seek, W: Write + Seek>(
    source: &mut ZipArchive<R>,
    writer: &mut zip::ZipWriter<W>,
    staged_entries: &[(String, Vec<u8>)],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let staged_paths = staged_entries
        .iter()
        .map(|(path, _)| path.as_str())
        .collect::<HashSet<_>>();

    for i in 0..source.len() {
        let file = source.by_index_raw(i)?;
        if staged_paths.contains(file.name()) {
            continue;
        }
        if file.is_dir() {
            // raw_copy_file would OR S_IFREG into a directory's mode, so re-add it explicitly.
            let mut options: zip::write::SimpleFileOptions =
                zip::write::SimpleFileOptions::default();
            if let Some(modified) = file.last_modified().filter(|value| value.is_valid()) {
                options = options.last_modified_time(modified);
            }
            if let Some(mode) = file.unix_mode() {
                options = options.unix_permissions(mode & 0o7777);
            }
            let name = file.name().to_string();
            drop(file);
            writer.add_directory(name, options)?;
            continue;
        }
        writer.raw_copy_file(file)?;
    }

    let options: zip::write::SimpleFileOptions =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (path, data) in staged_entries {
        writer.start_file(path.as_str(), options)?;
        writer.write_all(data)?;
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(if little_endian {
        u32::from_le_bytes(slice.try_into().ok()?)
    } else {
        u32::from_be_bytes(slice.try_into().ok()?)
    })
}

fn parse_macho_load_commands(bytes: &[u8], little_endian: bool, is_64: bool) -> Option<bool> {
    let header_size = if is_64 { 32 } else { 28 };
    let ncmds = read_u32(bytes, 16, little_endian)? as usize;
    let mut offset = header_size;

    for _ in 0..ncmds {
        let cmd = read_u32(bytes, offset, little_endian)?;
        let cmdsize = read_u32(bytes, offset + 4, little_endian)? as usize;
        if cmdsize < 8 || offset.checked_add(cmdsize)? > bytes.len() {
            return None;
        }
        if matches!(cmd, LC_ENCRYPTION_INFO | LC_ENCRYPTION_INFO_64) {
            let cryptid = read_u32(bytes, offset + 16, little_endian)?;
            return Some(cryptid == 1);
        }
        offset += cmdsize;
    }

    Some(false)
}

fn macho_cryptid_one(bytes: &[u8]) -> Option<bool> {
    let magic_be = read_u32(bytes, 0, false)?;
    let magic_le = read_u32(bytes, 0, true)?;

    match magic_le {
        MH_MAGIC => return parse_macho_load_commands(bytes, true, false),
        MH_MAGIC_64 => return parse_macho_load_commands(bytes, true, true),
        FAT_CIGAM => {
            let nfat_arch = read_u32(bytes, 4, true)? as usize;
            for index in 0..nfat_arch {
                let arch_offset = 8 + index * 20;
                let slice_offset = read_u32(bytes, arch_offset + 8, true)? as usize;
                let slice_size = read_u32(bytes, arch_offset + 12, true)? as usize;
                let slice = bytes.get(slice_offset..slice_offset.checked_add(slice_size)?)?;
                if macho_cryptid_one(slice)? {
                    return Some(true);
                }
            }
            return Some(false);
        }
        FAT_CIGAM_64 => {
            let nfat_arch = read_u32(bytes, 4, true)? as usize;
            for index in 0..nfat_arch {
                let arch_offset = 8 + index * 32;
                let hi = read_u32(bytes, arch_offset + 8, true)? as u64;
                let lo = read_u32(bytes, arch_offset + 12, true)? as u64;
                let size_hi = read_u32(bytes, arch_offset + 16, true)? as u64;
                let size_lo = read_u32(bytes, arch_offset + 20, true)? as u64;
                let slice_offset = ((hi << 32) | lo) as usize;
                let slice_size = ((size_hi << 32) | size_lo) as usize;
                let slice = bytes.get(slice_offset..slice_offset.checked_add(slice_size)?)?;
                if macho_cryptid_one(slice)? {
                    return Some(true);
                }
            }
            return Some(false);
        }
        _ => {}
    }

    match magic_be {
        MH_CIGAM => parse_macho_load_commands(bytes, false, false),
        MH_CIGAM_64 => parse_macho_load_commands(bytes, false, true),
        FAT_MAGIC => {
            let nfat_arch = read_u32(bytes, 4, false)? as usize;
            for index in 0..nfat_arch {
                let arch_offset = 8 + index * 20;
                let slice_offset = read_u32(bytes, arch_offset + 8, false)? as usize;
                let slice_size = read_u32(bytes, arch_offset + 12, false)? as usize;
                let slice = bytes.get(slice_offset..slice_offset.checked_add(slice_size)?)?;
                if macho_cryptid_one(slice)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        FAT_MAGIC_64 => {
            let nfat_arch = read_u32(bytes, 4, false)? as usize;
            for index in 0..nfat_arch {
                let arch_offset = 8 + index * 32;
                let hi = read_u32(bytes, arch_offset + 8, false)? as u64;
                let lo = read_u32(bytes, arch_offset + 12, false)? as u64;
                let size_hi = read_u32(bytes, arch_offset + 16, false)? as u64;
                let size_lo = read_u32(bytes, arch_offset + 20, false)? as u64;
                let slice_offset = ((hi << 32) | lo) as usize;
                let slice_size = ((size_hi << 32) | size_lo) as usize;
                let slice = bytes.get(slice_offset..slice_offset.checked_add(slice_size)?)?;
                if macho_cryptid_one(slice)? {
                    return Some(true);
                }
            }
            Some(false)
        }
        _ => None,
    }
}

pub fn inspect_ipa_path(
    path: &Path,
) -> Result<IpaInspection, Box<dyn std::error::Error + Send + Sync>> {
    let mut zip = read_zip(&path.to_string_lossy())?;
    let app_bundle_name = find_app_bundle_name(&mut zip)?;
    let manifest_targets = read_manifest_targets(&mut zip, &app_bundle_name)?.unwrap_or_default();
    let declared_sinf_paths = ordered_unique_paths(
        manifest_targets
            .sinf_paths
            .iter()
            .cloned()
            .chain(manifest_targets.sinf_replication_paths.iter().cloned()),
    );

    let app_prefix = format!("Payload/{}/", app_bundle_name);
    let mut present_sinf_paths = Vec::new();
    let mut plugin_dirs = HashSet::new();
    for name in zip.file_names() {
        if name.starts_with(&app_prefix) && name.ends_with(".sinf") {
            present_sinf_paths.push(name.trim_start_matches(&app_prefix).to_string());
        }
        if name.starts_with(&app_prefix) && name.contains("/PlugIns/") && name.ends_with(".appex/")
        {
            plugin_dirs.insert(name.trim_end_matches('/').to_string());
        }
    }
    present_sinf_paths = ordered_unique_paths(present_sinf_paths);

    let present_sinf_set = present_sinf_paths.iter().cloned().collect::<HashSet<_>>();
    let missing_sinf_paths = declared_sinf_paths
        .iter()
        .filter(|path| !present_sinf_set.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let mut encrypted_binaries = Vec::new();
    let app_info_path = format!("Payload/{}/Info.plist", app_bundle_name);
    if let Ok(Value::Dictionary(info)) = read_zip_plist(&mut zip, &app_info_path) {
        if let Some(executable) = info
            .get("CFBundleExecutable")
            .and_then(|value| value.as_string())
        {
            let binary_path = format!("Payload/{}/{}", app_bundle_name, executable);
            if let Ok(binary) = read_zip_entry_bytes(&mut zip, &binary_path) {
                if macho_cryptid_one(&binary).unwrap_or(false) {
                    encrypted_binaries.push(binary_path.trim_start_matches("Payload/").to_string());
                }
            }
        }
    }

    for plugin_dir in ordered_unique_paths(plugin_dirs) {
        let info_path = format!("{}/Info.plist", plugin_dir);
        if let Ok(Value::Dictionary(info)) = read_zip_plist(&mut zip, &info_path) {
            if let Some(executable) = info
                .get("CFBundleExecutable")
                .and_then(|value| value.as_string())
            {
                let binary_path = format!("{}/{}", plugin_dir, executable);
                if let Ok(binary) = read_zip_entry_bytes(&mut zip, &binary_path) {
                    if macho_cryptid_one(&binary).unwrap_or(false) {
                        encrypted_binaries
                            .push(binary_path.trim_start_matches("Payload/").to_string());
                    }
                }
            }
        }
    }

    let has_sc_info_manifest = !declared_sinf_paths.is_empty();
    let sinf_fully_injected = has_sc_info_manifest && missing_sinf_paths.is_empty();
    let has_embedded_mobileprovision = zip
        .by_name(&format!(
            "Payload/{}/embedded.mobileprovision",
            app_bundle_name
        ))
        .is_ok();

    let mut blockers = Vec::new();

    // Missing sinf files that the manifest declares → always a blocker.
    if !missing_sinf_paths.is_empty() {
        blockers.push(format!(
            "包内声明了 {} 个 .sinf 目标，但缺少 {} 个：{}",
            declared_sinf_paths.len(),
            missing_sinf_paths.len(),
            missing_sinf_paths.join(", ")
        ));
    }

    if sinf_fully_injected {
        // App Store IPA with all declared sinfs properly injected.
        // FairPlay encrypted binaries are expected and will be handled by iOS
        // at runtime using the sinf data — this is how ipatool.js and
        // ApplePackage (Asspp) work. Do NOT block OTA install.
        if !encrypted_binaries.is_empty() {
            log::info!(
                "[inspect] {} encrypted binaries present but all {} sinfs injected — OTA installable (App Store package)",
                encrypted_binaries.len(),
                present_sinf_paths.len(),
            );
        }
    } else if has_embedded_mobileprovision {
        // Developer-signed / sideloaded IPA with provisioning profile.
        if !encrypted_binaries.is_empty() {
            blockers.push(format!(
                "检测到 {} 个 FairPlay 加密二进制，这类包通常不是可直接侧载的成品 IPA",
                encrypted_binaries.len()
            ));
        }
    } else if !encrypted_binaries.is_empty() {
        // Encrypted binaries with no sinfs and no provisioning profile → unrunnable.
        blockers.push(format!(
            "检测到 {} 个 FairPlay 加密二进制，且未发现 embedded.mobileprovision，这类包不能直接侧载，继续安装大概率黑屏或闪退",
            encrypted_binaries.len()
        ));
    } else {
        // No encryption, no provisioning profile, no SC_Info → likely broken.
        blockers.push(
            "包内未发现 embedded.mobileprovision，当前看起来不像已正确重签的可侧载 IPA".to_string(),
        );
    }

    let direct_install_ok = blockers.is_empty();
    let blocked_reason = (!blockers.is_empty()).then(|| blockers.join("；"));
    let recommended_action = blocked_reason.as_ref().map(|reason| {
        if reason.contains("缺少") {
            "请确认 Apple 下载响应是否返回了完整的 sinf 数据".to_string()
        } else {
            "请先获取完整解密并正确重签（含全部 .appex）的 IPA，再重新上传或安装".to_string()
        }
    });
    let summary = if sinf_fully_injected && !encrypted_binaries.is_empty() {
        format!(
            "App Store 签名包：已注入 {} 个 .sinf（含 {} 个加密二进制），iOS 运行时负责 FairPlay 解密",
            present_sinf_paths.len(),
            encrypted_binaries.len()
        )
    } else {
        match (&blocked_reason, &recommended_action) {
            (Some(reason), Some(action)) => format!("{}。{}。", reason, action),
            (Some(reason), None) => reason.clone(),
            _ if has_sc_info_manifest => "未检测到缺失的 .sinf 目标，可继续安装验证".to_string(),
            _ => "未发现明显的 FairPlay / 签名阻塞，可继续安装验证".to_string(),
        }
    };

    Ok(IpaInspection {
        has_sc_info_manifest,
        has_embedded_mobileprovision,
        declared_sinf_paths,
        present_sinf_paths,
        missing_sinf_paths,
        encrypted_binaries,
        direct_install_ok,
        blocked_reason,
        recommended_action,
        summary,
    })
}

fn json_value_to_plist(value: &serde_json::Value) -> plist::Value {
    match value {
        serde_json::Value::String(s) => plist::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                plist::Value::Integer(plist::Integer::from(i))
            } else if let Some(f) = n.as_f64() {
                plist::Value::Real(f)
            } else {
                plist::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => plist::Value::Boolean(*b),
        serde_json::Value::Null => plist::Value::String("".to_string()),
        serde_json::Value::Array(arr) => {
            plist::Value::Array(arr.iter().map(json_value_to_plist).collect())
        }
        serde_json::Value::Object(map) => plist::Value::Dictionary(json_map_to_plist_dict(map)),
    }
}

fn json_map_to_plist_dict(map: &serde_json::Map<String, serde_json::Value>) -> plist::Dictionary {
    let mut dict = plist::Dictionary::new();
    for (key, value) in map {
        dict.insert(key.clone(), json_value_to_plist(value));
    }
    dict
}

impl SignatureClient {
    pub fn new(
        song_list_0: &serde_json::Value,
        email: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let raw_metadata = song_list_0
            .get("metadata")
            .and_then(|value| value.as_object())
            .cloned();

        let metadata = SignatureMetadata {
            bundle_display_name: song_list_0["metadata"]["bundleDisplayName"]
                .as_str()
                .map(|s| s.to_string()),
            bundle_short_version_string: song_list_0["metadata"]["bundleShortVersionString"]
                .as_str()
                .map(|s| s.to_string()),
            bundle_id: song_list_0["metadata"]["bundleId"]
                .as_str()
                .map(|s| s.to_string()),
            artwork_url: {
                let url_60 = song_list_0["metadata"]["artworkUrl60"].as_str();
                let url_512 = song_list_0["metadata"]["artworkUrl512"].as_str();
                let url_100 = song_list_0["metadata"]["artworkUrl100"].as_str();
                url_60.or(url_512).or(url_100).map(|s| s.to_string())
            },
            artist_name: song_list_0["metadata"]["artistName"]
                .as_str()
                .map(|s| s.to_string()),
            apple_id: Some(email.to_string()),
            user_name: Some(email.to_string()),
        };

        let signatures = song_list_0["sinfs"]
            .as_array()
            .map(|sinfs| {
                sinfs
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sinf)| parse_sinf_entry(sinf, index))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if signatures.is_empty() {
            eprintln!(
                "[signature] WARNING: no usable sinf entries found in Apple response; keys={}, sinfs={:?}",
                song_list_0
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>().join(","))
                    .unwrap_or_default(),
                song_list_0["sinfs"]
            );
        }

        Ok(SignatureClient {
            filename: String::new(),
            raw_metadata,
            metadata,
            signatures,
            email: email.to_string(),
            staged_entries: Vec::new(),
        })
    }

    pub fn load_file(
        &mut self,
        path: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Only the central directory is read here; entry data is streamed later by `write`.
        read_zip(path)?;
        self.filename = path.to_string();
        self.staged_entries.clear();
        Ok(())
    }

    /// Stage an entry to be added (or to replace an existing entry with the same path).
    fn stage_entry(&mut self, path: String, data: Vec<u8>) {
        self.staged_entries
            .retain(|(staged_path, _)| *staged_path != path);
        self.staged_entries.push((path, data));
    }

    pub fn append_metadata(&mut self) -> &mut Self {
        // Build metadata plist from Apple's raw response if available (matches ApplePackage behaviour),
        // otherwise fall back to the extracted SignatureMetadata fields.
        let metadata_content = if let Some(raw) = &self.raw_metadata {
            let mut map = raw.clone();
            map.insert(
                "apple-id".to_string(),
                serde_json::Value::String(self.email.clone()),
            );
            map.insert(
                "userName".to_string(),
                serde_json::Value::String(self.email.clone()),
            );

            // Convert JSON map → plist Dictionary
            let dict = json_map_to_plist_dict(&map);
            let value = plist::Value::Dictionary(dict);
            let mut buf = Vec::new();
            let options = plist::XmlWriteOptions::default();
            if let Err(e) = plist::to_writer_xml_with_options(&mut buf, &value, &options) {
                log::warn!("append_metadata: failed to serialize raw Apple metadata to plist: {e}, falling back to extracted fields");
                return self.build_metadata_fallback();
            }
            match String::from_utf8(buf) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("append_metadata: raw Apple metadata produced non-UTF-8 plist: {e}, falling back to extracted fields");
                    return self.build_metadata_fallback();
                }
            }
        } else {
            return self.build_metadata_fallback();
        };

        self.write_metadata_to_archive(&metadata_content)
    }

    /// Fallback: build iTunesMetadata.plist from the extracted SignatureMetadata fields.
    fn build_metadata_fallback(&mut self) -> &mut Self {
        let mut dict = plist::Dictionary::new();
        if let Some(name) = &self.metadata.bundle_display_name {
            dict.insert(
                "bundleDisplayName".to_string(),
                plist::Value::String(name.clone()),
            );
        }
        if let Some(version) = &self.metadata.bundle_short_version_string {
            dict.insert(
                "bundleShortVersionString".to_string(),
                plist::Value::String(version.clone()),
            );
        }
        if let Some(bundle_id) = &self.metadata.bundle_id {
            dict.insert(
                "bundleId".to_string(),
                plist::Value::String(bundle_id.clone()),
            );
        }
        if let Some(artwork_url) = &self.metadata.artwork_url {
            dict.insert(
                "artworkUrl".to_string(),
                plist::Value::String(artwork_url.clone()),
            );
        }
        if let Some(artist_name) = &self.metadata.artist_name {
            dict.insert(
                "artistName".to_string(),
                plist::Value::String(artist_name.clone()),
            );
        }
        dict.insert(
            "apple-id".to_string(),
            plist::Value::String(self.email.clone()),
        );
        dict.insert(
            "userName".to_string(),
            plist::Value::String(self.email.clone()),
        );

        let value = plist::Value::Dictionary(dict);
        let mut buf = Vec::new();
        if let Err(e) =
            plist::to_writer_xml_with_options(&mut buf, &value, &plist::XmlWriteOptions::default())
        {
            log::error!("append_metadata: fallback metadata serialization also failed: {e}");
            // Nothing we can do — leave archive unchanged.
            return self;
        }
        let metadata_content = match String::from_utf8(buf) {
            Ok(s) => s,
            Err(e) => {
                log::error!("append_metadata: fallback metadata produced non-UTF-8: {e}");
                return self;
            }
        };

        self.write_metadata_to_archive(&metadata_content)
    }

    fn write_metadata_to_archive(&mut self, metadata_content: &str) -> &mut Self {
        self.stage_entry(
            "iTunesMetadata.plist".to_string(),
            metadata_content.as_bytes().to_vec(),
        );
        self
    }

    pub fn append_signatures(
        &mut self,
    ) -> Result<SignatureApplyResult, Box<dyn std::error::Error + Send + Sync>> {
        let mut zip = read_zip(&self.filename)?;
        let app_bundle_name = find_app_bundle_name(&mut zip)?;
        let apply_result = if let Some(manifest_targets) =
            read_manifest_targets(&mut zip, &app_bundle_name)?
        {
            build_injection_plan(&self.signatures, &manifest_targets)?
        } else if let Some(executable) = read_bundle_executable(&mut zip, &app_bundle_name)? {
            if self.signatures.is_empty() {
                SignatureApplyResult {
                    applied_paths: Vec::new(),
                    replacements: Vec::new(),
                    warning: Some(
                        "包内无 Manifest，且 Apple 下载响应未返回任何真实 sinf".to_string(),
                    ),
                }
            } else {
                let path = format!("SC_Info/{}.sinf", executable);
                SignatureApplyResult {
                    applied_paths: vec![path.clone()],
                    replacements: vec![SignatureReplacement {
                        path,
                        signature_index: 0,
                    }],
                    warning: Some(
                        "包内未找到 SC_Info/Manifest.plist，已按主 app 可执行文件回退注入首个 sinf"
                            .to_string(),
                    ),
                }
            }
        } else {
            SignatureApplyResult {
                applied_paths: Vec::new(),
                replacements: Vec::new(),
                warning: Some("包内未找到 SC_Info/Manifest.plist，且无法从 Info.plist 推断 sinf 路径，跳过注入".to_string()),
            }
        };

        if apply_result.applied_paths.is_empty() {
            return Ok(apply_result);
        }

        let decoded_signatures = decode_signatures(&self.signatures)?;
        for replacement in &apply_result.replacements {
            self.stage_entry(
                format!("Payload/{}/{}", app_bundle_name, replacement.path),
                decoded_signatures[replacement.signature_index].clone(),
            );
        }
        Ok(apply_result)
    }

    /// Apply the staged entries by streaming the archive into a temp file next to the
    /// original, then atomically renaming it over the original.
    pub fn write(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if self.staged_entries.is_empty() {
            return Ok(());
        }

        let temp_path = format!("{}.rewrite-{}", self.filename, std::process::id());
        let rewritten = (|| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let mut source = read_zip(&self.filename)?;
            let output = BufWriter::new(File::create(&temp_path)?);
            let mut writer = zip::ZipWriter::new(output);
            rewrite_archive(&mut source, &mut writer, &self.staged_entries)?;
            let mut output = writer.finish()?;
            output.flush()?;
            output.get_ref().sync_all()?;
            Ok(())
        })();

        if let Err(error) = rewritten {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error);
        }
        if let Err(error) = std::fs::rename(&temp_path, &self.filename) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(error.into());
        }
        self.staged_entries.clear();
        Ok(())
    }
}

pub fn read_zip(
    path: &str,
) -> Result<ZipArchive<std::fs::File>, Box<dyn std::error::Error + Send + Sync>> {
    let file = File::open(path)?;
    let zip = ZipArchive::new(file)?;
    Ok(zip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn empty_metadata() -> SignatureMetadata {
        SignatureMetadata {
            bundle_display_name: None,
            bundle_short_version_string: None,
            bundle_id: None,
            artwork_url: None,
            artist_name: None,
            apple_id: None,
            user_name: None,
        }
    }

    /// Write `archive` to a unique temp file and return a client loaded from it.
    fn client_for_archive(
        archive: Vec<u8>,
        raw_metadata: Option<serde_json::Map<String, serde_json::Value>>,
        signatures: Vec<Sinf>,
    ) -> SignatureClient {
        let path = std::env::temp_dir().join(format!(
            "signature-test-{}-{}.ipa",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, archive).unwrap();
        let mut client = SignatureClient {
            filename: String::new(),
            raw_metadata,
            metadata: empty_metadata(),
            signatures,
            email: "tester@example.com".to_string(),
            staged_entries: Vec::new(),
        };
        client.load_file(&path.to_string_lossy()).unwrap();
        client
    }

    /// Apply staged changes and return the rewritten archive, removing the temp file.
    fn written_archive(client: &mut SignatureClient) -> ZipArchive<Cursor<Vec<u8>>> {
        client.write().unwrap();
        let bytes = std::fs::read(&client.filename).unwrap();
        let _ = std::fs::remove_file(&client.filename);
        ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    fn sample_signatures(count: usize) -> Vec<Sinf> {
        (0..count)
            .map(|index| Sinf {
                id: index.to_string(),
                sinf: base64::engine::general_purpose::STANDARD.encode(format!("sinf-{index}")),
            })
            .collect()
    }

    fn write_zip(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
        let mut out = Vec::new();
        let cursor = Cursor::new(&mut out);
        let mut zip = zip::ZipWriter::new(cursor);
        let file_options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let dir_options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

        for (path, content) in entries {
            if path.ends_with('/') {
                zip.add_directory(path, dir_options).unwrap();
            } else {
                zip.start_file(path, file_options).unwrap();
                zip.write_all(&content).unwrap();
            }
        }
        let _ = zip.finish().unwrap();
        out
    }

    fn info_plist(executable: &str) -> Vec<u8> {
        let mut dict = plist::Dictionary::new();
        dict.insert(
            "CFBundleExecutable".to_string(),
            Value::String(executable.to_string()),
        );
        let value = Value::Dictionary(dict);
        let mut out = Vec::new();
        plist::to_writer_xml(&mut out, &value).unwrap();
        out
    }

    #[test]
    fn build_injection_plan_maps_replication_to_same_basename() {
        let signatures = sample_signatures(2);
        let manifest_targets = ManifestTargets {
            sinf_paths: vec![
                "SC_Info/Main.sinf".to_string(),
                "PlugIns/Widget.appex/SC_Info/Widget.sinf".to_string(),
            ],
            sinf_replication_paths: vec!["Extensions/Copy/SC_Info/Widget.sinf".to_string()],
        };

        let plan = build_injection_plan(&signatures, &manifest_targets).unwrap();
        assert_eq!(
            plan.applied_paths,
            vec![
                "SC_Info/Main.sinf",
                "PlugIns/Widget.appex/SC_Info/Widget.sinf",
                "Extensions/Copy/SC_Info/Widget.sinf"
            ]
        );
        assert_eq!(plan.replacements.len(), 3);
        assert_eq!(plan.replacements[0].signature_index, 0);
        assert_eq!(plan.replacements[1].signature_index, 1);
        assert_eq!(plan.replacements[2].signature_index, 1);
        assert!(plan.warning.is_none());
    }

    #[test]
    fn build_injection_plan_rejects_unresolvable_replication_mapping() {
        let signatures = sample_signatures(2);
        let manifest_targets = ManifestTargets {
            sinf_paths: vec![
                "SC_Info/Main.sinf".to_string(),
                "PlugIns/Widget.appex/SC_Info/Widget.sinf".to_string(),
            ],
            sinf_replication_paths: vec!["Extensions/Copy/SC_Info/Unknown.sinf".to_string()],
        };

        let plan = build_injection_plan(&signatures, &manifest_targets).unwrap();
        assert!(plan.applied_paths.is_empty());
        assert!(plan.replacements.is_empty());
        assert!(plan
            .warning
            .unwrap_or_default()
            .contains("找不到同 basename 的主 SinfPath"));
    }

    #[test]
    fn append_signatures_falls_back_to_info_plist_when_manifest_missing() {
        let archive = write_zip(vec![
            ("Payload/Test.app/", Vec::new()),
            ("Payload/Test.app/Info.plist", info_plist("TestExec")),
        ]);

        let mut client = client_for_archive(
            archive,
            None,
            vec![Sinf {
                id: "0".to_string(),
                sinf: base64::engine::general_purpose::STANDARD.encode(b"fallback-sinf"),
            }],
        );

        let result = client.append_signatures().unwrap();
        assert_eq!(result.applied_paths, vec!["SC_Info/TestExec.sinf"]);
        assert!(result
            .warning
            .unwrap_or_default()
            .contains("回退注入首个 sinf"));

        let mut zip = written_archive(&mut client);
        let mut entry = zip
            .by_name("Payload/Test.app/SC_Info/TestExec.sinf")
            .unwrap();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"fallback-sinf");
    }

    #[test]
    fn append_signatures_injects_replication_with_matching_primary_signature() {
        let mut manifest = plist::Dictionary::new();
        manifest.insert(
            "SinfPaths".to_string(),
            Value::Array(vec![
                Value::String("SC_Info/Main.sinf".to_string()),
                Value::String("PlugIns/Widget.appex/SC_Info/Widget.sinf".to_string()),
            ]),
        );
        manifest.insert(
            "SinfReplicationPaths".to_string(),
            Value::Array(vec![Value::String(
                "Extensions/Copy/SC_Info/Widget.sinf".to_string(),
            )]),
        );
        let mut manifest_bytes = Vec::new();
        plist::to_writer_xml(&mut manifest_bytes, &Value::Dictionary(manifest)).unwrap();

        let archive = write_zip(vec![
            ("Payload/Test.app/", Vec::new()),
            ("Payload/Test.app/Info.plist", info_plist("Main")),
            ("Payload/Test.app/SC_Info/Manifest.plist", manifest_bytes),
        ]);

        let signatures = vec![
            Sinf {
                id: "0".to_string(),
                sinf: base64::engine::general_purpose::STANDARD.encode(b"main-sinf"),
            },
            Sinf {
                id: "1".to_string(),
                sinf: base64::engine::general_purpose::STANDARD.encode(b"widget-sinf"),
            },
        ];

        let mut client = client_for_archive(archive, None, signatures);

        let result = client.append_signatures().unwrap();
        assert!(result.warning.is_none());

        let mut zip = written_archive(&mut client);

        let mut main_bytes = Vec::new();
        zip.by_name("Payload/Test.app/SC_Info/Main.sinf")
            .unwrap()
            .read_to_end(&mut main_bytes)
            .unwrap();

        let mut widget_bytes = Vec::new();
        zip.by_name("Payload/Test.app/PlugIns/Widget.appex/SC_Info/Widget.sinf")
            .unwrap()
            .read_to_end(&mut widget_bytes)
            .unwrap();

        let mut replication_bytes = Vec::new();
        zip.by_name("Payload/Test.app/Extensions/Copy/SC_Info/Widget.sinf")
            .unwrap()
            .read_to_end(&mut replication_bytes)
            .unwrap();

        assert_eq!(main_bytes, b"main-sinf");
        assert_eq!(widget_bytes, b"widget-sinf");
        assert_eq!(replication_bytes, b"widget-sinf");
    }

    #[test]
    fn append_metadata_preserves_raw_apple_metadata_fields() {
        let archive = write_zip(vec![
            ("Payload/Test.app/", Vec::new()),
            ("Payload/Test.app/Info.plist", info_plist("TestExec")),
        ]);

        let raw_metadata = serde_json::json!({
            "bundleDisplayName": "TestApp",
            "bundleShortVersionString": "3.2.1",
            "bundleVersion": "321",
            "bundleId": "com.test.app",
            "artworkUrl60": "https://example.com/icon60.png",
            "artworkUrl512": "https://example.com/icon512.png",
            "artistName": "TestArtist",
            "fileSizeBytes": 12345678,
            "releaseDate": "2025-01-15T08:00:00Z",
            "softwareVersionExternalIdentifiers": [{"externalVersionId":"v1"},{"externalVersionId":"v2"}],
            "someExtraField": "extra-value"
        })
        .as_object()
        .cloned()
        .unwrap();

        let mut client = client_for_archive(archive, Some(raw_metadata), Vec::new());

        client.append_metadata();

        let mut zip = written_archive(&mut client);
        let mut entry = zip.by_name("iTunesMetadata.plist").unwrap();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();

        let parsed = plist::from_bytes::<plist::Value>(&content).unwrap();
        let dict = parsed.as_dictionary().expect("expected dictionary");

        // Raw Apple fields preserved
        assert_eq!(
            dict.get("bundleDisplayName").and_then(|v| v.as_string()),
            Some("TestApp")
        );
        assert_eq!(
            dict.get("bundleVersion").and_then(|v| v.as_string()),
            Some("321")
        );
        assert_eq!(
            dict.get("fileSizeBytes")
                .and_then(|v| v.as_signed_integer()),
            Some(12345678)
        );
        assert_eq!(
            dict.get("someExtraField").and_then(|v| v.as_string()),
            Some("extra-value")
        );

        // Injected fields
        assert_eq!(
            dict.get("apple-id").and_then(|v| v.as_string()),
            Some("tester@example.com")
        );
        assert_eq!(
            dict.get("userName").and_then(|v| v.as_string()),
            Some("tester@example.com")
        );
    }

    #[test]
    fn write_preserves_untouched_entries_and_their_compression() {
        let mut out = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut out));
            let deflated: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);
            zip.add_directory(
                "Payload/Test.app/",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.start_file("Payload/Test.app/Info.plist", deflated)
                .unwrap();
            zip.write_all(&info_plist("TestExec")).unwrap();
            zip.start_file("Payload/Test.app/TestExec", deflated)
                .unwrap();
            zip.write_all(&vec![7u8; 64 * 1024]).unwrap();
            zip.start_file("iTunesMetadata.plist", deflated).unwrap();
            zip.write_all(b"stale").unwrap();
            zip.finish().unwrap();
        }

        let mut client = client_for_archive(
            out,
            None,
            vec![Sinf {
                id: "0".to_string(),
                sinf: base64::engine::general_purpose::STANDARD.encode(b"main-sinf"),
            }],
        );
        client.append_metadata();
        client.append_signatures().unwrap();
        let mut zip = written_archive(&mut client);

        let names = zip.file_names().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(
            names
                .iter()
                .filter(|name| *name == "iTunesMetadata.plist")
                .count(),
            1
        );

        let mut binary = zip.by_name("Payload/Test.app/TestExec").unwrap();
        assert_eq!(binary.compression(), zip::CompressionMethod::Deflated);
        assert_eq!(binary.unix_mode().map(|mode| mode & 0o777), Some(0o755));
        let mut content = Vec::new();
        binary.read_to_end(&mut content).unwrap();
        assert_eq!(content, vec![7u8; 64 * 1024]);
        drop(binary);

        assert!(zip.by_name("Payload/Test.app/").unwrap().is_dir());

        let mut metadata = Vec::new();
        zip.by_name("iTunesMetadata.plist")
            .unwrap()
            .read_to_end(&mut metadata)
            .unwrap();
        assert!(String::from_utf8_lossy(&metadata).contains("tester@example.com"));

        let mut sinf = Vec::new();
        zip.by_name("Payload/Test.app/SC_Info/TestExec.sinf")
            .unwrap()
            .read_to_end(&mut sinf)
            .unwrap();
        assert_eq!(sinf, b"main-sinf");
    }
}
