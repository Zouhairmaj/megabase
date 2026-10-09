//! Per-unit state, derived from three independent sources:
//!
//! - **implemented**: a `megabase:unit <id>` marker in `crates/` source,
//!   placed on the code that serves the unit;
//! - **tested**: implemented, and at least one judge case in `judge/cases/`
//!   lists the unit;
//! - **conformant**: tested, and every judge case listing the unit passed in
//!   the last judge run (`coverage/judge-results.json`).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{UnitsFile, COMPONENTS};

pub const MARKER: &str = "megabase:unit ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Missing,
    Implemented,
    Tested,
    Conformant,
}

#[derive(Debug, Deserialize)]
pub struct CaseFile {
    #[serde(default)]
    pub case: Vec<CaseRef>,
}

#[derive(Debug, Deserialize)]
pub struct CaseRef {
    pub id: String,
    #[serde(default)]
    pub units: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct JudgeResults {
    pub schema: u32,
    #[serde(default)]
    pub cases: Vec<CaseResult>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct CaseResult {
    pub id: String,
    pub pass: bool,
}

#[derive(Debug, Default, Clone, Copy, Serialize, PartialEq)]
pub struct Counts {
    pub units: usize,
    pub implemented: usize,
    pub tested: usize,
    pub conformant: usize,
}

impl Counts {
    pub fn add(&mut self, state: State) {
        self.units += 1;
        self.implemented += usize::from(state >= State::Implemented);
        self.tested += usize::from(state >= State::Tested);
        self.conformant += usize::from(state >= State::Conformant);
    }
}

#[derive(Debug)]
pub struct Status {
    pub states: BTreeMap<String, State>,
    pub cases_total: usize,
    pub cases_passing: usize,
}

impl Status {
    pub fn state(&self, id: &str) -> State {
        self.states.get(id).copied().unwrap_or(State::Missing)
    }
}

pub fn compute(root: &Path, units: &UnitsFile) -> Result<Status> {
    let known: HashSet<&str> = units.units.iter().map(|u| u.id.as_str()).collect();
    let implemented = markers(root, &known)?;

    let mut cases_by_unit: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut case_ids = BTreeSet::new();
    let cases_dir = root.join("judge/cases");
    if cases_dir.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&cases_dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "toml"))
            .collect();
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)?;
            let parsed: CaseFile =
                toml::from_str(&text).with_context(|| format!("parsing {}", file.display()))?;
            for case in parsed.case {
                if !case_ids.insert(case.id.clone()) {
                    bail!("duplicate judge case id {}", case.id);
                }
                for unit in case.units {
                    if !known.contains(unit.as_str()) {
                        bail!("judge case {} lists unknown unit `{unit}`", case.id);
                    }
                    cases_by_unit.entry(unit).or_default().push(case.id.clone());
                }
            }
        }
    }

    let results_path = root.join("coverage/judge-results.json");
    let results: JudgeResults = if results_path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&results_path)?)
            .context("parsing coverage/judge-results.json")?
    } else {
        JudgeResults::default()
    };
    let passed: BTreeMap<&str, bool> = results
        .cases
        .iter()
        .filter(|c| case_ids.contains(&c.id))
        .map(|c| (c.id.as_str(), c.pass))
        .collect();

    let mut states = BTreeMap::new();
    for id in implemented {
        let cases = cases_by_unit.get(&id);
        let state = match cases {
            None => State::Implemented,
            Some(cases) if cases.iter().all(|c| passed.get(c.as_str()) == Some(&true)) => {
                State::Conformant
            }
            Some(_) => State::Tested,
        };
        states.insert(id, state);
    }
    Ok(Status {
        states,
        cases_total: case_ids.len(),
        cases_passing: passed.values().filter(|p| **p).count(),
    })
}

/// Unit ids claimed by `megabase:unit` markers under `crates/`.
fn markers(root: &Path, known: &HashSet<&str>) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&path)?;
                for line in text.lines() {
                    let Some(pos) = line.find(MARKER) else {
                        continue;
                    };
                    let id = line[pos + MARKER.len()..].trim();
                    if !known.contains(id) {
                        bail!("{}: marker for unknown unit `{id}`", path.display());
                    }
                    found.insert(id.to_string());
                }
            }
        }
    }
    Ok(found)
}

#[derive(Serialize)]
pub struct ComponentSummary {
    #[serde(flatten)]
    pub counts: Counts,
    pub groups: BTreeMap<String, Counts>,
}

#[derive(Serialize)]
pub struct Summary {
    pub schema: u32,
    pub totals: Counts,
    pub percent: Percent,
    pub judge: JudgeSummary,
    pub components: BTreeMap<String, ComponentSummary>,
    pub units: BTreeMap<String, State>,
}

#[derive(Serialize)]
pub struct Percent {
    /// implemented / all units
    pub coverage: f64,
    /// passing judge cases / all judge cases
    pub conformance: f64,
    /// conformant / all units
    pub done: f64,
}

#[derive(Serialize)]
pub struct JudgeSummary {
    pub cases: usize,
    pub passing: usize,
}

pub fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 * 1000.0 / whole as f64).round() / 10.0
    }
}

