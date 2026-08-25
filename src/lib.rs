pub mod backup;
pub mod caffeine;
pub mod cli;
pub mod codex_config;
pub mod command;
pub mod doctor;
pub mod fsutil;
pub mod launcher;
pub mod notify;
pub mod paths;
pub mod profiles;
pub mod state;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
