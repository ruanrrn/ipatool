use base64::Engine as _;
use serde::Deserialize;
use sha2::Digest;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Resource {
    size: usize,
    sha256: String,
    base64: String,
}

#[derive(Deserialize)]
pub struct Expansion {
    pub selection: Vec<usize>,
    pub whitening: u32,
}

#[derive(Deserialize)]
pub struct KeySelection {
    pub shift: u32,
    pub words: Vec<usize>,
    pub whitening: u8,
    pub linear: i32,
}

#[derive(Deserialize)]
pub struct Routing {
    pub table: usize,
    pub destinations: Vec<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct RawTables {
    #[serde(rename = "byte-tables")]
    byte_tables: Resource,
    #[serde(rename = "word-tables")]
    word_tables: Resource,
    #[serde(rename = "carry-masks")]
    carry_masks: Resource,
    #[serde(rename = "f2-tables")]
    f2_tables: Resource,
    iv_whiten: Vec<u8>,
    f1_whiten: Vec<u8>,
    f1_rc: Vec<u8>,
    f1_template: Vec<u8>,
    f1_g: Vec<Vec<u8>>,
    initialize_whiten: Vec<u8>,
    exchange_whiten: Vec<u8>,
    #[serde(rename = "entryPositions")]
    entry_positions: Vec<usize>,
    #[serde(rename = "finalPositions")]
    final_positions: Vec<usize>,
    #[serde(rename = "expansion")]
    expansion: Vec<Expansion>,
    #[serde(rename = "keySelections")]
    key_selections: Vec<Vec<KeySelection>>,
    #[serde(rename = "mainRouting")]
    main_routing: Vec<Vec<Routing>>,
    #[serde(rename = "padRows")]
    pad_rows: Vec<Vec<u8>>,
    #[serde(rename = "padColumns")]
    pad_columns: Vec<Vec<u8>>,
}

/// Fixed lookup tables and permutations of the SAP algorithm.
pub struct Tables {
    pub bytes: Vec<Vec<u8>>,
    pub words: Vec<Vec<u32>>,
    pub masks: Vec<Vec<u8>>,
    pub f2: Vec<u8>,
    pub iv_whiten: Vec<u8>,
    pub f1_whiten: Vec<u8>,
    pub f1_rc: Vec<u8>,
    pub f1_template: Vec<u8>,
    pub f1_g: Vec<Vec<u8>>,
    pub initialize_whiten: Vec<u8>,
    pub exchange_whiten: Vec<u8>,
    pub entry_positions: Vec<usize>,
    pub final_positions: Vec<usize>,
    pub expansion: Vec<Expansion>,
    pub key_selections: Vec<Vec<KeySelection>>,
    pub main_routing: Vec<Vec<Routing>>,
    pub pad_rows: Vec<Vec<u8>>,
    pub pad_columns: Vec<Vec<u8>>,
}

fn decode(resource: &Resource) -> Result<Vec<u8>, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&resource.base64)
        .map_err(|e| format!("invalid SAP table encoding: {}", e))?;
    let hash = hex::encode(sha2::Sha256::digest(&bytes));
    if bytes.len() != resource.size || hash != resource.sha256 {
        return Err("Invalid SAP algorithm tables".to_string());
    }
    Ok(bytes)
}

fn load() -> Result<Tables, String> {
    let raw: RawTables = serde_json::from_str(include_str!("tables.json"))
        .map_err(|e| format!("invalid SAP tables: {}", e))?;
    let bytes = decode(&raw.byte_tables)?;
    let words = decode(&raw.word_tables)?;
    let masks = decode(&raw.carry_masks)?;
    let f2 = decode(&raw.f2_tables)?;

    let mut offset = 0;
    let masks = [233, 239, 241, 251]
        .iter()
        .map(|&length| {
            let part = masks[offset..offset + length].to_vec();
            offset += length;
            part
        })
        .collect();

    Ok(Tables {
        bytes: bytes.chunks(256).take(200).map(<[u8]>::to_vec).collect(),
        words: words
            .chunks(1024)
            .take(72)
            .map(|table| {
                table
                    .chunks(4)
                    .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
                    .collect()
            })
            .collect(),
        masks,
        f2,
        iv_whiten: raw.iv_whiten,
        f1_whiten: raw.f1_whiten,
        f1_rc: raw.f1_rc,
        f1_template: raw.f1_template,
        f1_g: raw.f1_g,
        initialize_whiten: raw.initialize_whiten,
        exchange_whiten: raw.exchange_whiten,
        entry_positions: raw.entry_positions,
        final_positions: raw.final_positions,
        expansion: raw.expansion,
        key_selections: raw.key_selections,
        main_routing: raw.main_routing,
        pad_rows: raw.pad_rows,
        pad_columns: raw.pad_columns,
    })
}

pub fn tables() -> Result<&'static Tables, String> {
    static TABLES: OnceLock<Result<Tables, String>> = OnceLock::new();
    TABLES.get_or_init(load).as_ref().map_err(Clone::clone)
}
