// Ported from supabase/auth internal/crypto/recovery_codes.go (MIT),
// pin v2.197.0.

//! Recovery-code generation and argon2id verification.
//!
//! New hashes use m=19456, t=2, p=1, a 16-byte salt, and a 32-byte digest.
//! Verification reads the parameters from the stored PHC string.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
const MEMORY_KIB: u32 = 19_456;
const ITERATIONS: u32 = 2;
const PARALLELISM: u32 = 1;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;

/// `GenerateRecoveryCode`: `length` characters from the lowercase base32 alphabet.
///
/// # Errors
/// Returns [`HashError::Random`] when `/dev/urandom` cannot be read.
pub(crate) fn generate_code(length: usize) -> Result<String, HashError> {
    let mut raw = vec![0u8; length];
    fill_random(&mut raw)?;
    Ok(raw
        .into_iter()
        .map(|byte| char::from(ALPHABET[(byte as usize) % ALPHABET.len()]))
        .collect())
}

/// `NormalizeRecoveryCode`: drop hyphens and white space, then lowercase.
pub(crate) fn normalize(input: &str) -> String {
    input
        .chars()
        .filter(|ch| *ch != '-' && !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `GenerateRecoveryCodeHash`.
///
/// # Errors
/// Returns [`HashError`] when the salt cannot be read or argon2id rejects
/// the parameters.
pub(crate) fn hash_code(code: &str) -> Result<String, HashError> {
    let mut salt = [0u8; SALT_LEN];
    fill_random(&mut salt)?;
    let digest = argon2id(
        code.as_bytes(),
        &salt,
        ITERATIONS,
        MEMORY_KIB,
        PARALLELISM,
        KEY_LEN,
    )?;
    Ok(format!(
        "$argon2id$v=19$m={MEMORY_KIB},t={ITERATIONS},p={PARALLELISM}${}${}",
        STANDARD_NO_PAD.encode(salt),
        STANDARD_NO_PAD.encode(digest),
    ))
}

/// `CompareHashAndRecoveryCode`.
///
/// `Ok(true)` is a match. `Ok(false)` is a well-formed hash that does not
/// match. `Err` is a malformed hash, which GoTrue treats as a non-match.
pub(crate) fn matches(hash: &str, code: &str) -> Result<bool, HashError> {
    let phc = parse_phc(hash)?;
    let derived = argon2id(
        code.as_bytes(),
        &phc.salt,
        phc.time,
        phc.memory,
        phc.threads,
        phc.hash.len(),
    )?;
    Ok(ct_eq(&derived, &phc.hash))
}

fn argon2id(
    password: &[u8],
    salt: &[u8],
    time: u32,
    memory: u32,
    threads: u32,
    len: usize,
) -> Result<Vec<u8>, HashError> {
    let params = Params::new(memory, time, threads, Some(len)).map_err(|_| HashError::Params)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = vec![0u8; len];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|_| HashError::Params)?;
    Ok(out)
}

struct Phc {
    memory: u32,
    time: u32,
    threads: u32,
    salt: Vec<u8>,
    hash: Vec<u8>,
}

fn parse_phc(hash: &str) -> Result<Phc, HashError> {
    let rest = hash
        .strip_prefix("$argon2id$v=19$m=")
        .ok_or(HashError::Format)?;
    let (memory, rest) = split_number(rest, ",t=")?;
    let (time, rest) = split_number(rest, ",p=")?;
    let (threads, rest) = split_number(rest, "$")?;
    let (salt_b64, hash_b64) = rest.split_once('$').ok_or(HashError::Format)?;
    if memory > 1024 * 1024 || time == 0 || time > 20 || threads == 0 || threads > 16 {
        return Err(HashError::Params);
    }
    if memory < 8 * threads {
        return Err(HashError::Params);
    }
    if !is_raw_b64(salt_b64) || !is_raw_b64(hash_b64) {
        return Err(HashError::Format);
    }
    let salt = STANDARD_NO_PAD
        .decode(salt_b64)
        .map_err(|_| HashError::Format)?;
    let raw_hash = STANDARD_NO_PAD
        .decode(hash_b64)
        .map_err(|_| HashError::Format)?;
    if salt.len() < 8 || raw_hash.len() < 16 {
        return Err(HashError::Params);
    }
    Ok(Phc {
        memory,
        time,
        threads,
        salt,
        hash: raw_hash,
    })
}

fn split_number<'a>(input: &'a str, suffix: &str) -> Result<(u32, &'a str), HashError> {
    let end = input.find(suffix).ok_or(HashError::Format)?;
    let number = input[..end].parse::<u32>().map_err(|_| HashError::Format)?;
    Ok((number, &input[end + suffix.len()..]))
}

fn is_raw_b64(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/')
}

fn ct_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn fill_random(buf: &mut [u8]) -> Result<(), HashError> {
    let mut file = std::fs::File::open("/dev/urandom").map_err(|_| HashError::Random)?;
    std::io::Read::read_exact(&mut file, buf).map_err(|_| HashError::Random)
}

/// A recovery-code hash could not be built or read.
#[derive(Debug)]
pub(crate) enum HashError {
    /// `/dev/urandom` could not fill the salt.
    Random,
    /// The stored string is not the PHC subset GoTrue accepts.
    Format,
    /// Argon2id rejected the parameters or the password.
    Params,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_round_trip_and_normalization() {
        let code = generate_code(16).expect("code");
        let hash = hash_code(&code).expect("hash");
        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_eq!(matches(&hash, &code).ok(), Some(true));
        assert_eq!(matches(&hash, "nope-nope-nope-no").ok(), Some(false));
        assert!(matches("not-a-hash", &code).is_err());
        assert_eq!(normalize(" AbCd-EF\tgh\u{a0}"), "abcdefgh");
    }
}
