// Certificate parsing and primitives for the SAP setup exchange (port of AssppWeb crypto.ts).
use openssl::bn::{BigNum, BigNumContext};
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use openssl::symm::{Cipher, Crypter, Mode};

pub fn be32(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

pub fn read32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset.checked_add(4).ok_or("Truncated SAP record")?)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "Truncated SAP record".to_string())
}

pub fn sha1(bytes: &[u8]) -> [u8; 20] {
    openssl::sha::sha1(bytes)
}

pub fn xor(left: &[u8], right: &[u8]) -> Vec<u8> {
    left.iter().zip(right).map(|(a, b)| a ^ b).collect()
}

#[derive(Clone, Copy)]
struct DerNode {
    tag: u8,
    start: usize,
    end: usize,
}

fn der(bytes: &[u8], mut at: usize, limit: usize) -> Option<DerNode> {
    let tag = *bytes.get(at)?;
    let mut length = *bytes.get(at + 1)? as usize;
    at += 2;
    if length & 0x80 != 0 {
        let count = length & 0x7f;
        if !(1..=4).contains(&count) || at + count > limit {
            return None;
        }
        length = 0;
        for _ in 0..count {
            length = length * 256 + *bytes.get(at)? as usize;
            at += 1;
        }
    }
    if at > limit || at + length > limit {
        return None;
    }
    Some(DerNode {
        tag,
        start: at,
        end: at + length,
    })
}

fn children(bytes: &[u8], node: DerNode) -> Option<Vec<DerNode>> {
    let mut result = Vec::new();
    let mut at = node.start;
    while at < node.end {
        let child = der(bytes, at, node.end)?;
        at = child.end;
        result.push(child);
    }
    Some(result)
}

fn integer(bytes: &[u8], node: DerNode) -> Option<&[u8]> {
    if node.tag != 2 || node.start == node.end || bytes[node.start] & 0x80 != 0 {
        return None;
    }
    let mut start = node.start;
    while start + 1 < node.end && bytes[start] == 0 {
        start += 1;
    }
    Some(&bytes[start..node.end])
}

pub struct PublicKey {
    pub serial: Vec<u8>,
    pub modulus: Vec<u8>,
    pub exponent: Vec<u8>,
}

fn parse_certificate(bytes: &[u8], at: usize, selected_end: usize) -> Option<(PublicKey, usize)> {
    let root = der(bytes, at, bytes.len())?;
    if root.end <= selected_end || root.end - at < 512 {
        return None;
    }
    let certificate = children(bytes, root)?;
    if certificate.len() != 3
        || certificate[0].tag != 0x30
        || certificate[1].tag != 0x30
        || certificate[2].tag != 3
    {
        return None;
    }
    let fields = children(bytes, certificate[0])?;
    let version = if fields.first()?.tag == 0xa0 { 1 } else { 0 };
    let serial = integer(bytes, *fields.get(version)?)?;
    let public_key = children(bytes, *fields.get(version + 5)?)?;
    let algorithm = children(bytes, *public_key.first()?)?;
    let oid_node = *algorithm.first()?;
    let bit_string = *public_key.get(1)?;
    if oid_node.tag != 6
        || bytes[oid_node.start..oid_node.end] != [42, 134, 72, 134, 247, 13, 1, 1, 1]
        || bit_string.tag != 3
        || *bytes.get(bit_string.start)? != 0
    {
        return None;
    }
    let key = children(bytes, der(bytes, bit_string.start + 1, bit_string.end)?)?;
    if key.len() != 2 {
        return None;
    }
    let modulus = integer(bytes, key[0])?;
    let exponent = integer(bytes, key[1])?;
    let exponent_small = exponent.len() == 1 && exponent[0] < 3;
    if serial.len() > 127
        || modulus.len() != 256
        || modulus[0] < 128
        || exponent_small
        || exponent[exponent.len() - 1] & 1 == 0
    {
        return None;
    }
    let wire_serial = if serial[0] & 0x80 != 0 {
        [&[0u8][..], serial].concat()
    } else {
        serial.to_vec()
    };
    if wire_serial.len() > 127 {
        return None;
    }
    Some((
        PublicKey {
            serial: wire_serial,
            modulus: modulus.to_vec(),
            exponent: exponent.to_vec(),
        },
        root.end,
    ))
}

