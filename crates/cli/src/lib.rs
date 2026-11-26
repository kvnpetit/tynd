mod args;
mod commands;
mod config;
mod fsx;
mod mcp;
mod output;
mod process;
mod project;
mod source_edit;

pub use args::{Cli, McpCli};
pub use commands::run;
pub use mcp::run as run_mcp;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
