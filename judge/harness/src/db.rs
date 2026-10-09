//! PostgreSQL catalog and row snapshots for both judge databases.
//!
//! The harness talks to Postgres itself. It does not import Megabase crates
//! and it does not trust Megabase HTTP for schema or row state.

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use postgres::{Client, NoTls};
use serde_json::Value;

use crate::normalize;

/// How long `wait_for` retries a connection before giving up.
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);
const RETRY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub struct Databases {
    pub reference: String,
    pub megabase: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub schema: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct TableCatalog {
    pub columns: Vec<Column>,
    pub row_security: bool,
    pub indexes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Column {
    pub name: String,
    pub typ: String,
    pub not_null: bool,
    pub generated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct FunctionCatalog {
    pub args: String,
    pub result: String,
    pub language: String,
    pub volatility: String,
    pub body: String,
}

pub fn parse_relation(raw: &str) -> Result<Relation> {
    let trimmed = raw.trim().trim_end_matches("()");
    let Some((schema, name)) = trimmed.split_once('.') else {
        bail!("expected `schema.name`, got `{raw}`");
    };
    if !is_ident(schema) || !is_ident(name) {
        bail!("invalid identifier in `{raw}`");
    }
    Ok(Relation {
        schema: schema.to_string(),
        name: name.to_string(),
    })
}

pub fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z' | 'A'..='Z' | '_'))
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn sanitize_url(url: &str) -> String {
    let Some(scheme) = url.find("://") else {
        return url.to_string();
    };
    let Some(at) = url[scheme + 3..].find('@') else {
        return url.to_string();
    };
    let creds = &url[scheme + 3..scheme + 3 + at];
    let Some(colon) = creds.find(':') else {
        return url.to_string();
    };
    format!(
        "{}{}:***{}",
        &url[..scheme + 3],
        &creds[..colon],
        &url[scheme + 3 + at..]
    )
}

fn connect(url: &str) -> Result<Client> {
    Client::connect(url, NoTls).with_context(|| format!("connecting to {}", sanitize_url(url)))
}

fn connect_retry(url: &str, timeout: Duration) -> Result<Client> {
    let deadline = Instant::now() + timeout;
    loop {
        match connect(url) {
            Ok(client) => return Ok(client),
            Err(err) => {
                if Instant::now() > deadline {
                    return Err(err).context(format!(
                        "timed out after {}s waiting for {}",
                        timeout.as_secs(),
                        sanitize_url(url)
                    ));
                }
                std::thread::sleep(RETRY);
            }
        }
    }
}

pub fn ping(url: &str) -> Result<()> {
    drop(connect(url)?);
    Ok(())
}

fn database_name(url: &str) -> Result<String> {
    let config: postgres::Config = url
        .parse()
        .with_context(|| format!("parsing {}", sanitize_url(url)))?;
    Ok(config.get_dbname().unwrap_or("postgres").to_string())
}

/// Create the Megabase-side database if needed and load judge fixtures into it.
///
/// The official stack keeps using the cluster's `postgres` database. Sharing
/// that database would make side-effect checks vacuous (same rows on both
/// "sides") or inverted (two writers, one table).
pub fn prepare(admin_url: &str, megabase_url: &str, fixtures: &str) -> Result<()> {
    let dbname = database_name(megabase_url)?;
    if !is_ident(&dbname) {
        bail!("megabase database name `{dbname}` is not a safe identifier");
    }
    let mut admin = connect_retry(admin_url, WAIT_TIMEOUT)?;
    let exists: bool = admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
            &[&dbname],
        )?
        .get(0);
    if !exists {
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .with_context(|| format!("CREATE DATABASE {dbname}"))?;
        eprintln!("created database {dbname}");
    } else {
        eprintln!("database {dbname} already exists");
    }
    drop(admin);

    let mut megabase = connect_retry(megabase_url, WAIT_TIMEOUT)?;
    let loaded: bool = megabase
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = 'todos'
            )",
            &[],
        )?
        .get(0);
    if !loaded {
        megabase
            .batch_execute(fixtures)
            .context("loading judge fixtures into the megabase database")?;
        eprintln!("loaded judge fixtures into {dbname}");
    } else {
        eprintln!("judge fixtures already present in {dbname}");
    }
    Ok(())
}

pub fn normalize_sql(sql: &str) -> String {
    let without_line_comments = regex::Regex::new(r"--[^\n]*")
        .expect("comment regex")
        .replace_all(sql, " ");
    let compact = regex::Regex::new(r"\s+")
        .expect("ws regex")
        .replace_all(&without_line_comments, " ")
        .trim()
        .to_string();
    lowercase_sql_outside_literals(&compact)
}

