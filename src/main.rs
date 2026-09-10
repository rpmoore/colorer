mod cli;

use clap::Parser;
use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::List(_args) => {
            // M0: no device logic yet. Real implementation lands in section-02 (M1).
        }
    }
}