pub fn summarize(units: &UnitsFile, status: &Status) -> Summary {
    let mut totals = Counts::default();
    let mut components: BTreeMap<String, ComponentSummary> = COMPONENTS
        .iter()
        .map(|(id, _)| {
            (
                id.to_string(),
                ComponentSummary {
                    counts: Counts::default(),
                    groups: BTreeMap::new(),
                },
            )
        })
        .collect();
    for unit in &units.units {
        let state = status.state(&unit.id);
        totals.add(state);
        let component = components
            .get_mut(&unit.component)
            .expect("known component");
        component.counts.add(state);
        component
            .groups
            .entry(unit.group.clone())
            .or_default()
            .add(state);
    }
    Summary {
        schema: 1,
        totals,
        percent: Percent {
            coverage: pct(totals.implemented, totals.units),
            conformance: pct(status.cases_passing, status.cases_total),
            done: pct(totals.conformant, totals.units),
        },
        judge: JudgeSummary {
            cases: status.cases_total,
            passing: status.cases_passing,
        },
        components,
        units: status.states.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Collector, Pin, Source, UnitSpec};
    use crate::scan::TempTree;

    fn src() -> Source {
        Source {
            repo: "postgrest".into(),
            file: "a.hs".into(),
            line: 1,
        }
    }

    fn units_file() -> UnitsFile {
        let mut c = Collector::default();
        c.route("rest", "resources", "GET", "/rest/v1/todos", 1, src());
        c.route("rest", "resources", "POST", "/rest/v1/todos", 1, src());
        c.item(UnitSpec {
            component: "auth",
            group: "token",
            kind: "grant-type",
            name: "password".into(),
            level: 1,
            source: src(),
        });
        c.finish(vec![Pin {
            name: "postgrest".into(),
            path: "vendor/postgrest".into(),
            repo: "https://example".into(),
            tag: "v1".into(),
            commit: "abc".into(),
            license: "MIT".into(),
            image: Some("img".into()),
        }])
    }

    #[test]
    fn pct_rounds_tenth_and_zero_whole() {
        assert_eq!(pct(0, 0), 0.0);
        assert_eq!(pct(1, 3), 33.3);
        assert_eq!(pct(1, 2), 50.0);
    }

    #[test]
    fn counts_add_tracks_state_ranks() {
        let mut c = Counts::default();
        c.add(State::Missing);
        c.add(State::Implemented);
        c.add(State::Tested);
        c.add(State::Conformant);
        assert_eq!(c.units, 4);
        assert_eq!(c.implemented, 3);
        assert_eq!(c.tested, 2);
        assert_eq!(c.conformant, 1);
    }

    #[test]
    fn compute_implemented_tested_conformant_and_errors() {
        let units = units_file();
        let get_id = "rest:route:GET /rest/v1/todos";
        let post_id = "rest:route:POST /rest/v1/todos";
        let grant_id = "auth:grant-type:password";

        let tree = TempTree::new();
        tree.write(
            "crates/demo/src/lib.rs",
            &format!("// megabase:unit {get_id}\n// megabase:unit {post_id}\n// megabase:unit {grant_id}\n"),
        );
        tree.write(
            "judge/cases/rest.toml",
            r#"
[[case]]
id = "rest.select"
units = ["rest:route:GET /rest/v1/todos"]
[[case]]
id = "rest.insert"
units = ["rest:route:POST /rest/v1/todos"]
"#,
        );
        tree.write(
            "coverage/judge-results.json",
            r#"{"schema":1,"cases":[{"id":"rest.select","pass":true},{"id":"rest.insert","pass":false},{"id":"stale","pass":true}]}"#,
        );

        let status = compute(&tree.root, &units).unwrap();
        assert_eq!(status.state(get_id), State::Conformant);
        assert_eq!(status.state(post_id), State::Tested);
        assert_eq!(status.state(grant_id), State::Implemented);
        assert_eq!(status.state("missing"), State::Missing);
        assert_eq!(status.cases_total, 2);
        assert_eq!(status.cases_passing, 1);

        let summary = summarize(&units, &status);
        assert_eq!(summary.totals.units, 3);
        assert_eq!(summary.totals.implemented, 3);
        assert_eq!(summary.totals.tested, 2);
        assert_eq!(summary.totals.conformant, 1);
        assert_eq!(summary.percent.coverage, 100.0);
        assert_eq!(summary.percent.conformance, 50.0);
        assert!(summary.percent.done > 0.0);

        tree.write(
            "crates/demo/src/bad.rs",
            "// megabase:unit rest:route:GET /nope\n",
        );
        let err = compute(&tree.root, &units).unwrap_err().to_string();
        assert!(err.contains("unknown unit"), "{err}");
    }

    #[test]
    fn compute_rejects_duplicate_cases_and_unknown_unit_refs() {
        let units = units_file();
        let tree = TempTree::new();
        tree.write("crates/demo/src/lib.rs", "fn x() {}\n");
        tree.write(
            "judge/cases/a.toml",
            r#"
[[case]]
id = "dup"
units = ["rest:route:GET /rest/v1/todos"]
"#,
        );
        tree.write(
            "judge/cases/b.toml",
            r#"
[[case]]
id = "dup"
units = ["rest:route:GET /rest/v1/todos"]
"#,
        );
        let err = compute(&tree.root, &units).unwrap_err().to_string();
        assert!(err.contains("duplicate judge case"), "{err}");

        let tree = TempTree::new();
        tree.write("crates/demo/src/lib.rs", "fn x() {}\n");
        tree.write(
            "judge/cases/a.toml",
            r#"
[[case]]
id = "x"
units = ["rest:nope"]
"#,
        );
        let err = compute(&tree.root, &units).unwrap_err().to_string();
        assert!(err.contains("unknown unit"), "{err}");
    }

    #[test]
    fn compute_without_cases_or_results_is_implemented_only() {
        let units = units_file();
        let tree = TempTree::new();
        tree.write(
            "crates/demo/src/lib.rs",
            "// megabase:unit rest:route:GET /rest/v1/todos\n",
        );
        let status = compute(&tree.root, &units).unwrap();
        assert_eq!(
            status.state("rest:route:GET /rest/v1/todos"),
            State::Implemented
        );
        assert_eq!(status.cases_total, 0);
    }
}