/// Lowercase SQL keywords and identifiers, but keep `'quoted'` literal case
/// so `'email'` and `'EMAIL'` do not compare equal.
fn lowercase_sql_outside_literals(sql: &str) -> String {
    let mut result = String::with_capacity(sql.len());
    let mut in_literal = false;
    let mut chars = sql.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            result.push(ch);
            if in_literal && chars.peek() == Some(&'\'') {
                result.push(chars.next().expect("escaped quote"));
            } else {
                in_literal = !in_literal;
            }
        } else if in_literal {
            result.push(ch);
        } else {
            result.extend(ch.to_lowercase());
        }
    }
    result
}

fn sort_row_array(value: &mut Value) {
    if let Value::Array(items) = value {
        items.sort_by_cached_key(Value::to_string);
    }
}

fn table_catalog(client: &mut Client, rel: &Relation) -> Result<Option<TableCatalog>> {
    let found: bool = client
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p')
            )",
            &[&rel.schema, &rel.name],
        )?
        .get(0);
    if !found {
        return Ok(None);
    }
    let columns = client.query(
        "SELECT a.attname,
                t.typname,
                a.attnotnull,
                CASE WHEN a.attgenerated = 's'
                     THEN pg_get_expr(ad.adbin, ad.adrelid)
                     ELSE NULL
                END
           FROM pg_attribute a
           JOIN pg_class c ON c.oid = a.attrelid
           JOIN pg_namespace n ON n.oid = c.relnamespace
           JOIN pg_type t ON t.oid = a.atttypid
           LEFT JOIN pg_attrdef ad
             ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
          WHERE n.nspname = $1
            AND c.relname = $2
            AND a.attnum > 0
            AND NOT a.attisdropped
          ORDER BY a.attnum",
        &[&rel.schema, &rel.name],
    )?;
    let row_security: bool = client
        .query_one(
            "SELECT c.relrowsecurity
               FROM pg_class c
               JOIN pg_namespace n ON n.oid = c.relnamespace
              WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind IN ('r', 'p')",
            &[&rel.schema, &rel.name],
        )?
        .get(0);
    let indexes = client.query(
        "SELECT pg_get_indexdef(ix.indexrelid)
           FROM pg_index ix
           JOIN pg_class t ON t.oid = ix.indrelid
           JOIN pg_namespace n ON n.oid = t.relnamespace
          WHERE n.nspname = $1 AND t.relname = $2
          ORDER BY pg_get_indexdef(ix.indexrelid)",
        &[&rel.schema, &rel.name],
    )?;
    Ok(Some(TableCatalog {
        columns: columns
            .iter()
            .map(|row| Column {
                name: row.get(0),
                typ: row.get(1),
                not_null: row.get(2),
                generated: row.get::<_, Option<String>>(3).map(|s| normalize_sql(&s)),
            })
            .collect(),
        row_security,
        indexes: indexes
            .iter()
            .map(|row| normalize_sql(row.get::<_, String>(0).as_str()))
            .collect(),
    }))
}

fn function_catalog(client: &mut Client, rel: &Relation) -> Result<Option<Vec<FunctionCatalog>>> {
    let rows = client.query(
        "SELECT pg_get_function_identity_arguments(p.oid),
                pg_get_function_result(p.oid),
                l.lanname,
                p.provolatile::text,
                p.prosrc
           FROM pg_proc p
           JOIN pg_namespace n ON n.oid = p.pronamespace
           JOIN pg_language l ON l.oid = p.prolang
          WHERE n.nspname = $1 AND p.proname = $2
          ORDER BY pg_get_function_identity_arguments(p.oid)",
        &[&rel.schema, &rel.name],
    )?;
    if rows.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        rows.iter()
            .map(|row| FunctionCatalog {
                args: row.get(0),
                result: normalize_sql(&row.get::<_, String>(1)),
                language: row.get(2),
                volatility: row.get(3),
                body: normalize_sql(&row.get::<_, String>(4)),
            })
            .collect(),
    ))
}

fn snapshot_rows(client: &mut Client, rel: &Relation) -> Result<Option<Value>> {
    let found: bool = client
        .query_one(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = $1 AND table_name = $2
            )",
            &[&rel.schema, &rel.name],
        )?
        .get(0);
    if !found {
        return Ok(None);
    }
    // Qualified identifiers are validated in parse_relation.
    let sql = format!(
        "SELECT COALESCE(
            (SELECT jsonb_agg(row_to_json(t))
               FROM (SELECT * FROM {}.{} ORDER BY 1) t),
            '[]'::jsonb
        )",
        rel.schema, rel.name
    );
    let value: Value = client.query_one(&sql, &[])?.get(0);
    Ok(Some(value))
}

