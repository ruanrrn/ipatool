// SAP setup exchange and request signer (port of AssppWeb handshake.ts / signer.ts).
use super::cipher::SapCipher;
use super::crypto::{be32, cbc, hmac_sha1, read32, read_certificate, rsa_oaep, sha1, xor};
use super::f2::derive_f2;
use super::random::SapRandom;
use super::tables::{tables, Tables};

pub struct SapSession {
    pub source: Vec<u8>,
    pub f2_input1: Vec<u8>,
    pub f2_input2: Vec<u8>,
    pub f2_tail: Vec<u8>,
    pub f3: Vec<u8>,
    pub random: SapRandom,
    pub seed30: u32,
    pub seed54: u32,
    pub f1_address: u32,
    pub iv_address: u32,
}

pub struct Handshake {
    pub request: Vec<u8>,
    random: SapRandom,
    response_key: Vec<u8>,
    response_echo: Vec<u8>,
    material: Vec<u8>,
    seed30: u32,
    seed54: u32,
}

fn generate(
    random: &mut SapRandom,
    pool: &[u32; 128],
    tables: &Tables,
    count: usize,
    address: u32,
    exchange: bool,
) -> Vec<u8> {
    if exchange {
        random.fill(pool[15], address, &tables.exchange_whiten, 0x4b, count)
    } else {
        random.fill(pool[21], address, &tables.iv_whiten, 0x12, count)
    }
}

impl Handshake {
    pub fn new(
        certificate: &[u8],
        entropy: [u32; 3],
        seconds: i64,
        microseconds: i64,
    ) -> Result<Self, String> {
        let tables = tables()?;
        let public_key = read_certificate(certificate)?;
        let mut random = SapRandom::new();
        let pool = random.initialize(entropy, seconds, microseconds, tables)?;
        let seed30 = pool[12];
        let seed54 = pool[21];
        let mut gen = |count: usize, address: u32, exchange: bool| {
            generate(&mut random, &pool, tables, count, address, exchange)
        };
        let initial = gen(33, 0x7f_fac0, false);
        let material: Vec<u8> = initial[..16 + (initial[32] % 17) as usize]
            .iter()
            .map(|value| value ^ 0x2d)
            .collect();
        let response_key = gen(16, 0x7f_fc14, true);
        let response_echo = gen(16, 0x7f_fc14, true);
        let encryption_key = gen(16, 0x7f_fc14, true);
        let iv = gen(16, 0x7f_fc14, true);
        let authentication_key = gen(20, 0x7f_fc24, true);
        let padding: Vec<u8> = (0..11u32)
            .map(|index| {
                let value = 0x7f_f812 - index;
                ((value ^ 0xba).wrapping_mul(value + 0x11) ^ 0x2d) as u8
            })
            .collect();
        let plaintext = [
            &[2u8][..],
            &be32(1),
            &response_key,
            &response_echo,
            &padding,
        ]
        .concat();
        let oaep_seed = gen(20, 0x7f_f4e4, false);
        let key_block = rsa_oaep(
            &[&encryption_key[..], &authentication_key].concat(),
            &oaep_seed,
            &public_key.modulus,
            &public_key.exponent,
        )?;
        let encrypted = cbc(&plaintext, &encryption_key, &iv, true)?;
        let request = [
            &[1u8, public_key.serial.len() as u8][..],
            &public_key.serial,
            &key_block,
            &iv,
            &be32(encrypted.len() as u32),
            &encrypted,
            &hmac_sha1(&authentication_key, &encrypted)?,
        ]
        .concat();

        Ok(Handshake {
            request,
            random,
            response_key,
            response_echo,
            material,
            seed30,
            seed54,
        })
    }

