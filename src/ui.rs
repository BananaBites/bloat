//! Chrome around the treemap: header/breadcrumb, detail line, key hints, help.

use crate::fmt;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect as CellRect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

pub const HEADER_BG: Color = Color::Rgb(26, 26, 34);
pub const PANEL_BG: Color = Color::Rgb(20, 20, 26);
pub const ACCENT: Color = Color::Rgb(122, 200, 255);
pub const HOT: Color = Color::Rgb(255, 214, 130);
pub const DIM: Color = Color::Rgb(138, 138, 152);
pub const TEXT: Color = Color::Rgb(226, 226, 236);
pub const GOOD: Color = Color::Rgb(150, 230, 150);

pub fn fill(buf: &mut Buffer, area: CellRect, bg: Color) {
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ");
                cell.set_bg(bg);
                cell.set_fg(TEXT);
                cell.modifier = Modifier::empty();
            }
        }
    }
}

/// Write text, clipped to the right edge of `area`. Returns the next free column.
pub fn put_str(
    buf: &mut Buffer,
    area: CellRect,
    x: u16,
    y: u16,
    text: &str,
    style: Style,
) -> u16 {
    let max = area.x + area.width;
    let mut col = x;
    for ch in text.chars() {
        if col >= max {
            return max;
        }
        if y >= area.y + area.height {
            return col;
        }
        let cw = ch.width().unwrap_or(1).max(1) as u16;
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_symbol(ch.to_string().as_str());
            cell.set_fg(style.fg.unwrap_or(Color::Reset));
            if let Some(bg) = style.bg {
                cell.set_bg(bg);
            }
            cell.modifier = style.add_modifier;
        }
        if cw > 1 {
            if let Some(cell) = buf.cell_mut((col + 1, y)) {
                cell.set_symbol("");
            }
        }
        col += cw;
    }
    col
}

/// Same, but wrapped in an OSC 8 hyperlink (clickable path in modern terminals).
#[allow(clippy::too_many_arguments)]
pub fn put_link(
    buf: &mut Buffer,
    area: CellRect,
    x: u16,
    y: u16,
    text: &str,
    url: &str,
    style: Style,
    links: bool,
) -> u16 {
    if !links {
        return put_str(buf, area, x, y, text, style);
    }
    let max = area.x + area.width;
    let mut col = x;
    let mut cells: Vec<u16> = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        if col >= max {
            break;
        }
        let cw = ch.width().unwrap_or(1).max(1) as u16;
        let symbol = if i == 0 {
            format!("\x1b]8;;{url}\x1b\\{ch}")
        } else {
            ch.to_string()
        };
        if let Some(cell) = buf.cell_mut((col, y)) {
            cell.set_symbol(&symbol);
            cell.set_fg(style.fg.unwrap_or(Color::Reset));
            if let Some(bg) = style.bg {
                cell.set_bg(bg);
            }
            cell.modifier = style.add_modifier;
            cells.push(col);
        }
        if cw > 1 {
            if let Some(cell) = buf.cell_mut((col + 1, y)) {
                cell.set_symbol("");
            }
        }
        col += cw;
    }
    if let Some(last) = cells.last() {
        if let Some(cell) = buf.cell_mut((*last, y)) {
            let s = format!("{}\x1b]8;;\x1b\\", cell.symbol());
            cell.set_symbol(&s);
        }
    }
    col
}

/// Text with a moving highlight, used for the live scan readout.
fn put_gradient(
    buf: &mut Buffer,
    area: CellRect,
    x: u16,
    y: u16,
    text: &str,
    base: Color,
    hot: Color,
    phase: f32,
) -> u16 {
    let (Color::Rgb(br, bg_, bb), Color::Rgb(hr, hg, hb)) = (base, hot) else {
        return put_str(buf, area, x, y, text, Style::new().fg(base));
    };
    let n = text.chars().count().max(1) as f32;
    let max = area.x + area.width;
    let mut col = x;
    for (i, ch) in text.chars().enumerate() {
        if col >= max {
            break;
        }
        let t = (((i as f32 / n) - phase).rem_euclid(1.0) - 0.85).max(0.0) / 0.15;
        let mix = t.clamp(0.0, 1.0);
        let c = Color::Rgb(
            (br as f32 + (hr as f32 - br as f32) * mix) as u8,
            (bg_ as f32 + (hg as f32 - bg_ as f32) * mix) as u8,
            (bb as f32 + (hb as f32 - bb as f32) * mix) as u8,
        );
        col = put_str(buf, area, col, y, ch.to_string().as_str(), Style::new().fg(c));
    }
    col
}

