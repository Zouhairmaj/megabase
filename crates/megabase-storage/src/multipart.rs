//! `multipart/form-data` upload bodies, as `fileUploadFromRequest` reads
//! them through `@fastify/multipart` (`specs/storage/object.md`).
//!
//! Only the parts before (and including) the first file part matter: the
//! fields that precede it are `formData.fields`, and later parts are never
//! read. Anything Megabase does not reproduce exactly is `None`, which the
//! upload handler answers with a 501.

// Ported from supabase/storage src/storage/uploader.ts
// (`fileUploadFromRequest`) (Apache-2.0), pin v1.80.2.

use axum::body::Bytes;

/// The longest boundary RFC 2046 allows.
const MAX_BOUNDARY_LEN: usize = 70;

/// The file part of a multipart upload and the fields sent before it.
#[derive(Debug)]
pub(super) struct MultipartUpload {
    /// The file part's bytes.
    pub(super) body: Bytes,
    /// `formData.fields.contentType?.value || formData.mimetype`.
    pub(super) mime_type: String,
    /// `max-age=<cacheControl field>`, or `no-cache` without one.
    pub(super) cache_control: String,
}

/// Reads `body` as `multipart/form-data` with the boundary in
/// `content_type`. `None` when the body is malformed, has no file part, or
/// uses something Megabase does not port (`contentEncoding`, `metadata`
/// and `userMetadata` fields, a repeated `contentType` field, a file part
/// without its own `Content-Type`).
pub(super) fn parse(content_type: &str, body: &Bytes) -> Option<MultipartUpload> {
    let boundary = boundary(content_type)?;
    let mut delimiter = Vec::with_capacity(boundary.len() + 4);
    delimiter.extend_from_slice(b"\r\n--");
    delimiter.extend_from_slice(boundary.as_bytes());

    // The first delimiter may open the body, without its leading CRLF.
    let mut at = if body.starts_with(&delimiter[2..]) {
        delimiter.len() - 2
    } else {
        find(body, &delimiter, 0)? + delimiter.len()
    };
    let mut fields: Vec<(String, String)> = Vec::new();
    loop {
        // `--` closes the body; anything else must start a part.
        let rest = body.get(at..)?;
        if rest.starts_with(b"--") {
            return None;
        }
        at += line_end(rest)?;
        let headers_end = find(body, b"\r\n\r\n", at)?;
        let headers = std::str::from_utf8(body.get(at..headers_end)?).ok()?;
        let part = PartHeaders::parse(headers)?;
        let content_start = headers_end + 4;
        let content_end = find(body, &delimiter, content_start)?;
        if part.is_file {
            return upload(&fields, part, body.slice(content_start..content_end));
        }
        let value = std::str::from_utf8(body.get(content_start..content_end)?).ok()?;
        fields.push((part.name, value.to_string()));
        at = content_end + delimiter.len();
    }
}

/// `fileUploadFromRequest`'s multipart branch once the file part is found.
fn upload(fields: &[(String, String)], part: PartHeaders, body: Bytes) -> Option<MultipartUpload> {
    fn values<'f>(
        fields: &'f [(String, String)],
        name: &'f str,
    ) -> impl Iterator<Item = &'f (String, String)> {
        fields.iter().filter(move |(field, _)| field == name)
    }
    let named = |name: &'static str| values(fields, name);
    // `contentEncoding` and user metadata are not ported. A file part named
    // `contentEncoding` is `InvalidParameter`, also not ported.
    if part.name == "contentEncoding"
        || ["contentEncoding", "metadata", "userMetadata"]
            .iter()
            .any(|name| named(name).next().is_some())
    {
        return None;
    }
    // A repeated `contentType` field is an array, whose `.value` is
    // `undefined`; that fallback is not ported.
    let mut content_types = named("contentType");
    let field_type = content_types.next().map(|(_, value)| value.as_str());
    if content_types.next().is_some() {
        return None;
    }
    let mime_type = match field_type.filter(|value| !value.is_empty()) {
        Some(value) => value.to_string(),
        None => part.mime_type?,
    };
    // Older storage-js versions send `cacheControl` twice; the first wins.
    let cache_control = match named("cacheControl").next() {
        Some((_, value)) if !value.is_empty() => format!("max-age={value}"),
        _ => "no-cache".to_string(),
    };
    Some(MultipartUpload {
        body,
        mime_type,
        cache_control,
    })
}

