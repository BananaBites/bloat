//! Headless report: the graphical-CLI fallback for pipes, ssh and `--report`.

use crate::arena::NodeId;
use crate::fmt;
use crate::palette;
use crate::scan::Scan;

fn paint(rgb: [u8; 3], color: bool, text: &str) -> String {
    if color {
        format!("\x1b[38;2;{};{};{}m{text}\x1b[0m", rgb[0], rgb[1], rgb[2])
    } else {
        text.to_string()
    }
}

/// Cool-to-hot scale, red = big.
fn heat(fraction: f32) -> [u8; 3] {
    let hue = 205.0 * (1.0 - fraction.clamp(0.0, 1.0));
    palette::hsl_to_rgb(hue, 0.72, 0.62)
}

pub fn render(scan: &Scan, top: usize, color: bool) -> String {
    let arena = scan.arena_lock();
    let root = arena.root;
    let name = arena.nodes[root as usize].name.clone();
    let total = arena.nodes[root as usize].size;
    let files = arena.nodes[root as usize].files;
    let (_, entries, dirs, errors, _) = scan.prog.snapshot();
    let elapsed_ms = scan.prog.elapsed_ms.load(std::sync::atomic::Ordering::Relaxed);
    let mode = if scan.opts.apparent_size {
        "apparent size"
    } else {
        "disk usage"
    };

    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        paint([122, 200, 255], color, "bloat"),
        name
    ));
    out.push_str(&format!(
        "  {} · {} · {} dirs · {} entries · {:.1}s · {mode}\n\n",
        paint([226, 226, 236], color, &fmt::human(total)),
        format!("{} files", fmt::group(files)),
        fmt::group(dirs),
        fmt::group(entries),
        elapsed_ms as f64 / 1000.0,
    ));

    let (children, rest) = arena.top_children(root, top.max(1));
    if !children.is_empty() {
        out.push_str(&format!(
            "{}\n",
            paint([160, 160, 175], color, "  largest entries")
        ));
        for c in &children {
            let n = &arena.nodes[*c as usize];
            let f = if total > 0 {
                n.size as f32 / total as f32
            } else {
                0.0
            };
            out.push_str(&format!(
                "  {} {} {:>4.0}%  {}\n",
                paint(heat(f), color, &fmt::bar(f, 22)),
                paint([226, 226, 236], color, &format!("{:>7}", fmt::human(n.size))),
                fmt::percent(n.size, total),
                n.name,
            ));
        }
        if let Some((size, count, _)) = rest {
            let f = if total > 0 {
                size as f32 / total as f32
            } else {
                0.0
            };
            out.push_str(&format!(
                "  {} {} {:>4.0}%  +{} more\n",
                paint(heat(f), color, &fmt::bar(f, 22)),
                paint([160, 160, 175], color, &format!("{:>7}", fmt::human(size))),
                fmt::percent(size, total),
                fmt::group(count),
            ));
        }
        out.push('\n');
    }
    drop(arena);

    let files = scan.top_files(top.max(1));
    if !files.is_empty() {
        out.push_str(&format!(
            "{}\n",
            paint([160, 160, 175], color, "  largest files (anywhere)")
        ));
        let biggest = files[0].0.max(1);
        for (size, path) in &files {
            let f = *size as f32 / biggest as f32;
            out.push_str(&format!(
                "  {} {}  {}\n",
                paint(heat(f), color, &fmt::bar(f, 10)),
                paint([226, 226, 236], color, &format!("{:>7}", fmt::human(*size))),
                path.display(),
            ));
        }
        out.push('\n');
    }

    if errors > 0 {
        out.push_str(&format!(
            "  {} {} unreadable entries\n",
            paint([255, 214, 130], color, "!"),
            fmt::group(errors)
        ));
        if let Some(err) = scan.first_error() {
            out.push_str(&format!("    first error: {err}\n"));
        }
    }
    let _: NodeId = root;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::Options;

    #[test]
    fn renders_a_report() {
        let base = std::env::temp_dir().join(format!("bloat-report-{}", std::process::id()));
        std::fs::create_dir_all(base.join("big")).unwrap();
        std::fs::write(base.join("big/b.bin"), vec![0u8; 65536]).unwrap();
        std::fs::write(base.join("small.bin"), vec![0u8; 1024]).unwrap();

        let opts = Options::default();
        let scan = Scan::new(&base, opts).unwrap();
        scan.start_thread();
        scan.wait();
        let text = render(&scan, 5, false);
        assert!(text.contains("bloat"));
        assert!(text.contains("largest entries"));
        assert!(text.contains("big"));
        assert!(text.contains("largest files"));
        assert!(text.contains("b.bin"));
        assert!(!text.contains('\x1b'), "no colours when disabled");
        let colored = render(&scan, 5, true);
        assert!(colored.contains("\x1b[38;2;"));
        std::fs::remove_dir_all(&base).unwrap();
    }
}