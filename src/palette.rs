//! Deterministic colours: the same path always gets the same hue, siblings
//! differ, depth slightly darkens.

pub type Rgb = [u8; 3];

/// Cheap FNV-1a over the path.
pub fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn node_color(key: &str, depth: u16) -> Rgb {
    let h = hash(key);
    let hue = (h % 360) as f32;
    let sat = 0.42 + ((h >> 12) % 22) as f32 / 100.0;
    let light = (0.56 - 0.045 * depth as f32).clamp(0.26, 0.60);
    hsl_to_rgb(hue, sat, light)
}

pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> Rgb {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [
        (((r1 + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
        (((g1 + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
        (((b1 + m) * 255.0).round()).clamp(0.0, 255.0) as u8,
    ]
}

pub fn shade(c: Rgb, f: f32) -> Rgb {
    [
        ((c[0] as f32 * f).round()).clamp(0.0, 255.0) as u8,
        ((c[1] as f32 * f).round()).clamp(0.0, 255.0) as u8,
        ((c[2] as f32 * f).round()).clamp(0.0, 255.0) as u8,
    ]
}

pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * t).round().clamp(0.0, 255.0) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * t).round().clamp(0.0, 255.0) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * t).round().clamp(0.0, 255.0) as u8,
    ]
}

pub fn luminance(c: Rgb) -> f32 {
    (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0
}

pub fn text_on(c: Rgb) -> ratatui::style::Color {
    if luminance(c) > 0.55 {
        ratatui::style::Color::Rgb(12, 12, 16)
    } else {
        ratatui::style::Color::Rgb(242, 242, 248)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_stable_and_visible() {
        let a = node_color("/home/x/node_modules", 1);
        assert_eq!(a, node_color("/home/x/node_modules", 1));
        assert_ne!(a, node_color("/home/x/target", 1));
        for depth in 0..8 {
            let c = node_color(&format!("/x/{depth}"), depth);
            assert!(luminance(c) > 0.05 && luminance(c) < 0.85, "{c:?}");
        }
    }

    #[test]
    fn hsl_edges() {
        assert_eq!(hsl_to_rgb(0.0, 1.0, 0.5), [255, 0, 0]);
        assert_eq!(hsl_to_rgb(120.0, 1.0, 0.5), [0, 255, 0]);
        assert_eq!(hsl_to_rgb(240.0, 1.0, 0.5), [0, 0, 255]);
    }
}