/// The `boundary` parameter of a `multipart/form-data` content type.
fn boundary(content_type: &str) -> Option<String> {
    let mut params = content_type.split(';');
    let essence = params.next()?.trim();
    if !essence.eq_ignore_ascii_case("multipart/form-data") {
        return None;
    }
    let mut found = None;
    for param in params {
        let (key, value) = param.trim().split_once('=')?;
        if key.trim().eq_ignore_ascii_case("boundary") {
            if found.is_some() {
                return None;
            }
            found = Some(param_value(value.trim())?);
        }
    }
    found.filter(|boundary| (1..=MAX_BOUNDARY_LEN).contains(&boundary.len()))
}

/// A parameter value: a token, or a quoted string with `\` escapes.
fn param_value(value: &str) -> Option<String> {
    let Some(quoted) = value.strip_prefix('"') else {
        return (!value.is_empty() && !value.contains(char::is_whitespace))
            .then(|| value.to_string());
    };
    let mut out = String::with_capacity(quoted.len());
    let mut chars = quoted.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return chars.as_str().trim().is_empty().then_some(out),
            '\\' => out.push(chars.next()?),
            c => out.push(c),
        }
    }
    None
}

/// What a part's headers say about it.
struct PartHeaders {
    name: String,
    is_file: bool,
    mime_type: Option<String>,
}

impl PartHeaders {
    /// `Content-Disposition: form-data; name=...` is required; a
    /// `filename` (or `filename*`) parameter makes the part a file. A
    /// `Content-Type` becomes its lowercased `type/subtype`, parameters
    /// dropped. Other dispositions and malformed headers are `None`.
    fn parse(headers: &str) -> Option<Self> {
        let mut disposition = None;
        let mut mime_type = None;
        for line in headers.split("\r\n") {
            let (name, value) = line.split_once(':')?;
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-disposition") {
                if disposition.replace(value).is_some() {
                    return None;
                }
            } else if name.eq_ignore_ascii_case("content-type") {
                if mime_type.is_some() {
                    return None;
                }
                mime_type = Some(essence(value)?);
            }
        }
        let mut params = disposition?.split(';');
        if !params.next()?.trim().eq_ignore_ascii_case("form-data") {
            return None;
        }
        let mut name = None;
        let mut is_file = false;
        for param in params {
            let (key, value) = param.trim().split_once('=')?;
            let key = key.trim();
            if key.eq_ignore_ascii_case("name") {
                if name.is_some() {
                    return None;
                }
                // supabase-js names its file part `""`.
                name = Some(param_value(value.trim())?);
            } else if key.eq_ignore_ascii_case("filename") || key.eq_ignore_ascii_case("filename*")
            {
                is_file = true;
            }
        }
        Some(Self {
            name: name?,
            is_file,
            mime_type,
        })
    }
}

/// `type/subtype` of a part content type, lowercased.
fn essence(value: &str) -> Option<String> {
    let essence = value.split(';').next()?.trim();
    let (kind, subtype) = essence.split_once('/')?;
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
    };
    (token(kind) && token(subtype)).then(|| essence.to_ascii_lowercase())
}

/// The length of the CRLF-terminated delimiter line ending at the start of
/// `rest` (transport padding allowed), including the CRLF.
fn line_end(rest: &[u8]) -> Option<usize> {
    let padding = rest
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    rest.get(padding..)?
        .starts_with(b"\r\n")
        .then_some(padding + 2)
}

