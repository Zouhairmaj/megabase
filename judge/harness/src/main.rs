//! `megabase-judge`: differential tests between the official self-hosted
//! Supabase stack and Megabase. See `judge/README.md`.

mod case;
mod normalize;
mod run;

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use run::{CaseResult, Keys, Outcome, Results, Target};

const USAGE: &str = "\
usage:
  megabase-judge wait [--timeout SECS] [common]
  megabase-judge run [--cases DIR] [--out FILE] [--baseline FILE] [--summary FILE] [common]

common:
  --reference URL   reference gateway (default http://localhost:8000)
  --megabase URL    Megabase (default http://localhost:8100)
  --env FILE        env file with ANON_KEY and SERVICE_ROLE_KEY
                    (default vendor/supabase/docker/.env.example)";

struct Args {
    command: String,
    reference: String,
    megabase: String,
    env: PathBuf,
    cases: PathBuf,
    out: Option<PathBuf>,
    baseline: Option<PathBuf>,
    summary: Option<PathBuf>,
    timeout: u64,
}

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    let command = it.next().context(USAGE)?;
    let mut args = Args {
        command,
        reference: "http://localhost:8000".into(),
        megabase: "http://localhost:8100".into(),
        env: "vendor/supabase/docker/.env.example".into(),
        cases: "judge/cases".into(),
        out: None,
        baseline: None,
        summary: None,
        timeout: 300,
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--reference" => args.reference = value()?,
            "--megabase" => args.megabase = value()?,
            "--env" => args.env = value()?.into(),
            "--cases" => args.cases = value()?.into(),
            "--out" => args.out = Some(value()?.into()),
            "--baseline" => args.baseline = Some(value()?.into()),
            "--summary" => args.summary = Some(value()?.into()),
            "--timeout" => args.timeout = value()?.parse().context("--timeout")?,
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    Ok(args)
}

fn load_keys(path: &PathBuf) -> Result<Keys> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let get = |name: &str| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == name)
            .map(|(_, v)| v.trim().trim_matches('"').to_string())
            .filter(|v| !v.is_empty())
            .with_context(|| format!("{name} not set in {}", path.display()))
    };
    Ok(Keys {
        anon: get("ANON_KEY")?,
        service_role: get("SERVICE_ROLE_KEY")?,
    })
}

fn wait(args: &Args, keys: &Keys) -> Result<()> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    // The pinned Kong config serves the PostgREST OpenAPI root (`/rest/v1/`
    // exactly) to the admin consumer only, so that check needs the
    // service_role key; anon gets 403 there.
    let checks = [
        (format!("{}/auth/v1/health", args.reference), &keys.anon),
        (format!("{}/rest/v1/", args.reference), &keys.service_role),
        (format!("{}/_megabase/health", args.megabase), &keys.anon),
    ];
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    for (url, key) in &checks {
        loop {
            let last = match agent
                .get(url)
                .set("apikey", key)
                .set("Authorization", &format!("Bearer {key}"))
                .call()
            {
                Ok(r) if r.status() == 200 => {
                    eprintln!("ready: {url}");
                    break;
                }
                Ok(r) | Err(ureq::Error::Status(_, r)) => format!("status {}", r.status()),
                Err(err) => err.to_string(),
            };
            if Instant::now() > deadline {
                bail!(
                    "timed out after {}s waiting for {url} (last: {last})",
                    args.timeout
                );
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    Ok(())
}

fn summary_markdown(outcomes: &[Outcome], regressions: &[String]) -> String {
    let passing = outcomes.iter().filter(|o| o.pass).count();
    let mut md = format!(
        "### Judge: {passing}/{} cases conformant\n\n",
        outcomes.len()
    );
    if !regressions.is_empty() {
        let _ = writeln!(
            md,
            "**Regressions** (passed in the baseline, fail now): {}\n",
            regressions.join(", ")
        );
    }
    md.push_str("| Case | What it checks | Result | First difference |\n|---|---|---|---|\n");
    for o in outcomes {
        let detail = o
            .detail
            .as_deref()
            .unwrap_or("")
            .replace('|', "\\|")
            .replace('\n', " ");
        let result = if o.pass { "pass" } else { "fail" };
        let _ = writeln!(
            md,
            "| `{}` | {} | {result} | {detail} |",
            o.id, o.description
        );
    }
    md
}

fn run_all(args: &Args, keys: &Keys) -> Result<bool> {
    let cases = case::load(&args.cases)?;
    let reference = Target {
        name: "reference",
        base: args.reference.trim_end_matches('/').into(),
    };
    let megabase = Target {
        name: "megabase",
        base: args.megabase.trim_end_matches('/').into(),
    };
    let run_id = format!(
        "{:x}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let mut outcomes = Vec::new();
    for case in &cases {
        let outcome = run::run_case(case, &reference, &megabase, keys, &run_id)
            .with_context(|| format!("case `{}`", case.id))?;
        eprintln!(
            "{} {}",
            if outcome.pass { "pass" } else { "FAIL" },
            outcome.id
        );
        if let Some(detail) = &outcome.detail {
            eprintln!("     {detail}");
        }
        outcomes.push(outcome);
    }

    let results = Results {
        schema: 1,
        cases: outcomes
            .iter()
            .map(|o| CaseResult {
                id: o.id.clone(),
                pass: o.pass,
            })
            .collect(),
    };
    let mut regressions = Vec::new();
    if let Some(path) = &args.baseline {
        if path.exists() {
            let baseline: Results = serde_json::from_str(&std::fs::read_to_string(path)?)
                .with_context(|| format!("parsing {}", path.display()))?;
            for before in baseline.cases.iter().filter(|c| c.pass) {
                if !results.cases.iter().any(|c| c.id == before.id && c.pass) {
                    regressions.push(before.id.clone());
                }
            }
        }
    }
    if let Some(path) = &args.out {
        let mut json = serde_json::to_string_pretty(&results)?;
        json.push('\n');
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    }
    let md = summary_markdown(&outcomes, &regressions);
    if let Some(path) = &args.summary {
        std::fs::write(path, &md)?;
    }
    let passing = results.cases.iter().filter(|c| c.pass).count();
    eprintln!("{passing}/{} cases conformant", results.cases.len());
    if !regressions.is_empty() {
        eprintln!("regressions: {}", regressions.join(", "));
    }
    Ok(regressions.is_empty())
}

fn main() -> ExitCode {
    let result = parse_args().and_then(|args| {
        let keys = load_keys(&args.env)?;
        match args.command.as_str() {
            "wait" => wait(&args, &keys).map(|_| true),
            "run" => run_all(&args, &keys),
            other => bail!("unknown command `{other}`\n{USAGE}"),
        }
    });
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(2)
        }
    }
}
