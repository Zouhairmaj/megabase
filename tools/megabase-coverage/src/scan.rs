//! Small, language-aware helpers for scanning upstream source text.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::Regex;

use crate::model::Source;

pub struct Repo {
    pub name: String,
    pub root: PathBuf,
}

impl Repo {
    pub fn read(&self, rel: &str) -> Result<String> {
        let path = self.root.join(rel);
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    }

    pub fn source(&self, rel: &str, text: &str, offset: usize) -> Source {
        Source {
            repo: self.name.clone(),
            file: rel.to_string(),
            line: line_at(text, offset),
        }
    }

    /// Files under `dir` (relative to the repo root) whose name passes `keep`,
    /// as sorted repo-relative paths with `/` separators.
    pub fn files(&self, dir: &str, keep: impl Fn(&str) -> bool) -> Result<Vec<String>> {
        let mut out = Vec::new();
        walk(&self.root.join(dir), &mut |path| {
            let rel = path
                .strip_prefix(&self.root)
                .expect("walked path is under root")
                .to_string_lossy()
                .replace('\\', "/");
            if keep(&rel) {
                out.push(rel);
            }
        })?;
        out.sort();
        Ok(out)
    }
}

fn walk(dir: &Path, f: &mut dyn FnMut(&Path)) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let ft = entry.file_type()?;
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            walk(&path, f)?;
        } else {
            f(&path);
        }
    }
    Ok(())
}

pub fn line_at(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

#[derive(Clone, Copy)]
pub struct Syntax {
    pub line_comment: &'static str,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub quotes: &'static [char],
}

pub const GO: Syntax = Syntax {
    line_comment: "//",
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '`'],
};
pub const TS: Syntax = Syntax {
    line_comment: "//",
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\'', '`'],
};
pub const ELIXIR: Syntax = Syntax {
    line_comment: "#",
    block_comment: None,
    quotes: &['"'],
};
pub const HASKELL: Syntax = Syntax {
    line_comment: "--",
    block_comment: Some(("{-", "-}")),
    quotes: &['"'],
};

/// Source with comments replaced by spaces (`code`) and, additionally, string
/// contents replaced by spaces (`skeleton`). Both keep every byte offset and
/// newline of the original, so matches in one can be used in the other.
pub struct Cleaned {
    pub code: String,
    pub skeleton: String,
}

pub fn clean(text: &str, syntax: Syntax) -> Cleaned {
    let bytes = text.as_bytes();
    let mut code = bytes.to_vec();
    let mut skeleton = bytes.to_vec();
    let blank = |buf: &mut Vec<u8>, i: usize| {
        if buf[i] != b'\n' {
            buf[i] = b' ';
        }
    };
    let mut i = 0;
    while i < bytes.len() {
        let rest = &bytes[i..];
        if rest.starts_with(syntax.line_comment.as_bytes()) {
            while i < bytes.len() && bytes[i] != b'\n' {
                blank(&mut code, i);
                blank(&mut skeleton, i);
                i += 1;
            }
            continue;
        }
        if let Some((open, close)) = syntax.block_comment {
            if rest.starts_with(open.as_bytes()) {
                let end = rest[open.len()..]
                    .windows(close.len())
                    .position(|w| w == close.as_bytes())
                    .map(|p| i + open.len() + p + close.len())
                    .unwrap_or(bytes.len());
                for j in i..end {
                    blank(&mut code, j);
                    blank(&mut skeleton, j);
                }
                i = end;
                continue;
            }
        }
        let c = bytes[i] as char;
        if syntax.quotes.contains(&c) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] as char != c {
                if bytes[j] == b'\\' {
                    j += 1;
                } else if bytes[j] == b'\n' && c != '`' {
                    break;
                }
                j += 1;
            }
            for k in (i + 1)..j.min(bytes.len()) {
                blank(&mut skeleton, k);
            }
            i = j + 1;
            continue;
        }
        i += 1;
    }
    Cleaned {
        code: String::from_utf8(code).expect("only ASCII bytes were replaced"),
        skeleton: String::from_utf8(skeleton).expect("only ASCII bytes were replaced"),
    }
}

