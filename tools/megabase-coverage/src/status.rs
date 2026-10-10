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

/// Whether the Judge artifact may be applied onto the default-branch checkout.
///
/// The Judge commit is never checked out. `compare_status` is the GitHub
/// compare status of `judge_sha...checkout_sha`: `ahead` means the checkout
/// contains the Judge commit (main moved while Judge ran), `identical` means
/// the same commit. Any other status leaves the published summary unchanged.
/// `pages-badges.yml` inlines this predicate; the workflow test below keeps
/// the two copies aligned.
#[cfg(test)]
fn judge_results_apply(checkout_sha: &str, judge_sha: &str, compare_status: &str) -> bool {
    fn commit_id(sha: &str) -> bool {
        sha.len() == 40
            && sha
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
    if !commit_id(checkout_sha) || !commit_id(judge_sha) {
        return false;
    }
    if checkout_sha == judge_sha {
        return true;
    }
    matches!(compare_status, "ahead" | "identical")
}

pub fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        (part as f64 * 1000.0 / whole as f64).round() / 10.0
    }
}

/// Passing judge cases in one results file.
///
/// `None` when the file lists no cases. Callers must not substitute the
/// committed regression baseline for that absence (decision 0030).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Conformance {
    pub percent: f64,
    pub passing: usize,
    pub cases: usize,
}

impl Conformance {
    /// `percent passing cases`, the line release notes and history share.
    pub fn fields(&self) -> String {
        format!("{} {} {}", self.percent, self.passing, self.cases)
    }
}

pub fn conformance_of(results: &JudgeResults) -> Option<Conformance> {
    let cases = results.cases.len();
    if cases == 0 {
        return None;
    }
    let passing = results.cases.iter().filter(|c| c.pass).count();
    Some(Conformance {
        percent: pct(passing, cases),
        passing,
        cases,
    })
}

