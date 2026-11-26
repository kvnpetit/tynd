use anyhow::Result;
use clap::Parser;
use tynd_cli::McpCli;

fn main() -> Result<()> {
    tynd_cli::run_mcp(McpCli::parse())
}
