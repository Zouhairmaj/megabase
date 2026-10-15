//! Bucket `allowed_mime_types` normalization.

// Ported from supabase/storage src/storage/validators/mime-type.ts
// (Apache-2.0), pin v1.80.2.

/// Longest entry `normalizeAllowedMimeTypes` parses, in UTF-16 code units.
const MAX_MIME_TYPE_LENGTH: usize = 1000;

/// RFC 9110 `tchar`.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+.^_`|~-".contains(c)
}

fn is_space(c: char) -> bool {
    c == '\t' || c == ' '
}

/// `qdtext` of an RFC 9110 `quoted-string`. JavaScript matches `\x80-\xff`
/// against UTF-16 code units, so those are the characters U+0080 to U+00FF.
fn is_qdtext(c: char) -> bool {
    matches!(c, '\t' | ' ' | '\x21' | '\x23'..='\x5b' | '\x5d'..='\x7e' | '\u{80}'..='\u{ff}')
}

/// The character after `\` in a `quoted-pair`.
fn is_quoted_pair_char(c: char) -> bool {
    matches!(c, '\t' | '\x20'..='\x7e' | '\u{80}'..='\u{ff}')
}

/// Character cursor over the input; the upstream regular expressions never
/// need to backtrack because tokens, spaces and separators are disjoint.
struct Cursor<'a> {
    rest: &'a str,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<char> {
        self.rest.chars().next()
    }

    fn eat(&mut self, c: char) -> bool {
        match self.rest.strip_prefix(c) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> &'a str {
        let end = self
            .rest
            .char_indices()
            .find(|(_, c)| !keep(*c))
            .map_or(self.rest.len(), |(i, _)| i);
        let (taken, rest) = self.rest.split_at(end);
        self.rest = rest;
        taken
    }

    fn token(&mut self) -> Option<&'a str> {
        Some(self.take_while(is_token_char)).filter(|token| !token.is_empty())
    }

    fn quoted_string(&mut self) -> bool {
        if !self.eat('"') {
            return false;
        }
        loop {
            match self.peek() {
                Some('"') => {
                    self.eat('"');
                    return true;
                }
                Some('\\') => {
                    self.eat('\\');
                    match self.peek() {
                        Some(c) if is_quoted_pair_char(c) => {
                            self.eat(c);
                        }
                        _ => return false,
                    }
                }
                Some(c) if is_qdtext(c) => {
                    self.eat(c);
                }
                _ => return false,
            }
        }
    }

    /// `PARAMETER`: `;[\t ]*(?:token=(?:token|quoted-string)[\t ]*)?`.
    /// A failed optional group leaves a token character next, which the
    /// following `;` match rejects, so the whole parse fails there.
    fn parameter(&mut self) -> bool {
        if !self.eat(';') {
            return false;
        }
        self.take_while(is_space);
        match self.peek() {
            None | Some(';') => true,
            Some(_) => {
                if self.token().is_none() || !self.eat('=') {
                    return false;
                }
                if self.token().is_none() && !self.quoted_string() {
                    return false;
                }
                self.take_while(is_space);
                true
            }
        }
    }
}

/// `parseMediaType`: the lowercase `type/subtype`, or `None` when invalid.
fn parse_media_type(mime_type: &str, allow_subtype_wildcard: bool) -> Option<String> {
    let mut cursor = Cursor { rest: mime_type };
    cursor.take_while(is_space);
    let kind = cursor.token()?.to_ascii_lowercase();
    if !cursor.eat('/') {
        return None;
    }
    let subtype = cursor.token()?.to_ascii_lowercase();
    cursor.take_while(is_space);
    if kind.contains('*') || (subtype.contains('*') && !(allow_subtype_wildcard && subtype == "*"))
    {
        return None;
    }
    while !cursor.rest.is_empty() {
        if !cursor.parameter() {
            return None;
        }
    }
    Some(format!("{kind}/{subtype}"))
}

/// `normalizeAllowedMimeTypes`: each entry as its lowercase media type,
/// first occurrence order, duplicates dropped.
///
/// # Errors
///
/// The first entry that is not a media type (`type/subtype` with optional
/// parameters, `type/*` allowed), or is longer than 1000 UTF-16 units.
pub(crate) fn normalize_allowed_mime_types(mime_types: &[String]) -> Result<Vec<String>, &str> {
    let mut normalized: Vec<String> = Vec::with_capacity(mime_types.len());
    for mime_type in mime_types {
        let parsed = if mime_type.encode_utf16().count() <= MAX_MIME_TYPE_LENGTH {
            parse_media_type(mime_type, true)
        } else {
            None
        };
        let Some(parsed) = parsed else {
            return Err(mime_type);
        };
        if !normalized.contains(&parsed) {
            normalized.push(parsed);
        }
    }
    Ok(normalized)
}

/// `validateMimeType`: whether an upload's `mime_type` matches one of the
/// bucket's `allowed` entries exactly or as `type/*`.
pub(crate) fn is_allowed_mime_type(mime_type: &str, allowed: &[String]) -> bool {
    let Some(requested) = parse_media_type(mime_type, false) else {
        return false;
    };
    let wildcard = format!("{}*", &requested[..=requested.find('/').unwrap_or(0)]);
    allowed.iter().any(|entry| {
        *entry == requested
            || *entry == wildcard
            || parse_media_type(entry, true)
                .is_some_and(|parsed| parsed == requested || parsed == wildcard)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_mime_types_match_exact_or_wildcard_entries() {
        let allowed = vec!["image/png".to_string(), "text/*".to_string()];
        assert!(is_allowed_mime_type("image/png", &allowed));
        assert!(is_allowed_mime_type("IMAGE/PNG; charset=x", &allowed));
        assert!(is_allowed_mime_type("text/plain", &allowed));
        assert!(!is_allowed_mime_type("image/jpeg", &allowed));
        assert!(!is_allowed_mime_type("not a type", &allowed));
        assert!(!is_allowed_mime_type("image/*", &allowed));
    }

    fn normalize(input: &[&str]) -> Result<Vec<String>, String> {
        let owned: Vec<String> = input.iter().map(|s| (*s).to_string()).collect();
        normalize_allowed_mime_types(&owned).map_err(str::to_string)
    }

    #[test]
    fn media_types_are_lowercased_and_deduplicated() {
        assert_eq!(
            normalize(&[
                "Image/PNG",
                " image/png ; charset=utf-8",
                "image/*",
                "text/plain;a=\"b\\\"c\";"
            ]),
            Ok(vec![
                "image/png".to_string(),
                "image/*".to_string(),
                "text/plain".to_string()
            ])
        );
        assert_eq!(normalize(&[]), Ok(vec![]));
        assert_eq!(normalize(&["a/b;"]), Ok(vec!["a/b".to_string()]));
        assert_eq!(normalize(&["a/b; ;"]), Ok(vec!["a/b".to_string()]));
    }

    #[test]
    fn invalid_media_types_name_the_entry() {
        for bad in [
            "image",
            "*/*",
            "image/png*",
            "image/png x",
            "image/png;a",
            "image/png;a=",
            "image/png;a=\"b",
            "/png",
            "",
        ] {
            assert_eq!(
                normalize(&["image/png", bad]),
                Err(bad.to_string()),
                "{bad}"
            );
        }
        let long = format!("a/{}", "b".repeat(999));
        assert_eq!(normalize(&[&long]), Err(long.clone()));
        assert!(normalize(&[&long[..1000]]).is_ok());
    }
}
