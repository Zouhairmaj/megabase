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
