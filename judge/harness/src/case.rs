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
        let parsed: CaseFile =
            toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
        cases.extend(parsed.case);
    }
    let mut seen = std::collections::BTreeSet::new();
    for case in &cases {
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
