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
    /// Show full detail for a single discovered device.
    Show(ShowArgs),
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    /// Show every discovered HID device, not just known-RGB-vendor matches.
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct ShowArgs {
    /// The device id, as printed by `colorer list`.
    pub id: String,
}

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
            Commands::List(args) => assert!(!args.all),
            _ => panic!("expected Commands::List"),
        }
    }

    #[test]
    fn list_subcommand_parses_all_flag() {
        let cli = Cli::try_parse_from(["colorer", "list", "--all"]).unwrap();
        match cli.command {
            Commands::List(args) => assert!(args.all),
            _ => panic!("expected Commands::List"),
        }
    }

    #[test]
    fn missing_subcommand_is_error() {
        assert!(Cli::try_parse_from(["colorer"]).is_err());
    }

    #[test]
    fn show_subcommand_parses() {
        let cli = Cli::try_parse_from(["colorer", "show", "hid-deadbeef"]).unwrap();
        match cli.command {
            Commands::Show(args) => assert_eq!(args.id, "hid-deadbeef"),
            _ => panic!("expected Commands::Show"),
        }
    }

    #[test]
    fn show_missing_id_is_error() {
        assert!(Cli::try_parse_from(["colorer", "show"]).is_err());
    }
}