/// Find the largest RSA-2048 certificate in Apple's setup container.
pub fn read_certificate(bytes: &[u8]) -> Result<PublicKey, String> {
    if bytes.len() < 512 || bytes.len() > 1_048_576 {
        return Err("Invalid SAP certificate container".to_string());
    }
    let mut selected = None;
    let mut selected_end = 0;
    for at in 0..bytes.len() - 4 {
        if bytes[at] != 0x30 {
            continue;
        }
        // The service container also contains non-certificate DER sequences.
        if let Some((key, end)) = parse_certificate(bytes, at, selected_end) {
            selected = Some(key);
            selected_end = end;
        }
    }
    selected.ok_or_else(|| "SAP container has no RSA-2048 certificate".to_string())
}

/// RSA-OAEP (SHA-1) with an explicit seed, as SAP requires deterministic output.
pub fn rsa_oaep(
    message: &[u8],
    seed: &[u8],
    modulus: &[u8],
    exponent: &[u8],
) -> Result<Vec<u8>, String> {
    if message.len() > 214 || seed.len() != 20 || modulus.len() != 256 {
        return Err("Invalid SAP RSA inputs".to_string());
    }
    let mask = |input: &[u8], count: usize| -> Vec<u8> {
        let mut output = Vec::new();
        for index in 0..count.div_ceil(20) as u32 {
            output.extend_from_slice(&sha1(&[input, &be32(index)].concat()));
        }
        output.truncate(count);
        output
    };
    let mut database = sha1(&[]).to_vec();
    database.resize(database.len() + 214 - message.len(), 0);
    database.push(1);
    database.extend_from_slice(message);
    let masked_database = xor(&database, &mask(seed, 235));
    let masked_seed = xor(seed, &mask(&masked_database, 20));
    let encoded = [&[0u8][..], &masked_seed, &masked_database].concat();

    let bn = |bytes: &[u8]| BigNum::from_slice(bytes).map_err(|e| e.to_string());
    let n = bn(modulus)?;
    let base = bn(&encoded)?;
    let power = bn(exponent)?;
    let three = BigNum::from_u32(3).map_err(|e| e.to_string())?;
    if base >= n || power < three || !power.is_bit_set(0) {
        return Err("Invalid SAP RSA key".to_string());
    }
    let mut context = BigNumContext::new().map_err(|e| e.to_string())?;
    let mut result = BigNum::new().map_err(|e| e.to_string())?;
    result
        .mod_exp(&base, &power, &n, &mut context)
        .map_err(|e| e.to_string())?;
    result.to_vec_padded(256).map_err(|e| e.to_string())
}

/// AES-128-CBC over whole blocks, without padding.
#[allow(clippy::manual_is_multiple_of)]
pub fn cbc(bytes: &[u8], key: &[u8], iv: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    if key.len() != 16
        || iv.len() != 16
        || bytes.is_empty()
        || bytes.len() % 16 != 0
        || bytes.len() > 1_048_576
    {
        return Err("Invalid SAP AES blocks".to_string());
    }
    let mode = if encrypt {
        Mode::Encrypt
    } else {
        Mode::Decrypt
    };
    let mut crypter =
        Crypter::new(Cipher::aes_128_cbc(), mode, key, Some(iv)).map_err(|e| e.to_string())?;
    crypter.pad(false);
    let mut output = vec![0u8; bytes.len() + 16];
    let mut count = crypter
        .update(bytes, &mut output)
        .map_err(|e| e.to_string())?;
    count += crypter
        .finalize(&mut output[count..])
        .map_err(|e| e.to_string())?;
    output.truncate(count);
    Ok(output)
}

pub fn hmac_sha1(key: &[u8], message: &[u8]) -> Result<Vec<u8>, String> {
    let key = PKey::hmac(key).map_err(|e| e.to_string())?;
    let mut signer = Signer::new(MessageDigest::sha1(), &key).map_err(|e| e.to_string())?;
    signer.update(message).map_err(|e| e.to_string())?;
    signer.sign_to_vec().map_err(|e| e.to_string())
}
