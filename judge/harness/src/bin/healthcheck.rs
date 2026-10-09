//! Container healthcheck for the Megabase image: exits 0 when
//! `/_megabase/health` answers 200. The runtime image has no curl.

use std::process::ExitCode;
use std::time::Duration;

fn main() -> ExitCode {
    let port = std::env::var("MEGABASE_PORT").unwrap_or_else(|_| "8000".into());
    let url = format!("http://127.0.0.1:{port}/_megabase/health");
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(2))
        .build();
    match agent.get(&url).call() {
        Ok(response) if response.status() == 200 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
