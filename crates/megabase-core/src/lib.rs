// Megabase Core - Shared types, config, JWT, and error handling
// Ported from Supabase components (Apache-2.0, MIT licenses - see NOTICE)

pub mod config;
pub mod error;
pub mod jwt;

pub use config::Config;
pub use error::{Error, MegabaseNotImplemented, Result};