/// The first `needle` in `haystack` at or after `from`.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPE: &str = "multipart/form-data; boundary=----b";

    fn body(parts: &str) -> Bytes {
        Bytes::from(parts.replace('\n', "\r\n"))
    }

    #[test]
    fn supabase_js_form_data_is_read() {
        let upload = parse(
            TYPE,
            &body(
                "------b\n\
                 Content-Disposition: form-data; name=\"cacheControl\"\n\
                 \n\
                 3600\n\
                 ------b\n\
                 Content-Disposition: form-data; name=\"\"; filename=\"blob\"\n\
                 Content-Type: image/png\n\
                 \n\
                 PNG\n\
                 bytes\n\
                 ------b--\n",
            ),
        )
        .unwrap();
        assert_eq!(&upload.body[..], b"PNG\r\nbytes");
        assert_eq!(upload.mime_type, "image/png");
        assert_eq!(upload.cache_control, "max-age=3600");
    }

    #[test]
    fn defaults_and_overrides() {
        let upload = parse(
            "multipart/form-data; boundary=\"----b\"",
            &body(
                "preamble\n\
                 ------b\n\
                 Content-Disposition: form-data; name=\"contentType\"\n\
                 \n\
                 text/csv\n\
                 ------b\n\
                 content-disposition: form-data; name=\"file\"; filename=\"a.txt\"\n\
                 content-type: Text/Plain; charset=utf-8\n\
                 \n\
                 a,b\n\
                 ------b\n\
                 Content-Disposition: form-data; name=\"cacheControl\"\n\
                 \n\
                 60\n\
                 ------b--",
            ),
        )
        .unwrap();
        assert_eq!(&upload.body[..], b"a,b");
        assert_eq!(upload.mime_type, "text/csv");
        // A field after the file is not in `formData.fields`.
        assert_eq!(upload.cache_control, "no-cache");

        let upload = parse(
            TYPE,
            &body(
                "------b\n\
                 Content-Disposition: form-data; name=\"\"; filename=\"blob\"\n\
                 Content-Type: text/plain;charset=UTF-8\n\
                 \n\
                 hi\n\
                 ------b--\n",
            ),
        )
        .unwrap();
        assert_eq!(upload.mime_type, "text/plain");
    }

    #[test]
    fn unported_or_malformed_forms_are_none() {
        let file = "------b\n\
                    Content-Disposition: form-data; name=\"\"; filename=\"blob\"\n\
                    Content-Type: image/png\n\
                    \n\
                    x\n\
                    ------b--\n";
        let field = |name: &str| {
            format!("------b\nContent-Disposition: form-data; name=\"{name}\"\n\nv\n{file}")
        };
        for (content_type, form) in [
            ("multipart/mixed; boundary=----b", file.to_string()),
            ("multipart/form-data", file.to_string()),
            (TYPE, field("contentEncoding")),
            (TYPE, field("metadata")),
            (TYPE, field("userMetadata")),
            (
                TYPE,
                format!(
                    "------b\nContent-Disposition: form-data; name=\"contentType\"\n\na/b\n{}",
                    field("contentType")
                ),
            ),
            // No file part.
            (
                TYPE,
                "------b\nContent-Disposition: form-data; name=\"a\"\n\nv\n------b--\n".into(),
            ),
            // A file part without its own content type.
            (
                TYPE,
                "------b\nContent-Disposition: form-data; name=\"\"; filename=\"b\"\n\nx\n------b--\n"
                    .into(),
            ),
            // Unterminated file part.
            (
                TYPE,
                "------b\nContent-Disposition: form-data; name=\"\"; filename=\"b\"\nContent-Type: a/b\n\nx"
                    .into(),
            ),
            (TYPE, "garbage".into()),
            (TYPE, String::new()),
        ] {
            assert!(parse(content_type, &body(&form)).is_none(), "{form}");
        }
    }

    #[test]
    fn boundaries_and_values() {
        assert_eq!(boundary(TYPE).as_deref(), Some("----b"));
        assert_eq!(
            boundary("Multipart/Form-Data; charset=x; BOUNDARY=\"a b\"").as_deref(),
            Some("a b")
        );
        assert_eq!(boundary("multipart/form-data; boundary="), None);
        assert_eq!(
            boundary(&format!("multipart/form-data; boundary={}", "x".repeat(71))),
            None
        );
        assert_eq!(param_value("\"a\\\"b\"").as_deref(), Some("a\"b"));
        assert_eq!(param_value("\"open"), None);
        assert_eq!(essence("image/PNG; q=1").as_deref(), Some("image/png"));
        assert_eq!(essence("nonsense"), None);
    }
}
