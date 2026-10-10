//! Case files: `judge/cases/*.toml`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseFile {
    #[serde(default)]
    pub case: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    /// `coverage/units.json` ids this case exercises.
    pub units: Vec<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub step: Vec<Step>,
    /// Catalog / row checks independent of HTTP. A case may be db-only.
    #[serde(default)]
    pub db: Vec<DbCheck>,
    /// Tables snapshotted after HTTP steps. `None` means the default
    /// (`auth.users` and `public.todos`) when the case mutates. `[]`
    /// disables the default. Storage objects are Level 2.
    pub snapshot: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DbCheck {
    pub table: Option<String>,
    pub function: Option<String>,
    /// Also compare every row of `table`. Ignored for functions.
    #[serde(default)]
    pub rows: bool,
    /// The object must be missing on both databases (dropped on the pin).
    #[serde(default)]
    pub absent: bool,
}

impl DbCheck {
    pub fn kind(&self) -> Result<DbKind<'_>> {
        match (self.table.as_deref(), self.function.as_deref()) {
            (Some(table), None) => Ok(DbKind::Table {
                name: table,
                rows: self.rows,
                absent: self.absent,
            }),
            (None, Some(function)) => Ok(DbKind::Function {
                name: function,
                absent: self.absent,
            }),
            (Some(_), Some(_)) => bail!("a db check lists both `table` and `function`"),
            (None, None) => bail!("a db check lists neither `table` nor `function`"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbKind<'a> {
    Table {
        name: &'a str,
        rows: bool,
        absent: bool,
    },
    Function {
        name: &'a str,
        absent: bool,
    },
}

pub fn is_mutating_method(method: &str) -> bool {
    matches!(
        method.to_ascii_uppercase().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    )
}

impl Case {
    /// Relations whose rows are snapshotted after the HTTP steps.
    pub fn snapshot_relations(&self) -> Vec<String> {
        match &self.snapshot {
            Some(list) => list.clone(),
            None if self.step.iter().any(|s| is_mutating_method(&s.method)) => {
                vec!["auth.users".into(), "public.todos".into()]
            }
            None => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    #[default]
    Anon,
    ServiceRole,
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub key: Key,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// JSON request body, written as TOML.
    pub json: Option<toml::Value>,
    /// Raw request body.
    pub body: Option<String>,
    /// Extra response headers to compare on top of the defaults.
    #[serde(default)]
    pub compare_headers: Vec<String>,
    /// JSON pointers removed from both bodies before comparing.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// `name = "/json/pointer"`: store a value from each stack's own response
    /// for use as `{{name}}` in later steps.
    #[serde(default)]
    pub capture: BTreeMap<String, String>,
}

/// Parses one case file. `origin` is used only in the error.
///
/// # Errors
///
/// Returns an error when `text` is not the case-file schema.
pub fn parse_toml(text: &str, origin: &str) -> Result<Vec<Case>> {
    let parsed: CaseFile = toml::from_str(text).with_context(|| format!("parsing {origin}"))?;
    Ok(parsed.case)
}

/// Checks ids, units, and steps. Does not sort.
///
/// # Errors
///
/// Returns an error when a case is empty, duplicated, or internally inconsistent.
pub fn validate(cases: &[Case]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for case in cases {
        if !seen.insert(case.id.as_str()) {
            bail!("duplicate case id `{}`", case.id);
        }
        if case.units.is_empty() {
            bail!("case `{}` lists no units", case.id);
        }
        if case.step.is_empty() && case.db.is_empty() {
            bail!("case `{}` has no steps and no db checks", case.id);
        }
        for (i, check) in case.db.iter().enumerate() {
            check
                .kind()
                .with_context(|| format!("case `{}` db check {}", case.id, i + 1))?;
        }
        for step in &case.step {
            if step.json.is_some() && step.body.is_some() {
                bail!("case `{}`: a step has both `json` and `body`", case.id);
            }
        }
    }
    Ok(())
}

pub fn load(dir: &Path) -> Result<Vec<Case>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)?;
        cases.extend(parse_toml(&text, &file.display().to_string())?);
    }
    validate(&cases)?;
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cases)
}

