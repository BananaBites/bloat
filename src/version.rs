//! Version identity: what this binary is, and what the remote has.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_HASH: &str = env!("BLOAT_GIT_HASH");
pub const GIT_SHORT: &str = env!("BLOAT_GIT_SHORT");
pub const GIT_REF: &str = env!("BLOAT_GIT_REF");
pub const GIT_DIRTY: &str = env!("BLOAT_GIT_DIRTY");
pub const BUILD_DATE: &str = env!("BLOAT_BUILD_DATE");
pub const PROFILE: &str = env!("BLOAT_PROFILE");
pub const TARGET: &str = env!("BLOAT_TARGET");
/// Where `bloat update` installs from.
pub const REPO: &str = env!("CARGO_PKG_REPOSITORY");
pub const VERSION_LONG: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("BLOAT_GIT_SHORT"),
    ")"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub version: &'static str,
    pub hash: String,
    pub short: String,
    pub reference: String,
    pub dirty: bool,
    pub date: String,
    pub profile: String,
    pub target: String,
    pub binary: PathBuf,
}

impl Local {
    pub fn probe() -> Self {
        let binary = std::env::current_exe().unwrap_or_default();
        Self {
            version: VERSION,
            hash: GIT_HASH.to_string(),
            short: GIT_SHORT.to_string(),
            reference: GIT_REF.to_string(),
            dirty: GIT_DIRTY == "yes",
            date: BUILD_DATE.to_string(),
            profile: PROFILE.to_string(),
            target: TARGET.to_string(),
            binary,
        }
    }

    /// `v0.1.0 @ 6eb6784` — the hash is the identity, versions can repeat.
    pub fn stamp(&self) -> String {
        format!(
            "v{} @ {}{}",
            self.version,
            self.short,
            if self.dirty { "+dirty" } else { "" }
        )
    }

    pub fn is_unknown(&self) -> bool {
        self.hash == "unknown" || self.hash.is_empty()
    }

    /// True when this binary was built outside a git checkout, so it can never
    /// know its own commit.
    pub fn branch_label(&self) -> Option<&str> {
        match self.reference.as_str() {
            "" | "unknown" | "HEAD" => None,
            other => Some(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remote {
    pub hash: String,
    pub short: String,
    pub branch: String,
    pub date: Option<String>,
    pub subject: Option<String>,
}

impl Remote {
    pub fn stamp(&self) -> String {
        format!("@ {}", self.short)
    }
}

/// `git ls-remote --symref <repo> HEAD` — no full clone needed.
pub fn remote_head() -> Result<Remote> {
    let repo = repo_url();
    let out = Command::new("git")
        .args(["ls-remote", "--symref", &repo, "HEAD"])
        .output()
        .context("running git ls-remote (is git installed?)")?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("git ls-remote failed: {}", err.trim());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let remote = parse_ls_remote(&text).context("unexpected git ls-remote output")?;
    Ok(remote)
}

/// Parse the two lines we care about:
/// `ref: refs/heads/main\tHEAD` and `<sha>\tHEAD`.
pub fn parse_ls_remote(text: &str) -> Option<Remote> {
    let mut branch = None;
    let mut hash = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("ref: ") {
            let target = rest.split('\t').next().unwrap_or("");
            branch = target.rsplit('/').next().map(|s| s.to_string());
        } else if let Some((sha, name)) = line.split_once('\t') {
            if name.trim() == "HEAD" {
                hash = Some(sha.trim().to_string());
            }
        }
    }
    let hash = hash?;
    Some(Remote {
        short: hash.chars().take(7).collect(),
        hash,
        branch: branch.unwrap_or_else(|| "default".into()),
        date: None,
        subject: None,
    })
}

/// Where `bloat update` fetches from. `BLOAT_REPO` overrides the built-in URL
/// so a fork, mirror or local clone can be used for testing or self-hosting.
pub fn repo_url() -> String {
    std::env::var("BLOAT_REPO")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| REPO.to_string())
}

/// Best-effort enrichment from the GitHub API (commit date + subject).
/// Silently returns `None` when offline, when `curl` is missing, or when the
/// repository is not on GitHub.
pub fn enrich_remote(remote: &mut Remote) {
    let Some(slug) = github_slug() else {
        return;
    };
    let url = format!("https://api.github.com/repos/{slug}/commits/{}", remote.hash);
    let out = Command::new("curl")
        .args(["-fsS", "--max-time", "6", "-H", "Accept: application/vnd.github+json", &url])
        .output();
    let Ok(out) = out else { return };
    if !out.status.success() {
        return;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let date = json_string(&text, "\"date\":").filter(|d| d.starts_with("20"));
    let subject = json_string(&text, "\"message\":").map(|m| m.lines().next().unwrap_or("").to_string());
    remote.date = date;
    remote.subject = subject;
}

/// `owner/name` for the configured repository URL.
pub fn github_slug() -> Option<String> {
    let url = repo_url();
    let rest = url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .split("github.com")
        .nth(1)?
        .trim_start_matches(['/', ':']);
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if owner.is_empty() || name.is_empty() {
        None
    } else {
        Some(format!("{owner}/{name}"))
    }
}

/// Extract the first `"key": "value"` string value after `key`.
fn json_string(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)? + key.len();
    let rest = text[at..].trim_start().trim_start_matches(':').trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') | Some('r') => out.push(' '),
                Some(other) => out.push(other),
                None => break,
            },
            '"' => return Some(out.trim().to_string()),
            other => out.push(other),
        }
    }
    None
}

