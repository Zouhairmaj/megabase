//! Bucket name and file size limit rules.

// Ported from supabase/storage src/storage/limits.ts,
// src/http/schemas/file-size-limit.ts and src/config.ts (Apache-2.0),
// pin v1.80.2.

/// `FILE_SIZE_LIMIT` in the pinned self-hosted compose file
/// (`vendor/supabase/docker/docker-compose.yml`), in bytes.
pub(crate) const UPLOAD_FILE_SIZE_LIMIT: f64 = 52_428_800.0;

/// Default `ICEBERG_BUCKET_DETECTION_SUFFIX`; names ending with it are reserved.
const ICEBERG_BUCKET_DETECTION_SUFFIX: &str = "--table-s3";

/// `VALID_BUCKET_NAME`: `^[A-Za-z0-9_!.*'() &$=@;:+,?-]*$`.
fn is_bucket_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_!.*'() &$=@;:+,?-".contains(c)
}

/// `isValidBucketName`: 1 to 100 S3-safe characters.
pub(crate) fn is_valid_bucket_name(name: &str) -> bool {
    // Every accepted character is ASCII, so bytes equal UTF-16 units.
    !name.is_empty() && name.len() < 101 && name.chars().all(is_bucket_name_char)
}

/// `createBucket` name checks: no surrounding whitespace, valid, and not
/// ending with a reserved suffix.
pub(crate) fn is_creatable_bucket_name(name: &str) -> bool {
    name.trim() == name
        && is_valid_bucket_name(name)
        && !name.ends_with(ICEBERG_BUCKET_DETECTION_SUFFIX)
}

/// `isUuid`: `UUID_PATTERN`, versions 0 to 5 and variants `0`, `8`, `9`, `a`, `b`.
pub(crate) fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            14 => (b'0'..=b'5').contains(b),
            19 => b"089abAB".contains(b),
            _ => b.is_ascii_hexdigit(),
        })
}

/// `fileSizeLimitSchema` string branch: `^[0-9]+(?:\.[0-9]+)?(?:gb|mb|kb|b)$`
/// without case, parsed to bytes as `parseFileSizeToBytes` does
/// (`Math.round(parseFloat(value) * unit)`). `None` when the pattern fails.
pub(crate) fn parse_file_size_to_bytes(value: &str) -> Option<f64> {
    let lower = value.to_ascii_lowercase();
    let (number, multiplier) = [("gb", 1e9), ("mb", 1e6), ("kb", 1000.0), ("b", 1.0)]
        .into_iter()
        .find_map(|(unit, multiplier)| lower.strip_suffix(unit).map(|n| (n, multiplier)))?;
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || fraction.is_some_and(|f| !digits(f)) {
        return None;
    }
    let parsed: f64 = number.parse().ok()?;
    Some(js_round(parsed * multiplier))
}

/// `Math.round`: halves round toward positive infinity.
fn js_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_names_follow_the_s3_safe_set() {
        assert!(is_valid_bucket_name("avatars"));
        assert!(is_valid_bucket_name("a b(c)!*'&$=@;:+,?-_."));
        assert!(is_valid_bucket_name(&"a".repeat(100)));
        assert!(!is_valid_bucket_name(&"a".repeat(101)));
        assert!(!is_valid_bucket_name(""));
        assert!(!is_valid_bucket_name("a/b"));
        assert!(!is_valid_bucket_name("é"));
        assert!(is_creatable_bucket_name("avatars"));
        assert!(!is_creatable_bucket_name(" avatars"));
        assert!(!is_creatable_bucket_name("avatars "));
        assert!(!is_creatable_bucket_name("logs--table-s3"));
    }

    #[test]
    fn uuids_follow_uuid_pattern() {
        assert!(is_uuid("6f1e2c3a-1b2c-4d5e-8f90-123456789abc"));
        assert!(is_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!is_uuid("6f1e2c3a-1b2c-6d5e-8f90-123456789abc"));
        assert!(!is_uuid("6f1e2c3a-1b2c-4d5e-cf90-123456789abc"));
        assert!(!is_uuid("6f1e2c3a1b2c4d5e8f90123456789abc"));
        assert!(!is_uuid("user-1"));
    }

    #[test]
    fn file_sizes_parse_like_parse_file_size_to_bytes() {
        assert_eq!(parse_file_size_to_bytes("100MB"), Some(100_000_000.0));
        assert_eq!(parse_file_size_to_bytes("1.5kb"), Some(1500.0));
        assert_eq!(parse_file_size_to_bytes("2Gb"), Some(2e9));
        assert_eq!(parse_file_size_to_bytes("7B"), Some(7.0));
        assert_eq!(parse_file_size_to_bytes("0.5b"), Some(1.0));
        for bad in ["MB", "1.MB", ".5MB", "1 MB", "-1MB", "1TB", "100", "1e3B"] {
            assert_eq!(parse_file_size_to_bytes(bad), None, "{bad}");
        }
    }
}
