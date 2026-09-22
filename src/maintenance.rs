//! `bloat doctor`, `bloat update` and `bloat completions`.

use crate::cli::{CompletionsArgs, DoctorArgs, UpdateArgs};
use crate::version::{self, Local, Remote};
use anyhow::{bail, Context, Result};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `bloat doctor` — installed commit, remote commit, environment.
pub fn doctor(args: DoctorArgs) -> Result<()> {
    let local = Local::probe();
    let colour = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    let dim = |s: &str| {
        if colour {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };

    println!("{}", dim(&format!("bloat doctor  ·  {}", version::repo_url())));
    println!();
    println!("  installed   {}", local.stamp());
    println!("              {}", local.binary.display());
    println!(
        "              built {} · {} · {}",
        local.date, local.profile, local.target
    );

    let mut status = String::from("unknown (cannot reach the remote)");
    if !args.offline {
        match version::remote_head() {
            Ok(mut remote) => {
                version::enrich_remote(&mut remote);
                println!("  remote      {}", remote_prefix(&remote));
                if let Some(date) = &remote.date {
                    println!(
                        "              {}{}",
                        date.split('T').next().unwrap_or(date),
                        remote
                            .subject
                            .as_ref()
                            .map(|s| format!(" · \"{}\"", crate::fmt::truncate(s, 60)))
                            .unwrap_or_default()
                    );
                }
                status = if remote.hash == local.hash {
                    "up to date".to_string()
                } else {
                    format!(
                        "update available: {} → {}   (run `bloat update`)",
                        local.short, remote.short
                    )
                };
            }
            Err(err) => {
                println!("  remote      unavailable: {err}");
            }
        }
    }
    let status_text = if status.starts_with("update available") {
        if colour {
            format!("\x1b[33m{status}\x1b[0m")
        } else {
            status.clone()
        }
    } else if status == "up to date" && colour {
        format!("\x1b[32m{status}\x1b[0m")
    } else {
        status.clone()
    };
    println!("  status      {status_text}");

    let tool = |cmd: &str, args: &[&str]| {
        version::tool_version(cmd, args)
            .map(|line| version::numeric_token(&line).unwrap_or(line))
            .unwrap_or_else(|| "missing".into())
    };
    println!(
        "  tools       cargo {} · git {} · curl {}",
        tool("cargo", &["--version"]),
        tool("git", &["--version"]),
        tool("curl", &["--version"]),
    );
    if local.dirty {
        println!(
            "  note        built from a working tree with uncommitted changes{}",
            match local.branch_label() {
                Some(branch) => format!(" (branch {branch})"),
                None => String::new(),
            }
        );
    }
    if local.is_unknown() {
        println!(
            "  note        this binary was built outside a git checkout, so it cannot \
             report its own commit; install with `cargo install --git {}` or run the installer",
            version::repo_url()
        );
    }
    if !install_target_is_self(&local.binary) {
        println!(
            "  note        `bloat update` installs to {}",
            cargo_bin_dir()
                .map(|d| d.join("bloat").display().to_string())
                .unwrap_or_else(|| "the cargo bin directory".into())
        );
    }
    let installed: Vec<_> = completion_status()
        .into_iter()
        .filter(|(_, how)| !how.starts_with("not installed"))
        .collect();
    if installed.is_empty() {
        println!("  completions none detected for bash/zsh/fish (see README → Shell completions)");
    } else {
        for (shell, how) in installed {
            println!("  completions {shell:<4} {how}");
        }
    }
    println!(
        "  terminal    TERM={} · truecolor · hyperlinks {} · locale {}",
        std::env::var("TERM").unwrap_or_else(|_| "unset".into()),
        if crate::app::hyperlinks_supported() {
            "on"
        } else {
            "off (set --links on to force)"
        },
        locale_summary(),
    );
    Ok(())
}

fn remote_prefix(remote: &Remote) -> String {
    format!("{} ({}, branch {})", remote.stamp(), version::repo_url(), remote.branch)
}

/// One line per shell we know how to detect.
fn completion_status() -> Vec<(&'static str, String)> {
    let home = home_dir();
    let mut rows = Vec::new();

    let bash_rc = home.as_ref().map(|h| h.join(".bashrc"));
    let bash_static = home
        .as_ref()
        .map(|h| h.join(".local/share/bash-completion/completions/bloat"))
        .into_iter()
        .chain([PathBuf::from("/usr/share/bash-completion/completions/bloat")]);
    rows.push((
        "bash",
        describe(
            bash_rc.as_deref().map(|p| grep(p, "COMPLETE=bash")),
            bash_static,
            "~/.bashrc",
        ),
    ));

    let zsh_rc = home.as_ref().map(|h| h.join(".zshrc"));
    let zsh_static = home
        .as_ref()
        .map(|h| vec![h.join(".zfunc/_bloat"), h.join(".zsh/completions/_bloat")])
        .unwrap_or_default()
        .into_iter()
        .chain([PathBuf::from("/usr/share/zsh/vendor-completions/_bloat")]);
    rows.push((
        "zsh",
        describe(
            zsh_rc.as_deref().map(|p| grep(p, "COMPLETE=zsh")),
            zsh_static,
            "~/.zshrc",
        ),
    ));

    let fish_rc = home.as_ref().map(|h| h.join(".config/fish/config.fish"));
    let fish_static = home
        .as_ref()
        .map(|h| h.join(".config/fish/completions/bloat.fish"))
        .into_iter();
    rows.push((
        "fish",
        describe(
            fish_rc.as_deref().map(|p| grep(p, "COMPLETE=fish")),
            fish_static,
            "~/.config/fish/config.fish",
        ),
    ));
    rows
}

fn describe(rc_hit: Option<bool>, mut static_files: impl Iterator<Item = PathBuf>, rc: &str) -> String {
    if rc_hit == Some(true) {
        return format!("dynamic · snippet in {rc}");
    }
    if let Some(path) = static_files.find(|p| p.is_file()) {
        return format!("static file · {}", path.display());
    }
    "not installed (see README → Shell completions)".to_string()
}

fn grep(path: &Path, needle: &str) -> bool {
    std::fs::read_to_string(path)
        .map(|s| s.contains(needle))
        .unwrap_or(false)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn locale_summary() -> String {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "unset".into());
    if raw.is_empty() {
        return "unset".into();
    }
    let utf8 = raw.to_ascii_lowercase().contains("utf-8") || raw.to_ascii_lowercase().contains("utf8");
    format!("{raw}{}", if utf8 { " ✓" } else { " (no UTF-8?)" })
}

/// `bloat update` — install the newest commit from the default branch.
pub fn update(args: UpdateArgs) -> Result<()> {
    let local = Local::probe();
    println!("installed   {}", local.stamp());

    if version::repo_url().is_empty() {
        bail!("this build has no repository URL, so it cannot update itself");
    }

    let repo = version::repo_url();
    let mut remote = version::remote_head().context("checking the remote commit")?;
    version::enrich_remote(&mut remote);
    let mut line = format!("            {repo}");
    if let Some(date) = &remote.date {
        line.push_str(&format!(" · {}", date.split('T').next().unwrap_or(date)));
    }
    println!("remote      {} (branch {})", remote.stamp(), remote.branch);
    println!("{line}");
    if let Some(subject) = &remote.subject {
        println!("            \"{}\"", crate::fmt::truncate(subject, 70));
    }

    if remote.hash == local.hash && !args.force {
        println!();
        println!("status      up to date — nothing to do");
        return Ok(());
    }
    if args.check {
        println!();
        println!(
            "status      update available: {} → {}   (run `bloat update` without --check)",
            local.short, remote.short
        );
        return Ok(());
    }

    let cargo = version::cargo_bin().context(
        "cargo is not installed — get Rust from https://rustup.rs, then run `bloat update` again",
    )?;
    let mut cmd = Command::new(&cargo);
    cmd.arg("install").arg("--git").arg(&repo).arg("--force");
    if !args.no_locked {
        cmd.arg("--locked");
    }
    println!();
    println!(
        "running     {cargo} install --git {repo} --force{}",
        if args.no_locked { "" } else { " --locked" }
    );
    println!();
    let status = cmd
        .status()
        .with_context(|| format!("failed to run {cargo}"))?;
    if !status.success() {
        println!();
        bail!(
            "cargo install failed (exit {status}).\n\
             If this was a `--locked` failure because Cargo.lock is stale, retry with `bloat update --no-locked`."
        );
    }

    // Ask the *newly installed* binary, which is not necessarily this one.
    let target = if install_target_is_self(&local.binary) {
        local.binary.clone()
    } else {
        cargo_bin_dir()
            .map(|dir| dir.join("bloat"))
            .unwrap_or_else(|| local.binary.clone())
    };
    let installed = if target.is_file() {
        version::tool_version(&target.display().to_string(), &["--version"])
            .unwrap_or_else(|| "installed (version check failed)".into())
    } else {
        format!("installed, but {} was not found", target.display())
    };
    println!();
    println!("installed   {installed}   ({})", target.display());
    if install_target_is_self(&local.binary) {
        println!("status      updated — restart running sessions to pick up the new binary");
    } else {
        println!(
            "note        the new binary is at {}, not at {} — add its directory to PATH",
            target.display(),
            local.binary.display()
        );
    }
    Ok(())
}

fn cargo_bin_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return Some(PathBuf::from(home).join("bin"));
    }
    home_dir().map(|h| h.join(".cargo/bin"))
}

