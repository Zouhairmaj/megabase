//! Erlang-side values the channel needs from JSON: payload sizes, presence
//! refs and default presence keys.

// Ported from supabase/realtime lib/realtime/tenants.ex `validate_payload_size/2` and
// lib/realtime_web/channels/payloads/join.ex `presence_key/1` (Apache-2.0), pin v2.143.3.
// `phx_ref` follows Phoenix.Tracker 1.7 `random_ref/0` and the default key
// follows elixir_uuid `UUID.uuid1/0`; neither is vendored.

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::Value;
use uuid::Uuid;

/// `Tenants.validate_payload_size/2` for the self-hosted tenant:
/// `max_payload_size_in_kb` 3000 plus 500 bytes of padding.
pub(crate) const MAX_PAYLOAD_SIZE: usize = 3000 * 1000 + 500;

/// `:erlang.external_size/1` of `value` as Jason decodes it: objects are maps
/// with binary keys, strings binaries, `null`/`true`/`false` atoms.
///
/// Integers past `u64` decode as floats here and as bignums in Elixir, so
/// their size differs by a few bytes.
pub(crate) fn external_size(value: &Value) -> usize {
    // The version byte.
    1 + term_size(value)
}

fn term_size(value: &Value) -> usize {
    match value {
        // SMALL_ATOM_UTF8_EXT: tag, length, name.
        Value::Null => 2 + "nil".len(),
        Value::Bool(true) => 2 + "true".len(),
        Value::Bool(false) => 2 + "false".len(),
        Value::Number(number) => number
            .as_i64()
            .map(i128::from)
            .or_else(|| number.as_u64().map(i128::from))
            .map_or(9, integer_size),
        // BINARY_EXT: tag, 4-byte length, bytes.
        Value::String(text) => 5 + text.len(),
        // NIL_EXT, or LIST_EXT (tag, 4-byte length, elements, NIL_EXT tail).
        Value::Array(items) if items.is_empty() => 1,
        Value::Array(items) => 6 + items.iter().map(term_size).sum::<usize>(),
        // MAP_EXT: tag, 4-byte arity, binary keys and values.
        Value::Object(map) => {
            5 + map
                .iter()
                .map(|(key, value)| 5 + key.len() + term_size(value))
                .sum::<usize>()
        }
    }
}

fn integer_size(value: i128) -> usize {
    if (0..=255).contains(&value) {
        // SMALL_INTEGER_EXT
        2
    } else if i32::try_from(value).is_ok() {
        // INTEGER_EXT
        5
    } else {
        // SMALL_BIG_EXT: tag, digit count, sign, little-endian digits.
        let magnitude = value.unsigned_abs();
        let digits = (128 - magnitude.leading_zeros()).div_ceil(8);
        3 + digits as usize
    }
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

/// Phoenix.Tracker `random_ref/0`: base64 of the 64-bit system time in
/// nanoseconds, a 16-bit node/process hash and a 16-bit unique integer.
pub(crate) fn phx_ref() -> String {
    static UNIQUE: AtomicU16 = AtomicU16::new(0);
    let nanos = u64::try_from(unix_nanos()).unwrap_or(u64::MAX);
    let random = Uuid::new_v4();
    let hash = &random.as_bytes()[..2];
    let unique = UNIQUE.fetch_add(1, Ordering::Relaxed);
    let mut bytes = Vec::with_capacity(12);
    bytes.extend_from_slice(&nanos.to_be_bytes());
    bytes.extend_from_slice(hash);
    bytes.extend_from_slice(&unique.to_be_bytes());
    STANDARD.encode(bytes)
}

/// `UUID.uuid1/0`: a version 1 UUID with a random clock sequence and a
/// random multicast node id (no MAC address is exposed).
pub(crate) fn uuid1() -> String {
    // 100 ns intervals between 1582-10-15 and 1970-01-01.
    const GREGORIAN_OFFSET: u128 = 0x01B2_1DD2_1381_4000;
    let ticks = unix_nanos() / 100 + GREGORIAN_OFFSET;
    // The timestamp is 60 bits wide.
    let ticks = u64::try_from(ticks & 0x0FFF_FFFF_FFFF_FFFF).unwrap_or(0);
    let random = *Uuid::new_v4().as_bytes();
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&((ticks & 0xFFFF_FFFF) as u32).to_be_bytes());
    bytes[4..6].copy_from_slice(&(((ticks >> 32) & 0xFFFF) as u16).to_be_bytes());
    bytes[6..8].copy_from_slice(&((((ticks >> 48) & 0x0FFF) as u16) | 0x1000).to_be_bytes());
    // RFC 4122 variant and a 14-bit clock sequence.
    bytes[8] = (random[0] & 0x3F) | 0x80;
    bytes[9] = random[1];
    bytes[10..].copy_from_slice(&random[2..8]);
    bytes[10] |= 0x01;
    Uuid::from_bytes(bytes).hyphenated().to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn external_size_matches_term_to_binary() {
        // Worked from the external term format tags (OTP 26 defaults:
        // UTF-8 atoms, NEW_FLOAT_EXT); not measured on a BEAM.
        let cases = [
            (json!({}), 6),
            (json!([]), 2),
            (json!(null), 6),
            (json!(true), 7),
            (json!(false), 8),
            (json!(7), 3),
            (json!(256), 6),
            (json!(-1), 6),
            (json!(1_i64 << 40), 10),
            (json!(u64::MAX), 12),
            (json!(1.5), 10),
            (json!("abc"), 9),
            (json!([1, 2]), 1 + 6 + 2 + 2),
            (json!({"a": 1}), 1 + 5 + 6 + 2),
        ];
        for (value, size) in cases {
            assert_eq!(external_size(&value), size, "{value}");
        }
    }

    #[test]
    fn phx_ref_is_twelve_bytes_of_base64() {
        let reference = phx_ref();
        assert_eq!(reference.len(), 16);
        assert_eq!(STANDARD.decode(&reference).unwrap().len(), 12);
        assert_ne!(reference, phx_ref());
    }

    #[test]
    fn uuid1_is_version_1() {
        let key = Uuid::parse_str(&uuid1()).unwrap();
        assert_eq!(key.get_version_num(), 1);
        assert_eq!(key.get_variant(), uuid::Variant::RFC4122);
    }
}
