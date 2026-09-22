//! bloat — interactive disk-usage treemap for the terminal.
//!
//! `bloat [PATH]` opens a TUI; when stdout is not a terminal (or with
//! `--report`) it prints a static report instead. `bloat update`, `bloat
//! doctor` and `bloat completions` maintain the installation itself.

mod app;
mod arena;
mod cli;
mod fmt;
mod maintenance;
mod palette;
mod render;
mod report;
mod scan;
mod treemap;
mod ui;
mod version;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Cmd, LinkMode};
use std::io::IsTerminal;

fn main() -> Result<()> {
    restore_sigpipe();
    // Dynamic completions: the shell calls us with COMPLETE=<shell> set, we
    // answer and exit. Must happen before anything writes to stdout.
    clap_complete::CompleteEnv::with_factory(cli::command).complete();

    let args = Cli::parse();
    match args.command {
        Some(Cmd::Doctor(d)) => maintenance::doctor(d),
        Some(Cmd::Update(u)) => maintenance::update(u),
        Some(Cmd::Completions(c)) => maintenance::completions(c),
        None => scan_and_show(args),
    }
}

/// Rust ignores SIGPIPE, which turns `bloat / | head` into a panic on EPIPE.
/// Restore the default so short-lived pipes end the process quietly.
fn restore_sigpipe() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn scan_and_show(args: Cli) -> Result<()> {
    let root = match args.path.clone() {
        Some(p) => p,
        None => std::env::current_dir()?,
    };
    let _ = args.one_file_system; // `-x` describes the default behaviour
    let opts = scan::Options {
        apparent_size: args.apparent_size,
        one_file_system: !args.cross_filesystem,
        excludes: args.exclude.clone(),
        jobs: args.jobs,
        top_n: 500,
    };
    let links = match args.links {
        LinkMode::On => true,
        LinkMode::Off => false,
        LinkMode::Auto => app::hyperlinks_supported(),
    };

    let scan = scan::Scan::new(&root, opts.clone())?;
    scan.start_thread();

    if args.report || !app::interactive() {
        scan.wait();
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        print!("{}", report::render(&scan, args.top, color));
        return Ok(());
    }

    app::run(scan, root, opts, links, args.depth)
}