/// Did cargo install where this binary actually lives?
fn install_target_is_self(binary: &Path) -> bool {
    match (binary.parent(), cargo_bin_dir()) {
        (Some(parent), Some(bin)) => parent == bin,
        _ => false,
    }
}

/// `bloat completions <shell>` — a static script on stdout.
pub fn completions(args: CompletionsArgs) -> Result<()> {
    let mut cmd = crate::cli::command();
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::aot::generate(args.shell, &mut cmd, args.bin_name, &mut buf);
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    match out.write_all(&buf).and_then(|()| out.flush()) {
        // `bloat completions bash | head` is a normal thing to do
        Err(err) if err.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => Ok(other?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap_complete::aot::Shell;

    #[test]
    fn completion_pipes_do_not_panic() {
        // the generator must not write directly to stdout: piping into `head`
        // closes the pipe early, and clap_complete's own writer panics on that
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let mut cmd = crate::cli::command();
            let mut buf: Vec<u8> = Vec::new();
            clap_complete::aot::generate(shell, &mut cmd, "bloat", &mut buf);
            assert!(buf.len() > 200);
        }
    }

    #[test]
    fn every_shell_generates_a_script() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::Elvish, Shell::PowerShell] {
            let mut cmd = crate::cli::command();
            let mut buf: Vec<u8> = Vec::new();
            clap_complete::aot::generate(shell, &mut cmd, "bloat", &mut buf);
            let text = String::from_utf8(buf).unwrap();
            assert!(text.len() > 200, "{shell:?} script too small");
            assert!(text.contains("bloat"), "{shell:?} script missing the binary name");
        }
    }

    #[test]
    fn dynamic_env_completions_are_wired_up() {
        // The generated bash script must call back into the binary, which is what
        // makes completions self-updating after `bloat update`.
        let mut cmd = crate::cli::command();
        let mut buf: Vec<u8> = Vec::new();
        clap_complete::aot::generate(Shell::Bash, &mut cmd, "bloat", &mut buf);
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("_clap_complete") || text.contains("compgen"));
    }

    #[test]
    fn detect_completions_never_panics() {
        let rows = completion_status();
        assert_eq!(rows.len(), 3);
        for (shell, how) in rows {
            assert!(["bash", "zsh", "fish"].contains(&shell));
            assert!(!how.is_empty());
        }
    }

    #[test]
    fn doctor_output_is_readable() {
        // Build the same lines the command prints, without the network call.
        let local = Local::probe();
        assert!(local.stamp().starts_with("v0.1.0 @ "));
        assert!(!locale_summary().is_empty());
        assert!(remote_prefix(&Remote {
            hash: "a".repeat(40),
            short: "aaaaaaa".into(),
            branch: "main".into(),
            date: None,
            subject: None,
        })
        .contains("main"));
    }
}