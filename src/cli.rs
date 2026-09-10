use clap::{Args, Parser, Subcommand};

// Top-level CLI definition, parsed once in `main`. Parsing only — never touches
// devices/filesystem; see docs/knowledge/cli/parsing-boundary.md.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List discovered RGB-capable devices.
    List(ListArgs),
}

// Empty for M0; `--all` is added in section-02 (M1).
#[derive(Args, Debug)]
pub struct ListArgs {}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, error::ErrorKind};

    #[test]
    fn debug_assert_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn help_flag_triggers_display_help() {
        let err = Cli::try_parse_from(["colorer", "--help"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }

    #[test]
    fn version_flag_triggers_display_version() {
        let err = Cli::try_parse_from(["colorer", "--version"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn list_subcommand_parses() {
        let cli = Cli::try_parse_from(["colorer", "list"]).unwrap();
        match cli.command {
            Commands::List(_) => {}
        }
    }

    #[test]
    fn missing_subcommand_is_error() {
        assert!(Cli::try_parse_from(["colorer"]).is_err());
    }
}
