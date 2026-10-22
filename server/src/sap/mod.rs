//! SAP request signing for App Store authentication. See README.md for provenance.
mod cipher;
mod crypto;
mod f2;
mod handshake;
mod random;
mod source;
mod tables;

pub use handshake::SapSigner;

use handshake::Handshake;
use rand::Rng;
use std::time::{SystemTime, UNIX_EPOCH};

const SETUP_CERT_URL: &str = "https://s.mzstatic.com/sap/setupCert.plist";
const SETUP_URL: &str = "https://fpinit.itunes.apple.com/v1/signSapSetup/legacy";
const MAX_SETUP_BODY: usize = 2 * 1024 * 1024;

fn plist_data(body: &[u8], key: &str) -> Result<Vec<u8>, String> {
    let value: plist::Value = plist::from_bytes(body)
        .map_err(|e| format!("Invalid SAP setup service response: {}", e))?;
    match value.as_dictionary().and_then(|dict| dict.get(key)) {
        Some(plist::Value::Data(data)) if !data.is_empty() && data.len() <= 1_048_576 => {
            Ok(data.clone())
        }
        _ => Err("Invalid SAP setup service response".to_string()),
    }
}

async fn exchange(
    client: &reqwest::Client,
    user_agent: &str,
    url: &str,
    key: &str,
    payload: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let request = match payload {
        Some(payload) => {
            let mut dict = plist::Dictionary::new();
            dict.insert(
                "sign-sap-setup-buffer".to_string(),
                plist::Value::Data(payload.to_vec()),
            );
            let mut body = Vec::new();
            plist::to_writer_xml(&mut body, &plist::Value::Dictionary(dict))
                .map_err(|e| e.to_string())?;
            client.post(url).body(body)
        }
        None => client.get(url),
    };
    let response = request
        .header("User-Agent", user_agent)
        .header("Content-Type", "application/x-apple-plist")
        .send()
        .await
        .map_err(|e| format!("SAP setup request failed: {}", e))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|e| format!("SAP setup read failed: {}", e))?;
    if status != reqwest::StatusCode::OK || body.len() > MAX_SETUP_BODY {
        return Err(format!("SAP setup failed (HTTP {})", status.as_u16()));
    }
    plist_data(&body, key)
}

/// Run a fresh SAP setup exchange with Apple and return a signer.
/// Keep the signer for the whole login, including the verification-code step.
pub async fn connect(client: &reqwest::Client, user_agent: &str) -> Result<SapSigner, String> {
    let certificate = exchange(
        client,
        user_agent,
        SETUP_CERT_URL,
        "sign-sap-setup-cert",
        None,
    )
    .await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let entropy: [u32; 3] = rand::thread_rng().gen();
    let handshake = Handshake::new(
        &certificate,
        entropy,
        now.as_secs() as i64,
        now.subsec_micros() as i64,
    )?;
    let reply = exchange(
        client,
        user_agent,
        SETUP_URL,
        "sign-sap-setup-buffer",
        Some(&handshake.request),
    )
    .await?;
    SapSigner::new(handshake.complete(&reply)?, 1)
}

#[cfg(test)]
mod tests {
    use super::cipher::SapCipher;
    use super::f2::derive_f2;
    use super::handshake::{Handshake, SapSession, SapSigner};
    use super::random::SapRandom;
    use super::tables::tables;
    use serde::Deserialize;
    use sha2::Digest;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Case {
        entropy: [u64; 3],
        seconds: i64,
        microseconds: i64,
        request: String,
        reply: String,
        random_hash: String,
        cursor: usize,
        seed30: u32,
        seed54: u32,
        f2_input2: String,
        f2_tail: String,
    }

    #[derive(Deserialize)]
    struct Fixture {
        certificate: String,
        cases: Vec<Case>,
    }

    fn fixture() -> Fixture {
        serde_json::from_str(include_str!("testdata/sap-handshake.json")).unwrap()
    }

    fn unhex(value: &str) -> Vec<u8> {
        hex::decode(value).unwrap()
    }

    fn handshake(fixture: &Fixture, case: &Case) -> Handshake {
        Handshake::new(
            &unhex(&fixture.certificate),
            case.entropy.map(|value| value as u32),
            case.seconds,
            case.microseconds,
        )
        .unwrap()
    }