pub fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Value::from(*f),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(a) => serde_json::Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => serde_json::Value::Object(
            t.iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_cases() -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "megabase-judge-cases-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn load_parses_steps_and_sorts_by_id() {
        let dir = temp_cases();
        fs::write(
            dir.join("b.toml"),
            r#"
[[case]]
id = "z.last"
units = ["rest:route:GET /rest/v1/{relation}"]
description = "later"
[[case.step]]
method = "GET"
path = "/rest/v1/todos"
key = "none"
"#,
        )
        .unwrap();
        fs::write(
            dir.join("a.toml"),
            r#"
[[case]]
id = "a.first"
units = ["auth:route:POST /auth/v1/signup"]
[[case.step]]
method = "POST"
path = "/auth/v1/signup"
key = "service_role"
headers = { Prefer = "return=representation" }
json = { email = "a@example.com", n = 1, ok = true, tags = ["x"] }
compare_headers = ["Retry-After"]
ignore = ["/id"]
capture = { user = "/id" }
"#,
        )
        .unwrap();
        let cases = load(&dir).unwrap();
        assert_eq!(
            cases.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["a.first", "z.last"]
        );
        assert_eq!(cases[0].step[0].key, Key::ServiceRole);
        assert_eq!(cases[1].step[0].key, Key::None);
        assert_eq!(cases[0].step[0].headers["Prefer"], "return=representation");
        let json = toml_to_json(cases[0].step[0].json.as_ref().unwrap());
        assert_eq!(json["email"], "a@example.com");
        assert_eq!(json["n"], 1);
        assert_eq!(json["ok"], true);
        assert_eq!(json["tags"], serde_json::json!(["x"]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rejects_duplicates_empty_units_steps_and_json_plus_body() {
        let dir = temp_cases();
        fs::write(
            dir.join("dup.toml"),
            r#"
[[case]]
id = "same"
units = ["u"]
[[case.step]]
method = "GET"
path = "/"
[[case]]
id = "same"
units = ["u"]
[[case.step]]
method = "GET"
path = "/"
"#,
        )
        .unwrap();
        assert!(load(&dir)
            .unwrap_err()
            .to_string()
            .contains("duplicate case id"));
        let _ = fs::remove_dir_all(&dir);

        let dir = temp_cases();
        fs::write(
            dir.join("empty-units.toml"),
            r#"
[[case]]
id = "x"
units = []
[[case.step]]
method = "GET"
path = "/"
"#,
        )
        .unwrap();
        assert!(load(&dir)
            .unwrap_err()
            .to_string()
            .contains("lists no units"));
        let _ = fs::remove_dir_all(&dir);

        let dir = temp_cases();
        fs::write(
            dir.join("no-steps.toml"),
            r#"
[[case]]
id = "x"
units = ["u"]
step = []
"#,
        )
        .unwrap();
        assert!(load(&dir).unwrap_err().to_string().contains("has no steps"));
        let _ = fs::remove_dir_all(&dir);

        let dir = temp_cases();
        fs::write(
            dir.join("both.toml"),
            r#"
[[case]]
id = "x"
units = ["u"]
[[case.step]]
method = "POST"
path = "/"
json = { a = 1 }
body = "raw"
"#,
        )
        .unwrap();
        assert!(load(&dir)
            .unwrap_err()
            .to_string()
            .contains("both `json` and `body`"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rejects_unknown_fields() {
        let dir = temp_cases();
        fs::write(
            dir.join("extra.toml"),
            r#"
[[case]]
id = "x"
units = ["u"]
surprise = true
[[case.step]]
method = "GET"
path = "/"
"#,
        )
        .unwrap();
        assert!(load(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn toml_to_json_covers_scalars_arrays_tables_and_datetime() {
        let value: toml::Value = toml::from_str(
            r#"
s = "hi"
i = 2
f = 1.5
b = false
when = 1979-05-27T07:32:00Z
nested = { k = "v" }
list = [1, "a"]
"#,
        )
        .unwrap();
        let json = toml_to_json(&value);
        assert_eq!(json["s"], "hi");
        assert_eq!(json["i"], 2);
        assert_eq!(json["f"], 1.5);
        assert_eq!(json["b"], false);
        assert_eq!(json["when"], "1979-05-27T07:32:00Z");
        assert_eq!(json["nested"]["k"], "v");
        assert_eq!(json["list"], serde_json::json!([1, "a"]));
    }

    fn parse_one(text: &str) -> Case {
        let parsed: CaseFile = toml::from_str(text).unwrap();
        parsed.case.into_iter().next().unwrap()
    }

    #[test]
    fn mutating_cases_snapshot_auth_users_and_fixtures() {
        let case = parse_one(
            r#"
[[case]]
id = "signup"
units = ["auth:route:POST /auth/v1/signup"]
[[case.step]]
method = "POST"
path = "/auth/v1/signup"
"#,
        );
        assert_eq!(case.snapshot_relations(), ["auth.users", "public.todos"]);
    }

    #[test]
    fn get_cases_have_no_default_snapshot() {
        let case = parse_one(
            r#"
[[case]]
id = "health"
units = ["auth:route:GET /auth/v1/health"]
[[case.step]]
method = "GET"
path = "/auth/v1/health"
"#,
        );
        assert!(case.snapshot_relations().is_empty());
    }

    #[test]
    fn empty_snapshot_disables_the_default() {
        let case = parse_one(
            r#"
[[case]]
id = "signup"
units = ["auth:route:POST /auth/v1/signup"]
snapshot = []
[[case.step]]
method = "POST"
path = "/auth/v1/signup"
"#,
        );
        assert!(case.snapshot_relations().is_empty());
    }

    #[test]
    fn db_only_case_is_valid() {
        let case = parse_one(
            r#"
[[case]]
id = "uid"
units = ["auth:sql-function:auth.uid()"]
[[case.db]]
function = "auth.uid()"
"#,
        );
        assert!(case.step.is_empty());
        assert_eq!(
            case.db[0].kind().unwrap(),
            DbKind::Function {
                name: "auth.uid()",
                absent: false
            }
        );
    }

    #[test]
    fn db_check_rejects_both_fields() {
        let check = DbCheck {
            table: Some("auth.users".into()),
            function: Some("auth.uid()".into()),
            rows: false,
            absent: false,
        };
        assert!(check.kind().is_err());
    }

    #[test]
    fn absent_table_check_parses() {
        let case = parse_one(
            r#"
[[case]]
id = "sso"
units = ["auth:sql-table:auth.sso_sessions"]
[[case.db]]
table = "auth.sso_sessions"
absent = true
"#,
        );
        assert_eq!(
            case.db[0].kind().unwrap(),
            DbKind::Table {
                name: "auth.sso_sessions",
                rows: false,
                absent: true
            }
        );
    }

    #[test]
    fn repo_case_files_load() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../cases");
        let cases = load(&root).expect("judge/cases should parse");
        assert!(cases.iter().any(|c| c.id == "auth.sql.function.uid"));
        assert!(cases.iter().any(|c| c.id == "auth.signup.password"
            && c.snapshot_relations() == ["auth.users", "public.todos"]));
        assert!(cases
            .iter()
            .any(|c| c.id == "auth.sql.table.users" && c.step.is_empty() && !c.db.is_empty()));
        let sso = cases
            .iter()
            .find(|c| c.id == "auth.sql.table.sso_sessions")
            .expect("sso_sessions case");
        assert_eq!(
            sso.db[0].kind().unwrap(),
            DbKind::Table {
                name: "auth.sso_sessions",
                rows: false,
                absent: true
            }
        );
    }
}
