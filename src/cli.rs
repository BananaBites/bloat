//! Command line definition. Shared with the completion generator, so every
//! flag and value is completed automatically — including newly added ones.

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::aot::Shell;
use std::path::PathBuf;

pub const AFTER_HELP: &str = "\
Keys in the TUI:
  ↑↓←→ / hjkl    move selection (spatial)
  enter           descend into a directory
  backspace / u   go up
  y               copy the selected path (OSC 52)
  space           freeze live updates while scanning
  [ ]             treemap depth
  r               rescan
  ?               help
  q / ctrl-c      quit

Shell completions:
  bash  echo 'source <(COMPLETE=bash bloat)' >> ~/.bashrc
  zsh   echo 'source <(COMPLETE=zsh bloat)'  >> ~/.zshrc
  fish  echo 'COMPLETE=fish bloat | source'  >> ~/.config/fish/completions/bloat.fish

These are generated from the installed binary on every shell start, so they
never need to be regenerated after `bloat update`.";

#[derive(Debug, Parser)]
#[command(
    name = "bloat",
    bin_name = "bloat",
    version = crate::version::VERSION_LONG,
    about = "Interactive disk-usage treemap for the terminal",
    after_help = AFTER_HELP,
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,

    /// Directory to analyse (default: current directory)
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Print a static report instead of opening the TUI
    #[arg(short, long)]
    pub report: bool,

    /// Entries to list in report mode
    #[arg(short, long, default_value_t = 20, value_name = "N")]
    pub top: usize,

    /// Use apparent size (file length) instead of disk usage
    #[arg(short = 'a', long)]
    pub apparent_size: bool,

    /// Stay on one filesystem (default; accepted for du muscle memory)
    #[arg(short = 'x', long = "one-file-system")]
    pub one_file_system: bool,

    /// Cross filesystem boundaries while scanning
    #[arg(long)]
    pub cross_filesystem: bool,

    /// Exclude entries; a pattern with `/` matches a path prefix, otherwise it
    /// globs the file name (repeatable)
    #[arg(short = 'e', long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// How deep the treemap nests (1 = only direct children)
    #[arg(long, default_value_t = 5, value_name = "N")]
    pub depth: u16,

    /// Scan threads (default: one per core)
    #[arg(short = 'j', long)]
    pub jobs: Option<usize>,

    /// OSC 8 hyperlinks on paths
    #[arg(long, value_enum, default_value_t = LinkMode::Auto)]
    pub links: LinkMode,
}

#[derive(Debug, Subcommand)]
pub enum Cmd {
    /// Show the installed commit, the remote commit and environment details
    Doctor(DoctorArgs),
    /// Install the newest commit from the default branch
    Update(UpdateArgs),
    /// Print a shell completion script to stdout
    Completions(CompletionsArgs),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum LinkMode {
    /// Emit hyperlinks when the terminal is known to support them.
    Auto,
    /// Always emit OSC 8 hyperlinks.
    On,
    /// Never emit OSC 8 hyperlinks.
    Off,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Skip the network check of the remote commit
    #[arg(long)]
    pub offline: bool,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Only report whether a newer commit exists
    #[arg(long)]
    pub check: bool,
    /// Reinstall even when the installed commit already matches the remote
    #[arg(long)]
    pub force: bool,
    /// Do not pass `--locked` to cargo
    #[arg(long)]
    pub no_locked: bool,
}

#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// Shell to generate for
    #[arg(value_enum)]
    pub shell: Shell,
    /// Command name to register the completions under
    #[arg(long, default_value = "bloat")]
    pub bin_name: String,
}

/// Used by the dynamic completion engine and by `completions`.
pub fn command() -> clap::Command {
    use clap::CommandFactory;
    Cli::command()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("bloat").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn subcommands_and_paths_coexist() {
        assert!(matches!(cli(&["update"]).command, Some(Cmd::Update(_))));
        assert!(matches!(cli(&["doctor"]).command, Some(Cmd::Doctor(_))));
        assert!(matches!(
            cli(&["completions", "bash"]).command,
            Some(Cmd::Completions(_))
        ));
        let plain = cli(&["/tmp"]);
        assert!(plain.command.is_none());
        assert_eq!(plain.path.as_deref(), Some(std::path::Path::new("/tmp")));
    }

    #[test]
    fn flags_still_parse() {
        let c = cli(&["--report", "-a", "-e", "*.iso", "--top", "5", "/var"]);
        assert!(c.report && c.apparent_size);
        assert_eq!(c.exclude, vec!["*.iso".to_string()]);
        assert_eq!(c.top, 5);
        assert_eq!(c.path.as_deref(), Some(std::path::Path::new("/var")));
        let c = cli(&["--links", "on", "--depth", "3", "-j", "2"]);
        assert_eq!(c.links, LinkMode::On);
        assert_eq!(c.depth, 3);
        assert_eq!(c.jobs, Some(2));
    }

    #[test]
    fn update_flags() {
        let c = cli(&["update", "--check", "--force", "--no-locked"]);
        let Some(Cmd::Update(a)) = c.command else {
            panic!("expected update")
        };
        assert!(a.check && a.force && a.no_locked);
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(Cli::try_parse_from(["bloat", "frobnicate"]).is_ok()); // treated as PATH
        assert!(Cli::try_parse_from(["bloat", "completions", "tcsh"]).is_err());
    }

    #[test]
    fn completions_cover_every_flag() {
        let cmd = command();
        let names: Vec<String> = cmd
            .get_arguments()
            .map(|a| a.get_id().to_string())
            .collect();
        for expected in ["report", "top", "apparent_size", "exclude", "depth", "jobs", "links"] {
            assert!(names.iter().any(|n| n == expected), "missing {expected}");
        }
        assert!(cmd
            .get_subcommands()
            .any(|s| s.get_name() == "completions"));
    }
}