/// Offset just past the `}` that closes the `{` at `open`, using a skeleton.
pub fn block_end(skeleton: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, b) in skeleton.bytes().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    skeleton.len()
}

/// Rewrites upstream path parameter syntaxes to `{name}`:
/// `:id(\\d+)` and `:id` (fastify, Phoenix), `[id]` / `[...slug]` (Next.js),
/// `*` (fastify wildcard), and `${expr.name}` (template literals).
pub fn normalize_path(path: &str) -> String {
    let colon = Regex::new(r":([A-Za-z_][A-Za-z0-9_]*)(\([^)]*\))?").unwrap();
    let next_catch = Regex::new(r"\[\[?\.\.\.([A-Za-z0-9_]+)\]?\]").unwrap();
    let next_param = Regex::new(r"\[([A-Za-z0-9_]+)\]").unwrap();
    let template = Regex::new(r"\$\{([A-Za-z0-9_]+)[^}]*\}").unwrap();
    let p = template.replace_all(path, "{$1}");
    let p = colon.replace_all(&p, "{$1}");
    let p = next_catch.replace_all(&p, "{*$1}");
    let p = next_param.replace_all(&p, "{$1}");
    let p = p.replace("/*", "/{*}");
    if p == "*" {
        return "/{*}".into();
    }
    p
}

/// Joins path segments the way routers nest prefixes: `["/", "/a/", "b"]`
/// becomes `/a/b`, and an empty result is `/`.
pub fn join_paths(parts: &[&str]) -> String {
    let mut out = String::new();
    for part in parts {
        let trimmed = part.trim_matches('/');
        if !trimmed.is_empty() {
            out.push('/');
            out.push_str(trimmed);
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

/// Resolves `NAME = "value"` / `const NAME = 'value'` definitions in a file.
pub fn string_consts(text: &str) -> Vec<(String, String)> {
    let re = Regex::new(
        r#"(?m)\b([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*[A-Za-z]+\s*)?=\s*["']([^"'\n]*)["']"#,
    )
    .unwrap();
    re.captures_iter(text)
        .map(|c| (c[1].to_string(), c[2].to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_parameter_syntaxes() {
        assert_eq!(normalize_path("/:id(\\\\d+)"), "/{id}");
        assert_eq!(
            normalize_path("/object/:bucketName/*"),
            "/object/{bucketName}/{*}"
        );
        assert_eq!(
            normalize_path("/project/[ref]/sql/[id]"),
            "/project/{ref}/sql/{id}"
        );
        assert_eq!(normalize_path("/docs/[[...slug]]"), "/docs/{*slug}");
        assert_eq!(
            normalize_path("/generators/${language.name}"),
            "/generators/{language}"
        );
    }

    #[test]
    fn joins_nested_prefixes() {
        assert_eq!(join_paths(&["/", "/admin", "/"]), "/admin");
        assert_eq!(join_paths(&["/", "/"]), "/");
        assert_eq!(
            join_paths(&["upload/resumable", "/sign", "/*"]),
            "/upload/resumable/sign/*"
        );
    }

    #[test]
    fn block_end_ignores_closing_brace_before_open() {
        let src = "} { inner }";
        let open = src.find('{').unwrap();
        assert_eq!(&src[..block_end(src, open)], "} { inner }");
        assert_eq!(block_end("no braces", 0), "no braces".len());
    }

    #[test]
    fn clean_keeps_offsets() {
        let src = "a // c {\nb \"{x}\" {";
        let cleaned = clean(src, GO);
        assert_eq!(cleaned.code.len(), src.len());
        assert!(!cleaned.code.contains("c {"));
        assert!(cleaned.code.contains("\"{x}\""));
        assert!(!cleaned.skeleton.contains("{x}"));
        assert!(cleaned.skeleton.ends_with('{'));
    }
}