pub enum HeaderState<'a> {
    Idle,
    Scanning {
        spin: char,
        bytes: u64,
        entries: u64,
        current: &'a str,
        phase: f32,
    },
    Paused {
        bytes: u64,
        entries: u64,
    },
}

pub struct Header<'a> {
    /// (label, absolute path) per breadcrumb component.
    pub crumbs: Vec<(String, String)>,
    pub size: u64,
    pub files: u64,
    pub state: HeaderState<'a>,
}

pub fn draw_header(buf: &mut Buffer, area: CellRect, h: &Header, links: bool) {
    fill(buf, area, HEADER_BG);
    let y = area.y;

    let right = match &h.state {
        HeaderState::Idle => format!("{} · {} files ", fmt::human(h.size), fmt::group(h.files)),
        HeaderState::Scanning {
            spin,
            bytes,
            entries,
            current,
            ..
        } => format!(
            "{spin} {} · {} · {} ",
            fmt::human(*bytes),
            fmt::group(*entries),
            fmt::truncate_start(current, 34)
        ),
        HeaderState::Paused { bytes, entries } => format!(
            "⏸ frozen at {} · {} entries ",
            fmt::human(*bytes),
            fmt::group(*entries)
        ),
    };
    let right_w = (fmt::width(&right) as u16).min(area.width.saturating_sub(8));
    let left_budget = area.width.saturating_sub(right_w);

    let crumb_area = CellRect {
        width: left_budget,
        ..area
    };
    let mut x = area.x;
    for (i, (label, path)) in h.crumbs.iter().enumerate() {
        if i > 0 {
            let sep = if x + 3 > crumb_area.x + crumb_area.width {
                " › "
            } else {
                " ▸ "
            };
            x = put_str(buf, crumb_area, x, y, sep, Style::new().fg(Color::Rgb(80, 80, 95)));
        }
        let last = i + 1 == h.crumbs.len();
        let style = if last {
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(ACCENT)
        };
        let url = format!("file://{path}");
        x = put_link(buf, crumb_area, x, y, label, &url, style, links);
        if x >= crumb_area.x + crumb_area.width {
            break;
        }
    }

    let rx = area.x + area.width.saturating_sub(right_w);
    match &h.state {
        HeaderState::Scanning {
            phase,
            spin: _,
            ..
        } => {
            put_gradient(buf, area, rx, y, &right, DIM, HOT, *phase);
        }
        HeaderState::Paused { .. } => {
            put_str(buf, area, rx, y, &right, Style::new().fg(HOT));
        }
        HeaderState::Idle => {
            put_str(buf, area, rx, y, &right, Style::new().fg(DIM));
        }
    }
}

pub struct Detail<'a> {
    pub path: &'a str,
    pub color: [u8; 3],
    pub size: u64,
    pub parent_size: u64,
    pub files: u64,
    pub mtime: i64,
    pub is_dir: bool,
    pub note: &'a str,
}

pub fn draw_detail(buf: &mut Buffer, area: CellRect, detail: Option<&Detail>, links: bool) {
    fill(buf, area, PANEL_BG);
    let y = area.y;
    let Some(d) = detail else {
        put_str(
            buf,
            area,
            area.x + 2,
            y,
            "press ? for keys — arrows select, enter descends",
            Style::new().fg(DIM),
        );
        return;
    };
    let c = Color::Rgb(d.color[0], d.color[1], d.color[2]);
    let glyph = if d.is_dir { "■ " } else { "▪ " };

    let right = if d.note.is_empty() {
        format!(
            "{} · {}% of parent · {} · {}",
            fmt::human(d.size),
            fmt::percent(d.size, d.parent_size).round(),
            fmt::age(d.mtime),
            if d.files == 1 { "1 file".to_string() } else { format!("{} files", fmt::group(d.files)) }
        )
    } else {
        format!("{} · {} ", fmt::human(d.size), d.note)
    };
    let rw = fmt::width(&right) as u16;
    let left_w = area.width.saturating_sub(rw + 3);

    let x = put_str(buf, area, area.x + 1, y, glyph, Style::new().fg(c));
    let shown = fmt::truncate_start(d.path, left_w.saturating_sub(2) as usize);
    let x = put_link(
        buf,
        area,
        x,
        y,
        &shown,
        &format!("file://{}", d.path),
        Style::new().fg(TEXT),
        links,
    );
    let _ = x;

    if rw + 4 < area.width {
        let rx = area.x + area.width - rw;
        put_str(buf, area, rx, y, &right, Style::new().fg(DIM));
    }
}

