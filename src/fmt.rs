//! Small formatting helpers (sizes, counts, width-aware truncation).

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];

/// Human size, binary units: 813B, 4.0K, 1.2M, 12G.
pub fn human(bytes: u64) -> String {
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    match i {
        0 => format!("{bytes}B"),
        _ if v < 10.0 => format!("{v:.1}{}", UNITS[i]),
        _ => format!("{v:.0}{}", UNITS[i]),
    }
}

/// Thousands separators: 1234567 -> 1,234,567
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate to `max` columns, marking the cut with a single `…`.
pub fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Keep the tail, which is usually the informative part of a path.
pub fn truncate_start(s: &str, max: usize) -> String {
    if width(s) <= max || max == 0 {
        return s.to_string();
    }
    let mut out: Vec<char> = Vec::new();
    let mut w = 0usize;
    for c in s.chars().rev() {
        let cw = c.width().unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.reverse();
    format!("…{}", out.into_iter().collect::<String>())
}

/// Relative age, coarse: "2h", "3d", "5 months".
pub fn age(mtime: i64) -> String {
    if mtime <= 0 {
        return "-".into();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let secs = (now - mtime).max(0);
    let mins = secs / 60;
    match mins {
        0 => "now".into(),
        1..=59 => format!("{mins}m"),
        _ => {
            let hours = mins / 60;
            if hours < 48 {
                format!("{hours}h")
            } else {
                let days = hours / 24;
                if days < 60 {
                    format!("{days}d")
                } else {
                    let months = days / 30;
                    if months < 24 {
                        format!("{months}mo")
                    } else {
                        format!("{}y", days / 365)
                    }
                }
            }
        }
    }
}

pub fn bar(fraction: f32, width: usize) -> String {
    let f = fraction.clamp(0.0, 1.0);
    let filled = (f * width as f32).round() as usize;
    let mut s = String::new();
    for i in 0..width {
        s.push(if i < filled { '█' } else { '·' });
    }
    s
}

pub fn percent(part: u64, whole: u64) -> f32 {
    if whole == 0 {
        0.0
    } else {
        part as f32 * 100.0 / whole as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humans() {
        assert_eq!(human(0), "0B");
        assert_eq!(human(813), "813B");
        assert_eq!(human(4096), "4.0K");
        assert_eq!(human(1024 * 1024 * 3 / 2), "1.5M");
        assert_eq!(human(13 * 1024 * 1024 * 1024), "13G");
    }

    #[test]
    fn groups() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1000), "1,000");
        assert_eq!(group(1234567), "1,234,567");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("abcdef", 10), "abcdef");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate_start("/a/b/c/verylong", 8), "…erylong");
        assert!(width(&truncate_start("/a/b/c/verylong", 8)) <= 8);
        assert_eq!(truncate_start("/a/b/c/verylong", 8), "…erylong");
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本"), 4);
    }

    #[test]
    fn bars() {
        assert_eq!(bar(0.0, 4), "····");
        assert_eq!(bar(0.5, 4), "██··");
        assert_eq!(bar(1.0, 4), "████");
        assert_eq!(percent(1, 4), 25.0);
    }
}