    pub fn complete(self, reply: &[u8]) -> Result<SapSession, String> {
        if reply.len() < 21 || reply.len() > 1_048_576 || reply[0] != 2 {
            return Err("Invalid SAP setup response".to_string());
        }
        let length = read32(reply, 17)? as i64;
        if length == 0 || length % 16 != 0 || length > reply.len() as i64 - 21 - 25 {
            return Err("Invalid SAP setup ciphertext length".to_string());
        }
        let record_offset = 21 + length as usize;
        let record_length = read32(reply, record_offset + 21)?.wrapping_add(25) as usize;
        if record_length > 16 * 1024 * 1024 || record_length > reply.len() - record_offset {
            return Err("Invalid SAP signature record".to_string());
        }
        let plaintext = cbc(
            &reply[21..record_offset],
            &self.response_key,
            &reply[1..17],
            false,
        )?;
        let mut offset = 0;
        let mut fields = Vec::new();
        for expected in [16usize, 16, 16, 576, 576, 16] {
            let count = read32(&plaintext, offset)? as usize;
            offset += 4;
            if count != expected || count > plaintext.len() - offset {
                return Err("Invalid decrypted SAP field".to_string());
            }
            fields.push(plaintext[offset..offset + count].to_vec());
            offset += count;
        }
        let difference = fields[5]
            .iter()
            .zip(&self.response_echo)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        if difference != 0 {
            return Err("SAP reply belongs to another exchange".to_string());
        }
        Ok(SapSession {
            source: fields[4].clone(),
            f2_input1: fields[2]
                .iter()
                .map(|value| value.wrapping_add(0x2d))
                .collect(),
            f2_input2: self.material[..16].to_vec(),
            f2_tail: self.material[16..]
                .iter()
                .map(|value| value ^ 0x2d)
                .collect(),
            f3: reply[record_offset..record_offset + record_length].to_vec(),
            random: self.random,
            seed30: self.seed30,
            seed54: self.seed54,
            f1_address: 0x604 + if self.material.len() > 28 { 16 } else { 0 },
            iv_address: 0x7f_ed90,
        })
    }
}

/// Signs App Store requests with `X-Apple-ActionSignature`.
pub struct SapSigner {
    session: SapSession,
    cipher: SapCipher,
    f2: Vec<u8>,
    platform: u32,
    blob: Option<Vec<u8>>,
}

impl std::fmt::Debug for SapSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SapSigner")
    }
}

impl SapSigner {
    pub fn new(session: SapSession, platform: u32) -> Result<Self, String> {
        let tables = tables()?;
        let cipher = SapCipher::new(&session.source, tables)?;
        let mut f2 = derive_f2(&session.f2_input1, &session.f2_input2, tables)?.to_vec();
        f2.extend_from_slice(&session.f2_tail);
        Ok(SapSigner {
            session,
            cipher,
            f2,
            platform,
            blob: None,
        })
    }

    pub fn sign(&mut self, body: &[u8]) -> Result<Vec<u8>, String> {
        let tables = tables()?;
        if self.blob.is_none() {
            let session = &mut self.session;
            let f1 = session
                .random
                .f1(session.seed30, session.f1_address, tables);
            let mut blob = [
                &[3u8][..],
                &be32(self.platform),
                &be32(256),
                &f1,
                &be32(self.f2.len() as u32),
                &self.f2,
                &be32(session.f3.len() as u32),
                &session.f3,
            ]
            .concat();
            blob.resize(blob.len() + (16 - blob.len() % 16) % 16, 0);
            self.blob = Some(blob);
        }
        let blob = self.blob.as_ref().unwrap();
        let iv = self.session.random.fill(
            self.session.seed54,
            self.session.iv_address,
            &tables.iv_whiten,
            0x12,
            16,
        );
        let hash = sha1(&[&blob[..], body].concat());
        let first = self.cipher.encrypt(&xor(&iv, &hash[..16]))?;
        let mut tail = hash[16..].to_vec();
        tail.resize(16, 0);
        let tag = self.cipher.encrypt(&xor(&first, &tail))?;
        Ok([&[2u8][..], &iv, &tag, &be32(blob.len() as u32), blob].concat())
    }
}