/// `cargo 1.98.1 (797e8a9b 2026-08-05)` -> `1.98.1`
pub fn numeric_token(line: &str) -> Option<String> {
    line.split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|t| t.to_string())
}

pub fn tool_version(cmd: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().lines().next()?.to_string())
}

/// The cargo binary to use for `bloat update`.
pub fn cargo_bin() -> Option<String> {
    if let Ok(cargo) = std::env::var("CARGO") {
        if !cargo.is_empty() {
            return Some(cargo);
        }
    }
    tool_version("cargo", &["--version"]).map(|_| "cargo".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ls_remote() {
        let text = "ref: refs/heads/main\tHEAD\n6eb6784305cb131f95eacc202447bf876b6d1b77\tHEAD\n";
        let remote = parse_ls_remote(text).unwrap();
        assert_eq!(remote.branch, "main");
        assert_eq!(remote.short, "6eb6784");
        assert_eq!(remote.hash.len(), 40);
    }

    #[test]
    fn parses_ls_remote_without_symref_line() {
        let text = "abcdef1234567890\tHEAD\n";
        let remote = parse_ls_remote(text).unwrap();
        assert_eq!(remote.branch, "default");
        assert_eq!(remote.short, "abcdef1");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_ls_remote("").is_none());
        assert!(parse_ls_remote("huh\n").is_none());
    }

    #[test]
    fn reads_json_fields() {
        let json = r#"{"sha":"abc","commit":{"author":{"name":"x","date":"2026-09-22T10:00:00Z"},"message":"fix: labels\n\nbody"}}"#;
        assert_eq!(json_string(json, "\"date\":").as_deref(), Some("2026-09-22T10:00:00Z"));
        let message = json_string(json, "\"message\":").unwrap();
        assert_eq!(message.lines().next(), Some("fix: labels"));
        assert_eq!(numeric_token("cargo 1.98.1 (797e8a9bc 2026-08-05)").as_deref(), Some("1.98.1"));
        assert_eq!(numeric_token("git version 2.43.0").as_deref(), Some("2.43.0"));
        assert_eq!(numeric_token("curl 8.5.0 (x86_64)").as_deref(), Some("8.5.0"));
    }

    #[test]
    fn repo_slug() {
        assert_eq!(github_slug().as_deref(), Some("BananaBites/bloat"));
    }

    #[test]
    fn repo_url_can_be_overridden() {
        // SAFETY: single-threaded test setup, and the value only affects the
        // already-parsed helpers.
        unsafe { std::env::set_var("BLOAT_REPO", "file:///tmp/fork.git") };
        assert_eq!(repo_url(), "file:///tmp/fork.git");
        assert_eq!(github_slug(), None);
        unsafe { std::env::remove_var("BLOAT_REPO") };
        assert!(repo_url().contains("github.com"));
    }

    #[test]
    fn local_stamp_is_readable() {
        let local = Local::probe();
        assert!(local.stamp().starts_with(&format!("v{VERSION} @ ")));
        assert!(!local.date.is_empty());
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days_public(0), (1970, 1, 1));
        assert_eq!(civil_from_days_public(20_000), (2024, 10, 4));
    }

    fn civil_from_days_public(days: i64) -> (i64, u32, u32) {
        // mirrors build.rs; verified against a known date
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        (if m <= 2 { y + 1 } else { y }, m, d)
    }
}