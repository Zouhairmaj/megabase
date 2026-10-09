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
    pub step: Vec<Step>,
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
        if case.step.is_empty() {
            bail!("case `{}` has no steps", case.id);
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
}