fn commit_sha(sha: &str) -> bool {
    sha.len() == 40
        && sha
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct JudgeHistory {
    schema: u32,
    runs: Vec<JudgeHistoryRun>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct JudgeHistoryRun {
    sha: String,
    passing: usize,
    cases: usize,
    percent: f64,
}

/// Insert or replace the measurement for `sha` in `coverage/judge-history.json`.
///
/// # Errors
///
/// Returns an error when `sha` is not a commit id, the history JSON is
/// unreadable or uses another schema, or `results` has no cases.
pub fn merge_judge_history(
    history_json: &str,
    sha: &str,
    results: &JudgeResults,
) -> Result<String> {
    if !commit_sha(sha) {
        bail!("judge history sha is not a commit id");
    }
    let measured = conformance_of(results).context("judge results have no cases")?;
    let mut history: JudgeHistory =
        serde_json::from_str(history_json).context("parsing judge history")?;
    if history.schema != 1 {
        bail!("unsupported judge history schema {}", history.schema);
    }
    let run = JudgeHistoryRun {
        sha: sha.to_string(),
        passing: measured.passing,
        cases: measured.cases,
        percent: measured.percent,
    };
    if let Some(existing) = history.runs.iter_mut().find(|r| r.sha == sha) {
        *existing = run;
    } else {
        history.runs.push(run);
    }
    let mut out = serde_json::to_string_pretty(&history)?;
    out.push('\n');
    Ok(out)
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
    fn unit_is_conformant_only_when_every_judge_case_passes() {
        let units = units_file();
        let get_id = "rest:route:GET /rest/v1/todos";
        let tree = TempTree::new();
        tree.write(
            "crates/demo/src/lib.rs",
            &format!("// megabase:unit {get_id}\n"),
        );
        tree.write(
            "judge/cases/rest.toml",
            &format!(
                r#"
[[case]]
id = "rest.select"
units = ["{get_id}"]
[[case]]
id = "rest.select.headers"
units = ["{get_id}"]
"#
            ),
        );

        tree.write(
            "coverage/judge-results.json",
            r#"{"schema":1,"cases":[{"id":"rest.select","pass":true},{"id":"rest.select.headers","pass":true}]}"#,
        );
        let status = compute(&tree.root, &units).unwrap();
        assert_eq!(status.state(get_id), State::Conformant);
        assert_eq!(status.cases_passing, 2);

        tree.write(
            "coverage/judge-results.json",
            r#"{"schema":1,"cases":[{"id":"rest.select","pass":true},{"id":"rest.select.headers","pass":false}]}"#,
        );
        let status = compute(&tree.root, &units).unwrap();
        assert_eq!(status.state(get_id), State::Tested);
        assert_eq!(status.cases_passing, 1);

        tree.write(
            "coverage/judge-results.json",
            r#"{"schema":1,"cases":[{"id":"rest.select","pass":true}]}"#,
        );
        let status = compute(&tree.root, &units).unwrap();
        assert_eq!(
            status.state(get_id),
            State::Tested,
            "a missing case result is not a pass"
        );
    }

    #[test]
    fn judge_results_apply_accepts_the_checkout_and_its_ancestors_only() {
        let head = "ff547511ed61f6b17661bdb1b98c131968d965df";
        let parent = "9f957e503eb6f9ad4f6e253162a6b2131e9ea120";
        assert!(judge_results_apply(head, head, "behind"));
        assert!(judge_results_apply(head, parent, "ahead"));
        assert!(judge_results_apply(head, parent, "identical"));
        assert!(!judge_results_apply(head, parent, "behind"));
        assert!(!judge_results_apply(head, parent, "diverged"));
        assert!(!judge_results_apply(head, parent, ""));
        assert!(!judge_results_apply(head, "9f957e5", "ahead"));
        assert!(!judge_results_apply(
            "FF547511ED61F6B17661BDB1B98C131968D965DF",
            "FF547511ED61F6B17661BDB1B98C131968D965DF",
            "identical"
        ));
    }

    #[test]
    fn pages_badges_keeps_judge_json_off_the_untrusted_checkout() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let badges = std::fs::read_to_string(root.join(".github/workflows/pages-badges.yml"))
            .expect("pages-badges.yml");
        let pages =
            std::fs::read_to_string(root.join(".github/workflows/pages.yml")).expect("pages.yml");
        assert!(
            !badges.contains("ref: ${{ github.event.workflow_run.head_sha }}"),
            "do not check out the triggering run"
        );
        assert!(
            !badges.contains("uses: ./.github/actions/rust-cache"),
            "the job that reads the Judge artifact must not write the default-branch cache"
        );
        assert!(badges.contains("judge_results_apply"));
        assert!(
            badges.contains("[[ ! \"$JUDGE_SHA\" =~ ^[0-9a-f]{40}$ ]]"),
            "workflow must reject a Judge SHA that is not a commit id"
        );
        assert!(
            badges.contains("[ \"$checked_out\" = \"$JUDGE_SHA\" ]"),
            "equal SHAs apply without the compare API"
        );
        assert!(badges.contains("[ \"$status\" = \"ahead\" ] || [ \"$status\" = \"identical\" ]"));
        assert!(badges.contains("actions/deploy-pages"));
        assert!(badges.contains("group: pages"));
        assert!(badges.contains("treemap.png"));
        assert!(
            badges.contains("judge-history"),
            "pages-badges must record the live score for release notes"
        );
        assert!(
            badges.contains(
                "api.github.com/repos/${GITHUB_REPOSITORY}/contents/coverage/judge-history.json?ref=gh-pages"
            ),
            "absence is a 404 from the gh-pages git tree, not the raw CDN"
        );
        assert!(
            badges.contains("[ \"$code\" = \"404\" ]"),
            "an empty history is written only when the contents API says the file is absent"
        );
        assert!(
            badges.contains("judge history download failed"),
            "any other fetch failure must fail the job before force_orphan"
        );
        assert!(
            !badges.contains("raw.githubusercontent.com"),
            "a raw CDN 404 must not be treated as a missing history file"
        );
        assert!(
            !badges.contains("if ! curl"),
            "a failed curl must not fall through to an empty history"
        );
        assert!(
            !pages.contains("judge-results"),
            "the cached Pages build must not download the Judge artifact"
        );
        assert!(
            !pages.contains("actions/deploy-pages"),
            "a push must not publish the baseline site over the live Judge score"
        );
        assert!(!pages.contains("github.event.workflow_run"));
    }

    #[test]
    fn conformance_fields_round_like_pct_and_history_replaces_one_sha() {
        let results = JudgeResults {
            schema: 1,
            cases: vec![
                CaseResult {
                    id: "a".into(),
                    pass: true,
                },
                CaseResult {
                    id: "b".into(),
                    pass: false,
                },
            ],
        };
        let measured = conformance_of(&results).unwrap();
        assert_eq!(measured.percent, 50.0);
        assert_eq!(measured.fields(), "50 1 2");
        assert!(conformance_of(&JudgeResults::default()).is_none());

        let sha = "ff547511ed61f6b17661bdb1b98c131968d965df";
        let other = "9f957e503eb6f9ad4f6e253162a6b2131e9ea120";
        let first = merge_judge_history(r#"{"schema":1,"runs":[]}"#, sha, &results).unwrap();
        let again = merge_judge_history(&first, sha, &results).unwrap();
        assert_eq!(again.matches(sha).count(), 1);
        let both = merge_judge_history(&again, other, &results).unwrap();
        assert!(both.contains(sha) && both.contains(other));
        assert!(merge_judge_history(&both, "FF547511", &results).is_err());
        assert!(merge_judge_history("{}", sha, &results).is_err());
        assert!(merge_judge_history(r#"{"schema":2,"runs":[]}"#, sha, &results).is_err());
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
