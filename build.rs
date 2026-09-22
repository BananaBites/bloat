//! Build-time version stamping: the git commit the binary was built from.
//!
//! `cargo install --git` builds inside a git checkout, so the hash is available
//! there too. When git (or the checkout) is missing we degrade to "unknown"
//! instead of failing the build.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for probe in [".git/HEAD", ".git/refs"] {
        if std::path::Path::new(probe).exists() {
            println!("cargo:rerun-if-changed={probe}");
        }
    }
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let hash = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let short = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| short_of(&hash));
    let reference = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let date = git(&["log", "-1", "--format=%cs"]).unwrap_or_else(build_date);

    println!("cargo:rustc-env=BLOAT_GIT_HASH={hash}");
    println!("cargo:rustc-env=BLOAT_GIT_SHORT={short}");
    println!("cargo:rustc-env=BLOAT_GIT_REF={reference}");
    println!(
        "cargo:rustc-env=BLOAT_GIT_DIRTY={}",
        if dirty { "yes" } else { "no" }
    );
    println!("cargo:rustc-env=BLOAT_BUILD_DATE={date}");
    println!(
        "cargo:rustc-env=BLOAT_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=BLOAT_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn short_of(hash: &str) -> String {
    hash.chars().take(7).collect()
}

/// `YYYY-MM-DD`, from SOURCE_DATE_EPOCH when set (reproducible builds).
fn build_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Howard Hinnant's days-from-civil, inverted.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
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