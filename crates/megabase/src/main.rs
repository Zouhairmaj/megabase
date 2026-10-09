// Megabase - Supabase-compatible API server, written in Rust
// A drop-in replacement for Supabase's self-hosted stack
// https://github.com/AgenP/megabase
//
// Licensed under Apache-2.0 - see NOTICE for credits

use megabase_core::Config;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "megabase=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env();
    megabase_server::run(config).await
}