    #[test]
    fn matches_independent_handshake_vectors() {
        let fixture = fixture();
        for case in &fixture.cases {
            let handshake = handshake(&fixture, case);
            assert_eq!(hex::encode(&handshake.request), case.request);
            let session = handshake.complete(&unhex(&case.reply)).unwrap();
            let words: Vec<u8> = session
                .random
                .words
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect();
            assert_eq!(hex::encode(sha2::Sha256::digest(&words)), case.random_hash);
            assert_eq!(session.random.cursor, case.cursor);
            assert_eq!(session.seed30, case.seed30);
            assert_eq!(session.seed54, case.seed54);
            assert_eq!(hex::encode(&session.f2_input2), case.f2_input2);
            assert_eq!(hex::encode(&session.f2_tail), case.f2_tail);
            let expected: Vec<u8> = (0..576u32).map(|index| (index * 17 + 3) as u8).collect();
            assert_eq!(session.source, expected);
            assert_eq!(session.f3.len(), 159);
        }
    }

    #[test]
    fn rejects_truncated_and_unrelated_setup_replies() {
        let fixture = fixture();
        let reply = unhex(&fixture.cases[0].reply);
        for count in [0, 20, 21, reply.len() - 1] {
            let handshake = handshake(&fixture, &fixture.cases[0]);
            assert!(handshake.complete(&reply[..count]).is_err());
        }
        let handshake = handshake(&fixture, &fixture.cases[0]);
        assert!(handshake.complete(&unhex(&fixture.cases[1].reply)).is_err());
    }

    #[test]
    fn matches_f2_block_encryption_and_consecutive_signatures() {
        let tables = tables().unwrap();
        fn word(state: &mut u32) -> u32 {
            *state ^= *state << 13;
            *state ^= *state >> 17;
            *state ^= *state << 5;
            *state
        }
        fn bytes(state: &mut u32, length: usize) -> Vec<u8> {
            (0..length).map(|_| word(state) as u8).collect()
        }
        let mut state: u32 = 0x5359_5031;
        let mut source = bytes(&mut state, 576);
        source[0xfa] = source[0xca] ^ 10;
        source[0xf3] = source[0xc3] ^ 147;
        let f2_input1 = bytes(&mut state, 16);
        let f2_input2 = bytes(&mut state, 16);
        let f2_tail = bytes(&mut state, 7);
        let label = b"SYNTHETIC TEST SESSION / NEVER SERVER-ISSUED / ";
        let mut f3 = label.to_vec();
        f3.extend(bytes(&mut state, 159 - label.len()));
        let mut random = SapRandom::new();
        for value in random.words.iter_mut() {
            *value = word(&mut state);
        }
        random.cursor = 624;
        let seed30 = word(&mut state);
        let seed54 = word(&mut state);
        let session = SapSession {
            source: source.clone(),
            f2_input1: f2_input1.clone(),
            f2_input2: f2_input2.clone(),
            f2_tail,
            f3,
            random,
            seed30,
            seed54,
            f1_address: 0x3450_0044,
            iv_address: 0xeed0_0050,
        };

        assert_eq!(
            hex::encode(derive_f2(&f2_input1, &f2_input2, tables).unwrap()),
            "ad1adda4bad5ada37fd54543681d001c"
        );
        let cipher = SapCipher::new(&source, tables).unwrap();
        let blocks: [Vec<u8>; 3] = [vec![0; 16], (0..16).collect(), vec![255; 16]];
        let expected_blocks = [
            "8fcdc9e6d7c5009c7f489288231582de",
            "65810b6c7555809e4b87529110631a4c",
            "73d2bd4f671e9abe8b105aac1883036d",
        ];
        for (block, expected) in blocks.iter().zip(expected_blocks) {
            assert_eq!(hex::encode(cipher.encrypt(block).unwrap()), expected);
        }

        let mut signer = SapSigner::new(session, 7).unwrap();
        let mut body2 = b"synthetic SAP\nbody-2".to_vec();
        body2.extend([0, 255]);
        let bodies: [Vec<u8>; 3] = [vec![], body2, (0..=255).collect()];
        let expected = [
            "ba76e674c73d9db75c2986233016b94b22e062556fd029d71733726519756517",
            "4acc6269de932c753260fecfa375a6405b5b63cf621a8adace3bd7b5f0305287",
            "853914149c9de004852fffa75a4489c3cdec3f179c92903e3d94b8b2d494e4e4",
        ];
        for (body, expected) in bodies.iter().zip(expected) {
            let signature = signer.sign(body).unwrap();
            assert_eq!(signature.len(), 501);
            assert_eq!(hex::encode(sha2::Sha256::digest(&signature)), expected);
        }
    }
}
