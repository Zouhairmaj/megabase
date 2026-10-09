#![no_main]

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use libfuzzer_sys::fuzz_target;
use megabase_core::{bearer_token, Hs256};
use sha2::Sha256;

/// Dedicated fuzz key. Not a production secret and not the demo `JWT_SECRET`.
const SECRET: &[u8] = b"fuzz-secret-not-a-production-key-32b!";
/// After the demo `iat`, before the demo `exp`.
const NOW: i64 = 1_791_504_000;

fn sign(secret: &[u8], header: &[u8], payload: &[u8]) -> String {
    let header_b64 = URL_SAFE_NO_PAD.encode(header);
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
    let signing_input = format!("{header_b64}.{payload_b64}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("hmac key");
    mac.update(signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{signing_input}.{sig}")
}

fuzz_target!(|data: &[u8]| {
    let verifier = Hs256::new(SECRET).expect("non-empty fuzz secret");
    if let Ok(token) = std::str::from_utf8(data) {
        let _ = verifier.verify_at(token, NOW);
        let _ = bearer_token(token);
    }
    // HMAC is checked before claims. Sign fuzz-controlled bytes so mutations
    // can reach header/payload parsing instead of stopping at BadCrypto.
    let mid = data.len() / 2;
    let signed = sign(SECRET, &data[..mid], &data[mid..]);
    let _ = verifier.verify_at(&signed, NOW);
});
