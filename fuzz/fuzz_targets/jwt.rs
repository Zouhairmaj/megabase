#![no_main]

use libfuzzer_sys::fuzz_target;
use megabase_core::{bearer_token, Hs256};

/// Dedicated fuzz key. Not a production secret and not the demo `JWT_SECRET`.
const SECRET: &[u8] = b"fuzz-secret-not-a-production-key-32b!";
/// After the demo `iat`, before the demo `exp`.
const NOW: i64 = 1_791_504_000;

fuzz_target!(|data: &[u8]| {
    let Ok(token) = std::str::from_utf8(data) else {
        return;
    };
    let verifier = Hs256::new(SECRET).expect("non-empty fuzz secret");
    let _ = verifier.verify_at(token, NOW);
    let _ = bearer_token(token);
});