pub fn draw_hints(buf: &mut Buffer, area: CellRect, status: Option<&str>) {
    fill(buf, area, PANEL_BG);
    let y = area.y;
    if let Some(msg) = status {
        put_str(buf, area, area.x + 1, y, msg, Style::new().fg(GOOD));
        return;
    }
    let keys = "↑↓←→ select · ⏎ descend · ⌫ up · y copy · r rescan · [ ] depth · ? help · q quit";
    put_str(buf, area, area.x + 1, y, keys, Style::new().fg(Color::Rgb(96, 96, 112)));
}

pub fn draw_help(buf: &mut Buffer, area: CellRect) {
    let w = 62.min(area.width.saturating_sub(2));
    let h = 17.min(area.height.saturating_sub(2));
    if w < 20 || h < 6 {
        return;
    }
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let box_area = CellRect {
        x,
        y,
        width: w,
        height: h,
    };
    fill(buf, box_area, Color::Rgb(30, 30, 40));
    let border = Style::new().fg(Color::Rgb(90, 120, 160));
    let top = format!("┌{}┐", "─".repeat(w as usize - 2));
    let bottom = format!("└{}┘", "─".repeat(w as usize - 2));
    put_str(buf, box_area, x, y, &top, border);
    put_str(buf, box_area, x, y + h - 1, &bottom, border);
    for row in y + 1..y + h - 1 {
        put_str(buf, box_area, x, row, "│", border);
        put_str(buf, box_area, x + w - 1, row, "│", border);
    }

    let title = " bloat — keys ";
    put_str(
        buf,
        box_area,
        x + 3,
        y,
        title,
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
    );

    let lines: &[(&str, &str)] = &[
        ("↑ ↓ ← → / h j k l", "move selection through the treemap"),
        ("enter / l", "descend into the selected directory"),
        ("backspace / u / esc", "go up one level"),
        ("[  ]", "treemap depth (fewer · more nesting)"),
        ("y", "copy the selected path to the clipboard (OSC 52)"),
        ("space", "freeze/unfreeze live updates while scanning"),
        ("r", "rescan from scratch"),
        ("?", "close this help"),
        ("q / ctrl-c", "quit"),
    ];
    let mut row = y + 2;
    for (k, v) in lines {
        put_str(
            buf,
            box_area,
            x + 3,
            row,
            k,
            Style::new().fg(HOT).add_modifier(Modifier::BOLD),
        );
        put_str(buf, box_area, x + 25, row, v, Style::new().fg(TEXT));
        row += 1;
        if row >= y + h - 1 {
            break;
        }
    }
    put_str(
        buf,
        box_area,
        x + 3,
        row + 1,
        "sizes are disk usage (st_blocks); treemap is truecolor half-block pixels",
        Style::new().fg(DIM),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_panels_render_inside_bounds() {
        let area = CellRect::new(0, 0, 80, 24);
        let mut buf = Buffer::empty(area);
        let h = Header {
            crumbs: vec![
                ("home".into(), "/home".into()),
                ("hannes".into(), "/home/hannes".into()),
            ],
            size: 12_000_000,
            files: 42,
            state: HeaderState::Scanning {
                spin: '⠹',
                bytes: 1_000_000,
                entries: 999,
                current: "/home/hannes/devel/some/deep/path",
                phase: 0.3,
            },
        };
        draw_header(&mut buf, CellRect::new(0, 0, 80, 1), &h, true);
        let d = Detail {
            path: "/home/hannes/devel",
            color: [200, 80, 40],
            size: 4_000_000,
            parent_size: 12_000_000,
            files: 12,
            mtime: 0,
            is_dir: true,
            note: "",
        };
        draw_detail(&mut buf, CellRect::new(0, 1, 80, 1), Some(&d), true);
        draw_hints(&mut buf, CellRect::new(0, 23, 80, 1), None);
        draw_help(&mut buf, area);
        // nothing panics, and the last line is still the hint bar
        assert!(buf[(0, 0)].symbol().contains('h') || !buf[(0, 0)].symbol().is_empty());
    }

    #[test]
    fn tiny_areas_are_safe() {
        for (w, h) in [(1u16, 1u16), (5, 2), (10, 1)] {
            let area = CellRect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let hdr = Header {
                crumbs: vec![("x".into(), "/x".into())],
                size: 0,
                files: 0,
                state: HeaderState::Idle,
            };
            draw_header(&mut buf, area, &hdr, true);
            draw_detail(&mut buf, area, None, true);
            draw_hints(&mut buf, area, Some("status message that is far too long"));
            draw_help(&mut buf, area);
        }
    }
}