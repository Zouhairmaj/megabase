//! Sends each case to both stacks and compares the normalized responses.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::{toml_to_json, Case, DbKind, Key, Step};
use crate::db::{self, Databases};
use crate::normalize;

pub struct Keys {
    pub anon: String,
    pub service_role: String,
}

pub struct Target {
    pub name: &'static str,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Observed {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Body,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Empty,
    Json(Value),
    Text(String),
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Results {
    pub schema: u32,
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct CaseResult {
    pub id: String,
    pub pass: bool,
}

pub struct Outcome {
    pub id: String,
    pub description: String,
    pub pass: bool,
    /// First difference found, for humans; never written to the results file.
    pub detail: Option<String>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirects(0)
        .build()
}

fn substitute(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (name, value) in vars {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    out
}

fn substitute_json(value: &mut Value, vars: &BTreeMap<String, String>) {
    match value {
        Value::String(s) => *s = substitute(s, vars),
        Value::Array(items) => items.iter_mut().for_each(|v| substitute_json(v, vars)),
        Value::Object(map) => map.values_mut().for_each(|v| substitute_json(v, vars)),
        _ => {}
    }
}

fn send(
    agent: &ureq::Agent,
    target: &Target,
    step: &Step,
    keys: &Keys,
    vars: &BTreeMap<String, String>,
) -> Result<Observed> {
    let url = format!("{}{}", target.base, substitute(&step.path, vars));
    let mut request = agent.request(&step.method, &url);
    let key = match step.key {
        Key::Anon => Some(&keys.anon),
        Key::ServiceRole => Some(&keys.service_role),
        Key::None => None,
    };
    if let Some(key) = key {
        request = request
            .set("apikey", key)
            .set("Authorization", &format!("Bearer {key}"));
    }
    for (name, value) in &step.headers {
        request = request.set(name, &substitute(value, vars));
    }
    let result = if let Some(json) = &step.json {
        let mut body = toml_to_json(json);
        substitute_json(&mut body, vars);
        request
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
    } else if let Some(body) = &step.body {
        request.send_string(&substitute(body, vars))
    } else {
        request.call()
    };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(err) => {
            return Err(err).with_context(|| format!("{} {} on {}", step.method, url, target.name))
        }
    };
    let status = response.status();
    let mut headers = BTreeMap::new();
    for name in normalize::DEFAULT_HEADERS
        .iter()
        .map(|s| s.to_string())
        .chain(step.compare_headers.iter().map(|s| s.to_ascii_lowercase()))
    {
        if let Some(value) = response.header(&name) {
            headers.insert(name.clone(), normalize::header(&name, value));
        }
    }
    let is_json = response.content_type().contains("json");
    let text = response.into_string().context("reading response body")?;
    let body = if text.is_empty() {
        Body::Empty
    } else if is_json {
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => Body::Json(value),
            Err(_) => Body::Text(text),
        }
    } else {
        Body::Text(text)
    };
    Ok(Observed {
        status,
        headers,
        body,
    })
}

fn capture(observed: &Observed, step: &Step, vars: &mut BTreeMap<String, String>) -> Result<()> {
    for (name, pointer) in &step.capture {
        let Body::Json(value) = &observed.body else {
            bail!("cannot capture `{name}`: response body is not JSON");
        };
        let found = value
            .pointer(pointer)
            .with_context(|| format!("cannot capture `{name}`: `{pointer}` not in response"))?;
        let text = match found {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        vars.insert(name.clone(), text);
    }
    Ok(())
}

fn normalized(mut observed: Observed, step: &Step) -> Observed {
    observed.body = match observed.body {
        Body::Json(mut value) => {
            normalize::json(&mut value, &step.ignore);
            Body::Json(value)
        }
        Body::Text(text) => Body::Text(normalize::text(&text)),
        Body::Empty => Body::Empty,
    };
    observed
}

fn describe(reference: &Observed, megabase: &Observed) -> String {
    let show = |b: &Body| -> String {
        let s = match b {
            Body::Empty => "<empty>".to_string(),
            Body::Json(v) => v.to_string(),
            Body::Text(t) => t.clone(),
        };
        if s.chars().count() > 300 {
            format!("{}…", s.chars().take(300).collect::<String>())
        } else {
            s
        }
    };
    if reference.status != megabase.status {
        format!(
            "status {} (reference) vs {} (megabase); megabase body: {}",
            reference.status,
            megabase.status,
            show(&megabase.body)
        )
    } else if reference.headers != megabase.headers {
        format!(
            "headers {:?} (reference) vs {:?} (megabase)",
            reference.headers, megabase.headers
        )
    } else {
        format!(
            "body {} (reference) vs {} (megabase)",
            show(&reference.body),
            show(&megabase.body)
        )
    }
}

pub fn run_case(
    case: &Case,
    reference: &Target,
    megabase: &Target,
    keys: &Keys,
    run_id: &str,
    databases: &Databases,
) -> Result<Outcome> {
    let agent = agent();
    let base_vars: BTreeMap<String, String> = [("run".to_string(), run_id.to_string())].into();
    let mut ref_vars = base_vars.clone();
    let mut mb_vars = base_vars;
    let snapshot_rels = case.snapshot_relations();
    let before = if snapshot_rels.is_empty() {
        None
    } else {
        Some(db::snapshot_relations(databases, &snapshot_rels)?)
    };
    for (i, step) in case.step.iter().enumerate() {
        // Reference-stack `send`/`capture` errors abort the run on purpose:
        // an unreachable reference stack is an environment failure, not a
        // Megabase regression, so the harness must not write a results file
        // or treat a baseline pass as a case failure.
        let r = send(&agent, reference, step, keys, &ref_vars)?;
        let m = match send(&agent, megabase, step, keys, &mb_vars) {
            Ok(observed) => observed,
            Err(err) => {
                return Ok(Outcome {
                    id: case.id.clone(),
                    description: case.description.clone(),
                    pass: false,
                    detail: Some(format!(
                        "step {} ({} {}): megabase transport error: {err:#}",
                        i + 1,
                        step.method,
                        step.path
                    )),
                });
            }
        };
        capture(&r, step, &mut ref_vars)
            .with_context(|| format!("case `{}` step {} on the reference stack", case.id, i + 1))?;
        let megabase_captured = capture(&m, step, &mut mb_vars);
        let (r, m) = (normalized(r, step), normalized(m, step));
        if r != m {
            return Ok(Outcome {
                id: case.id.clone(),
                description: case.description.clone(),
                pass: false,
                detail: Some(format!(
                    "step {} ({} {}): {}",
                    i + 1,
                    step.method,
                    step.path,
                    describe(&r, &m)
                )),
            });
        }
        if let Err(err) = megabase_captured {
            return Ok(Outcome {
                id: case.id.clone(),
                description: case.description.clone(),
                pass: false,
                detail: Some(format!("{err:#}")),
            });
        }
    }
    if let Some(detail) = compare_side_effects(case, databases, before.as_deref())? {
        return Ok(Outcome {
            id: case.id.clone(),
            description: case.description.clone(),
            pass: false,
            detail: Some(detail),
        });
    }
    Ok(Outcome {
        id: case.id.clone(),
        description: case.description.clone(),
        pass: true,
        detail: None,
    })
}

fn compare_side_effects(
    case: &Case,
    databases: &Databases,
    before: Option<&[db::RelationSnapshot]>,
) -> Result<Option<String>> {
    for (i, check) in case.db.iter().enumerate() {
        let detail = match check.kind()? {
            DbKind::Table { name, rows, absent } => {
                db::compare_table(databases, name, rows, absent)?
            }
            DbKind::Function { name, absent } => db::compare_function(databases, name, absent)?,
        };
        if let Some(detail) = detail {
            return Ok(Some(format!("db check {}: {detail}", i + 1)));
        }
    }
    if let Some(before) = before {
        let raws: Vec<String> = before.iter().map(|s| s.raw.clone()).collect();
        let after = db::snapshot_relations(databases, &raws)?;
        if let Some(detail) = db::compare_snapshot_deltas(before, &after)? {
            return Ok(Some(format!("snapshot: {detail}")));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_variables() {
        let vars: BTreeMap<String, String> = [("run".to_string(), "r1".to_string())].into();
        assert_eq!(
            substitute("judge-{{run}}@example.com", &vars),
            "judge-r1@example.com"
        );
        let mut v = serde_json::json!({"email": "a-{{run}}", "n": [1, "{{run}}"]});
        substitute_json(&mut v, &vars);
        assert_eq!(v, serde_json::json!({"email": "a-r1", "n": [1, "r1"]}));
        let mut num = serde_json::json!(3);
        substitute_json(&mut num, &vars);
        assert_eq!(num, serde_json::json!(3));
    }

    fn json_observed(status: u16, body: serde_json::Value) -> Observed {
        Observed {
            status,
            headers: BTreeMap::new(),
            body: Body::Json(body),
        }
    }

    fn step_with_capture(pointer: &str) -> Step {
        Step {
            method: "GET".into(),
            path: "/".into(),
            key: Key::Anon,
            headers: BTreeMap::new(),
            json: None,
            body: None,
            compare_headers: vec![],
            ignore: vec!["/volatile".into()],
            capture: [("id".into(), pointer.into())].into(),
        }
    }

    #[test]
    fn capture_reads_json_strings_and_other_types() {
        let step = step_with_capture("/id");
        let mut vars = BTreeMap::new();
        capture(
            &json_observed(200, serde_json::json!({"id": "abc"})),
            &step,
            &mut vars,
        )
        .unwrap();
        assert_eq!(vars["id"], "abc");
        vars.clear();
        capture(
            &json_observed(200, serde_json::json!({"id": 7})),
            &step,
            &mut vars,
        )
        .unwrap();
        assert_eq!(vars["id"], "7");
    }

    #[test]
    fn capture_requires_json_body_and_pointer() {
        let step = step_with_capture("/id");
        let mut vars = BTreeMap::new();
        let text = Observed {
            status: 200,
            headers: BTreeMap::new(),
            body: Body::Text("nope".into()),
        };
        assert!(capture(&text, &step, &mut vars)
            .unwrap_err()
            .to_string()
            .contains("not JSON"));
        assert!(capture(
            &json_observed(200, serde_json::json!({"other": 1})),
            &step,
            &mut vars
        )
        .unwrap_err()
        .to_string()
        .contains("not in response"));
    }

    #[test]
    fn normalized_applies_ignore_and_text_rules() {
        let step = step_with_capture("/missing");
        let json = json_observed(
            200,
            serde_json::json!({"volatile": 1, "keep": "2026-10-09T00:00:00Z"}),
        );
        match normalized(json, &step).body {
            Body::Json(v) => {
                assert!(v.get("volatile").is_none());
                assert_eq!(v["keep"], "<timestamp>");
            }
            other => panic!("{other:?}"),
        }
        let text = Observed {
            status: 200,
            headers: BTreeMap::new(),
            body: Body::Text("id 0b6a1f8e-3f43-4b5e-9c1d-2a0e5f6b7c8d".into()),
        };
        match normalized(text, &step).body {
            Body::Text(t) => assert_eq!(t, "id <uuid>"),
            other => panic!("{other:?}"),
        }
        let empty = Observed {
            status: 204,
            headers: BTreeMap::new(),
            body: Body::Empty,
        };
        assert_eq!(normalized(empty, &step).body, Body::Empty);
    }

    #[test]
    fn describe_reports_status_headers_body_and_truncates() {
        let reference = json_observed(200, serde_json::json!({"ok": true}));
        let megabase = json_observed(501, serde_json::json!({"code": "MEGABASE_NOT_IMPLEMENTED"}));
        let msg = describe(&reference, &megabase);
        assert!(msg.contains("status 200"));
        assert!(msg.contains("501"));

        let mut r = reference;
        r.headers
            .insert("content-type".into(), "application/json".into());
        let mut m = r.clone();
        m.headers.insert("content-type".into(), "text/plain".into());
        assert!(describe(&r, &m).contains("headers"));

        let long = "x".repeat(400);
        let r = Observed {
            status: 200,
            headers: BTreeMap::new(),
            body: Body::Text(long.clone()),
        };
        let m = Observed {
            status: 200,
            headers: BTreeMap::new(),
            body: Body::Empty,
        };
        let msg = describe(&r, &m);
        assert!(msg.contains("…"));
        assert!(msg.contains("<empty>"));
        assert!(msg.chars().count() < long.len());
    }

    #[test]
    fn observed_equality_is_the_response_snapshot() {
        let a = json_observed(
            200,
            serde_json::json!({"users": [{"id": "<uuid>", "email": "a@example.com"}]}),
        );
        let b = a.clone();
        assert_eq!(a, b);
        let mut c = a.clone();
        c.body = Body::Json(serde_json::json!({"users": []}));
        assert_ne!(a, c);
    }
}
