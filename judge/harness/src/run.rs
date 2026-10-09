//! Sends each case to both stacks and compares the normalized responses.

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::case::{toml_to_json, Case, Key, Step};
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
) -> Result<Outcome> {
    let agent = agent();
    let base_vars: BTreeMap<String, String> = [("run".to_string(), run_id.to_string())].into();
    let mut ref_vars = base_vars.clone();
    let mut mb_vars = base_vars;
    for (i, step) in case.step.iter().enumerate() {
        let r = send(&agent, reference, step, keys, &ref_vars)?;
        let m = send(&agent, megabase, step, keys, &mb_vars)?;
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
    Ok(Outcome {
        id: case.id.clone(),
        description: case.description.clone(),
        pass: true,
        detail: None,
    })
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
    }
}
