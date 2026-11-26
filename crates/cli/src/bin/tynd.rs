use anyhow::Result;
use clap::Parser;
use tynd_cli::Cli;

fn main() -> Result<()> {
    tynd_cli::run(Cli::parse())
}