fn describe_json(label: &str, value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() > 300 {
        format!("{label}: {}…", text.chars().take(300).collect::<String>())
    } else {
        format!("{label}: {text}")
    }
}

fn describe_table_diff(raw: &str, reference: &TableCatalog, megabase: &TableCatalog) -> String {
    let names = |cat: &TableCatalog| {
        cat.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    if reference
        .columns
        .iter()
        .map(|c| &c.name)
        .collect::<Vec<_>>()
        != megabase.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    {
        return format!(
            "table `{raw}` catalog differs: columns [{names}] vs [{other}]",
            names = names(reference),
            other = names(megabase)
        );
    }
    format!(
        "table `{raw}` catalog differs: {} vs {}",
        describe_json(
            "reference",
            &serde_json::to_value(reference).unwrap_or(Value::Null)
        ),
        describe_json(
            "megabase",
            &serde_json::to_value(megabase).unwrap_or(Value::Null)
        )
    )
}

/// Both sides missing is a match when the case requires absence.
pub fn absent_mismatch(ref_found: bool, mb_found: bool, kind: &str, raw: &str) -> Option<String> {
    match (ref_found, mb_found) {
        (false, false) => None,
        (true, false) => Some(format!(
            "{kind} `{raw}` exists on the reference stack but the case requires it to be absent"
        )),
        (false, true) => Some(format!(
            "{kind} `{raw}` exists on megabase but must be absent to match the pin"
        )),
        (true, true) => Some(format!(
            "{kind} `{raw}` exists on both stacks but the case requires it to be absent"
        )),
    }
}

/// Compare a table's catalog (and optionally rows) on both databases.
pub fn compare_table(
    databases: &Databases,
    raw: &str,
    rows: bool,
    reference_missing_is_fatal: bool,
    absent: bool,
) -> Result<Option<String>> {
    let rel = parse_relation(raw)?;
    let mut reference = connect(&databases.reference)?;
    let mut megabase = match connect(&databases.megabase) {
        Ok(client) => client,
        Err(err) => {
            return Ok(Some(format!(
                "megabase database unreachable for `{raw}`: {err:#}"
            )))
        }
    };
    let ref_cat = table_catalog(&mut reference, &rel)
        .with_context(|| format!("catalog `{raw}` on the reference stack"))?;
    let mb_cat = table_catalog(&mut megabase, &rel)
        .with_context(|| format!("catalog `{raw}` on megabase"))?;
    if absent {
        return Ok(absent_mismatch(
            ref_cat.is_some(),
            mb_cat.is_some(),
            "table",
            raw,
        ));
    }
    let Some(ref_cat) = ref_cat else {
        if reference_missing_is_fatal {
            bail!("reference database has no table `{raw}` (fixture or pin problem)");
        }
        return Ok(Some(format!(
            "table `{raw}` missing on the reference stack"
        )));
    };
    let Some(mb_cat) = mb_cat else {
        return Ok(Some(format!("table `{raw}` missing on megabase")));
    };
    if ref_cat != mb_cat {
        return Ok(Some(describe_table_diff(raw, &ref_cat, &mb_cat)));
    }
    if rows {
        return compare_row_snapshot(databases, raw, reference_missing_is_fatal);
    }
    Ok(None)
}

/// Compare a function's overloads on both databases.
pub fn compare_function(
    databases: &Databases,
    raw: &str,
    reference_missing_is_fatal: bool,
    absent: bool,
) -> Result<Option<String>> {
    let rel = parse_relation(raw)?;
    let mut reference = connect(&databases.reference)?;
    let mut megabase = match connect(&databases.megabase) {
        Ok(client) => client,
        Err(err) => {
            return Ok(Some(format!(
                "megabase database unreachable for `{raw}`: {err:#}"
            )))
        }
    };
    let ref_fn = function_catalog(&mut reference, &rel)
        .with_context(|| format!("function `{raw}` on the reference stack"))?;
    let mb_fn = function_catalog(&mut megabase, &rel)
        .with_context(|| format!("function `{raw}` on megabase"))?;
    if absent {
        return Ok(absent_mismatch(
            ref_fn.is_some(),
            mb_fn.is_some(),
            "function",
            raw,
        ));
    }
    let Some(ref_fn) = ref_fn else {
        if reference_missing_is_fatal {
            bail!("reference database has no function `{raw}` (pin problem)");
        }
        return Ok(Some(format!(
            "function `{raw}` missing on the reference stack"
        )));
    };
    let Some(mb_fn) = mb_fn else {
        return Ok(Some(format!("function `{raw}` missing on megabase")));
    };
    if ref_fn != mb_fn {
        return Ok(Some(format!(
            "function `{raw}` differs: {} vs {}",
            describe_json("reference", &serde_json::to_value(&ref_fn)?),
            describe_json("megabase", &serde_json::to_value(&mb_fn)?)
        )));
    }
    Ok(None)
}

/// Snapshot every row of a relation on both databases and compare after
/// the same JSON normalization used for HTTP bodies.
pub fn compare_row_snapshot(
    databases: &Databases,
    raw: &str,
    reference_missing_is_fatal: bool,
) -> Result<Option<String>> {
    let rel = parse_relation(raw)?;
    let mut reference = connect(&databases.reference)?;
    let mut megabase = match connect(&databases.megabase) {
        Ok(client) => client,
        Err(err) => {
            return Ok(Some(format!(
                "megabase database unreachable for `{raw}`: {err:#}"
            )))
        }
    };
    let Some(mut ref_rows) = snapshot_rows(&mut reference, &rel)
        .with_context(|| format!("snapshot `{raw}` on the reference stack"))?
    else {
        if reference_missing_is_fatal {
            bail!("reference database has no table `{raw}` to snapshot");
        }
        return Ok(Some(format!(
            "table `{raw}` missing on the reference stack"
        )));
    };
    let Some(mut mb_rows) = snapshot_rows(&mut megabase, &rel)
        .with_context(|| format!("snapshot `{raw}` on megabase"))?
    else {
        return Ok(Some(format!(
            "table `{raw}` missing on megabase (no rows to compare)"
        )));
    };
    normalize::json(&mut ref_rows, &[]);
    normalize::json(&mut mb_rows, &[]);
    sort_row_array(&mut ref_rows);
    sort_row_array(&mut mb_rows);
    if ref_rows != mb_rows {
        return Ok(Some(format!(
            "rows in `{raw}` differ: {} vs {}",
            describe_json("reference", &ref_rows),
            describe_json("megabase", &mb_rows)
        )));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qualified_names() {
        let rel = parse_relation("auth.uid()").unwrap();
        assert_eq!(rel.schema, "auth");
        assert_eq!(rel.name, "uid");
        let rel = parse_relation("public.todos").unwrap();
        assert_eq!(rel.name, "todos");
        assert!(parse_relation("todos").is_err());
        assert!(parse_relation("auth.users;drop").is_err());
        assert!(parse_relation("auth.foo-bar").is_err());
    }

    #[test]
    fn sanitizes_passwords() {
        assert_eq!(
            sanitize_url("postgres://postgres:secret@127.0.0.1:5432/megabase"),
            "postgres://postgres:***@127.0.0.1:5432/megabase"
        );
        assert_eq!(
            sanitize_url("postgres://postgres@127.0.0.1/postgres"),
            "postgres://postgres@127.0.0.1/postgres"
        );
    }

    #[test]
    fn sql_normalization_collapses_comments_and_space() {
        assert_eq!(
            normalize_sql("SELECT\n  -- note\n  coalesce(a, b)"),
            "select coalesce(a, b)"
        );
        assert_eq!(
            normalize_sql("identity_data->>'email'"),
            "identity_data->>'email'"
        );
        assert_ne!(
            normalize_sql("identity_data->>'EMAIL'"),
            normalize_sql("identity_data->>'email'")
        );
        assert_eq!(
            normalize_sql("WHERE name = 'O''Brien'"),
            "where name = 'O''Brien'"
        );
    }

    #[test]
    fn is_ident_rejects_injection() {
        assert!(is_ident("users"));
        assert!(is_ident("_todo"));
        assert!(!is_ident(""));
        assert!(!is_ident("1users"));
        assert!(!is_ident("users;"));
    }

    #[test]
    fn absent_matches_only_when_both_sides_lack_the_object() {
        assert_eq!(
            absent_mismatch(false, false, "table", "auth.sso_sessions"),
            None
        );
        assert!(absent_mismatch(true, false, "table", "auth.sso_sessions")
            .unwrap()
            .contains("reference"));
        assert!(absent_mismatch(false, true, "table", "auth.sso_sessions")
            .unwrap()
            .contains("megabase"));
        assert!(absent_mismatch(true, true, "table", "auth.sso_sessions")
            .unwrap()
            .contains("both"));
    }
}
