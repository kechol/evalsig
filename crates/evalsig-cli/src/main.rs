//! The `evalsig` command-line interface.

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "evalsig", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    Ok(())
}
