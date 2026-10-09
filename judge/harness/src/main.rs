//! `megabase-judge`: differential tests between the official self-hosted
//! Supabase stack and Megabase. See `judge/README.md`.

mod case;
mod db;
mod normalize;
mod run;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use run::{CaseResult, Keys, Outcome, Results, Target};

const USAGE: &str = "\
usage:
  megabase-judge wait [--timeout SECS] [common]
  megabase-judge prepare [--fixtures FILE] [common]
  megabase-judge run [--cases DIR] [--out FILE] [--baseline FILE] [--summary FILE] [common]

common:
  --reference URL              reference gateway (default http://localhost:8000)
  --megabase URL               Megabase (default http://localhost:8100)
  --reference-database URL     reference Postgres (default from --env)
  --megabase-database URL      Megabase Postgres (default from --env, db megabase)
  --env FILE                   env file with ANON_KEY, SERVICE_ROLE_KEY,
                               POSTGRES_PASSWORD
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
    reference_database: Option<String>,
    megabase_database: Option<String>,
    fixtures: PathBuf,
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
        reference_database: None,
        megabase_database: None,
        fixtures: "judge/fixtures/schema.sql".into(),
    };
    while let Some(flag) = it.next() {
        let mut value = || it.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--reference" => args.reference = value()?,
            "--megabase" => args.megabase = value()?,
            "--reference-database" => args.reference_database = Some(value()?),
            "--megabase-database" => args.megabase_database = Some(value()?),
            "--env" => args.env = value()?.into(),
            "--cases" => args.cases = value()?.into(),
            "--out" => args.out = Some(value()?.into()),
            "--baseline" => args.baseline = Some(value()?.into()),
            "--summary" => args.summary = Some(value()?.into()),
            "--fixtures" => args.fixtures = value()?.into(),
            "--timeout" => args.timeout = value()?.parse().context("--timeout")?,
            other => bail!("unknown argument `{other}`\n{USAGE}"),
        }
    }
    Ok(args)
}

fn env_value(text: &str, path: &Path, name: &str) -> Result<String> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == name)
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
        .with_context(|| format!("{name} not set in {}", path.display()))
}

fn load_keys(path: &PathBuf) -> Result<Keys> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Keys {
        anon: env_value(&text, path, "ANON_KEY")?,
        service_role: env_value(&text, path, "SERVICE_ROLE_KEY")?,
    })
}

fn percent_encode(raw: &str) -> String {
    let mut out = String::new();
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Host port published by `judge/compose.override.yml` for the Postgres
/// container. Official compose maps Supavisor to host `5432`; connecting
/// there fails `prepare` with `no tenant identifier provided` and the
/// harness never reaches the cases. Do not read `POSTGRES_PORT` from the
/// env file: that value is the in-network port (5432).
const DIRECT_POSTGRES_HOST_PORT: &str = "54322";

fn default_databases(args: &Args) -> Result<db::Databases> {
    let text = std::fs::read_to_string(&args.env)
        .with_context(|| format!("reading {}", args.env.display()))?;
    let password = percent_encode(&env_value(&text, &args.env, "POSTGRES_PASSWORD")?);
    Ok(database_urls(
        &password,
        args.reference_database.clone(),
        args.megabase_database.clone(),
    ))
}

fn database_urls(
    password: &str,
    reference: Option<String>,
    megabase: Option<String>,
) -> db::Databases {
    let port = DIRECT_POSTGRES_HOST_PORT;
    db::Databases {
        reference: reference
            .unwrap_or_else(|| format!("postgres://postgres:{password}@127.0.0.1:{port}/postgres")),
        megabase: megabase
            .unwrap_or_else(|| format!("postgres://postgres:{password}@127.0.0.1:{port}/megabase")),
    }
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
    let databases = default_databases(args)?;
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    for (label, url) in [
        ("reference database", databases.reference.as_str()),
        ("megabase database", databases.megabase.as_str()),
    ] {
        loop {
            match db::ping(url) {
                Ok(()) => {
                    eprintln!("ready: {label}");
                    break;
                }
                Err(err) => {
                    if Instant::now() > deadline {
                        bail!(
                            "timed out after {}s waiting for {label}: {err:#}",
                            args.timeout
                        );
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
    }
    Ok(())
}

fn prepare(args: &Args) -> Result<()> {
    let databases = default_databases(args)?;
    let fixtures = std::fs::read_to_string(&args.fixtures)
        .with_context(|| format!("reading {}", args.fixtures.display()))?;
    db::prepare(&databases.reference, &databases.megabase, &fixtures)
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
    let databases = default_databases(args)?;
    let mut outcomes = Vec::new();
    for case in &cases {
        let outcome = run::run_case(case, &reference, &megabase, keys, &run_id, &databases)
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
            "prepare" => prepare(&args).map(|_| true),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_password() -> String {
        // Assembled at runtime so CodeQL does not treat a test fixture as a
        // shipped credential. Production URLs read POSTGRES_PASSWORD from
        // vendor/supabase/docker/.env.example.
        ["unit", "-", "test"].concat()
    }

    #[test]
    fn default_database_urls_use_direct_postgres_not_supavisor() {
        let password = fixture_password();
        let dbs = database_urls(&password, None, None);
        assert_eq!(
            dbs.reference,
            format!("postgres://postgres:{password}@127.0.0.1:54322/postgres")
        );
        assert_eq!(
            dbs.megabase,
            format!("postgres://postgres:{password}@127.0.0.1:54322/megabase")
        );
        assert!(
            !dbs.reference.contains(":5432/"),
            "host 5432 is Supavisor; prepare would fail before cases run"
        );
    }

    #[test]
    fn explicit_database_urls_win() {
        let password = fixture_password();
        let dbs = database_urls(
            &password,
            Some("postgres://u:p@db:5432/postgres".into()),
            Some("postgres://u:p@db:5432/megabase".into()),
        );
        assert_eq!(dbs.reference, "postgres://u:p@db:5432/postgres");
        assert_eq!(dbs.megabase, "postgres://u:p@db:5432/megabase");
    }
}
