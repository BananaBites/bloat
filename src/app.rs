//! Interactive mode: event loop, navigation, live treemap rebuilds.

use crate::arena::{Arena, NodeId};
use crate::fmt;
use crate::render::TreemapView;
use crate::scan::{Options, Scan};
use crate::ui::{self, Detail, Header, HeaderState};
use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::Frame;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

struct DetailOwned {
    path: String,
    color: [u8; 3],
    size: u64,
    parent_size: u64,
    files: u64,
    mtime: i64,
    is_dir: bool,
    note: String,
}

pub struct App {
    scan: Arc<Scan>,
    root: PathBuf,
    opts: Options,
    links: bool,
    max_depth: u16,
    zoom: NodeId,
    history: Vec<NodeId>,
    view: TreemapView,
    view_area: Rect,
    selected: Option<usize>,
    selected_key: Option<(NodeId, bool)>,
    hover: Option<usize>,
    detail: Option<DetailOwned>,
    last_build: Instant,
    last_entries: u64,
    last_hover: Instant,
    last_title: Instant,
    dirty: bool,
    frozen: bool,
    help: bool,
    quit: bool,
    spin: u32,
    status: Option<(String, Instant)>,
}

/// Run the TUI until the user quits. Terminal setup/teardown is handled here.
pub fn run(scan: Arc<Scan>, root: PathBuf, opts: Options, links: bool, max_depth: u16) -> Result<()> {
    let mut app = App::new(scan, root, opts, links, max_depth);
    let mut terminal = ratatui::init();
    let mut out = io::stdout();
    let _ = execute!(out, EnableMouseCapture);
    let result = app.event_loop(&mut terminal);
    let _ = execute!(out, DisableMouseCapture);
    ratatui::restore();
    result
}

impl App {
    pub fn new(scan: Arc<Scan>, root: PathBuf, opts: Options, links: bool, max_depth: u16) -> Self {
        let now = Instant::now();
        Self {
            scan,
            root,
            opts,
            links,
            max_depth,
            zoom: 0,
            history: Vec::new(),
            view: TreemapView::build(&Arena::new_root("/".into()), 0, Rect::new(0, 0, 0, 0), 1),
            view_area: Rect::new(0, 0, 0, 0),
            selected: None,
            selected_key: None,
            hover: None,
            detail: None,
            last_build: now - Duration::from_secs(1),
            last_entries: u64::MAX,
            last_hover: now - Duration::from_secs(1),
            last_title: now - Duration::from_secs(1),
            dirty: true,
            frozen: false,
            help: false,
            quit: false,
            spin: 0,
            status: None,
        }
    }

    fn event_loop(&mut self, terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
        let mut out = io::stdout();
        while !self.quit {
            let _ = execute!(out, BeginSynchronizedUpdate);
            let drawn = terminal.draw(|f| self.draw(f));
            let _ = execute!(out, EndSynchronizedUpdate);
            drawn?;

            let scanning = !self.scan.prog.done.load(std::sync::atomic::Ordering::Relaxed)
                && !self.frozen;
            let timeout = if scanning { 90 } else { 220 };
            if event::poll(Duration::from_millis(timeout))? {
                match event::read()? {
                    Event::Key(k)
                        if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                    {
                        self.on_key(k)
                    }
                    Event::Mouse(m) => self.on_mouse(m),
                    Event::Resize(_, _) => self.dirty = true,
                    _ => {}
                }
            }
            self.on_tick();
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        if let Some((_, at)) = &self.status {
            if at.elapsed() > Duration::from_secs(5) {
                self.status = None;
            }
        }
        let done = self.scan.prog.done.load(std::sync::atomic::Ordering::Relaxed);
        if done || self.frozen {
            return;
        }
        let (bytes, entries, _, _, _) = self.scan.prog.snapshot();
        if self.last_title.elapsed() > Duration::from_secs(1) {
            self.last_title = Instant::now();
            let title = format!(
                "bloat · {} · {} entries",
                fmt::human(bytes),
                fmt::group(entries)
            );
            let mut out = io::stdout();
            let _ = write!(out, "\x1b]2;{title}\x07");
            let _ = out.flush();
        }
    }

    fn status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now()));
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.draw_into(f.buffer_mut(), area);
    }

    /// Everything the frame shows, drawn straight into a buffer (testable).
    fn draw_into(&mut self, buf: &mut Buffer, area: Rect) {
        let header_h = 1u16;
        let footer_h = if area.height >= 4 { 2 } else { 0 };
        let map_h = area.height.saturating_sub(header_h + footer_h);
        let header_area = Rect::new(area.x, area.y, area.width, header_h);
        let map_area = Rect::new(area.x, area.y + header_h, area.width, map_h);
        let detail_area = Rect::new(area.x, map_area.y + map_h, area.width, u16::from(footer_h > 0));
        let hints_area = Rect::new(
            area.x,
            detail_area.y + detail_area.height,
            area.width,
            u16::from(footer_h > 1),
        );

        let (bytes, entries, _, _, done) = self.scan.prog.snapshot();
        // Live updates while scanning (throttled), plus a guaranteed final
        // rebuild once the walk is finished.
        let changed = entries != self.last_entries;
        let stale = changed
            && (done || (!self.frozen && self.last_build.elapsed() > Duration::from_millis(220)));
        if self.dirty || stale || self.view_area != map_area {
            self.rebuild(map_area);
        }

        let crumbs = {
            let arena = self.scan.arena_lock();
            crumb_chain(&arena, self.zoom)
        };
        let (size, files) = {
            let arena = self.scan.arena_lock();
            let n = &arena.nodes[self.zoom as usize];
            (n.size, n.files)
        };
        let current = self.scan.prog.current();
        let state = if done {
            HeaderState::Idle
        } else if self.frozen {
            HeaderState::Paused { bytes, entries }
        } else {
            HeaderState::Scanning {
                spin: SPINNER[(self.spin as usize / 2) % SPINNER.len()],
                bytes,
                entries,
                current: &current,
                phase: (self.spin % 90) as f32 / 90.0,
            }
        };
        let header = Header {
            crumbs,
            size,
            files,
            state,
        };

        self.view.render_into(buf, self.selected, self.hover);
        ui::draw_header(buf, header_area, &header, self.links);
        let detail = self.detail.as_ref().map(|d| Detail {
            path: &d.path,
            color: d.color,
            size: d.size,
            parent_size: d.parent_size,
            files: d.files,
            mtime: d.mtime,
            is_dir: d.is_dir,
            note: &d.note,
        });
        ui::draw_detail(buf, detail_area, detail.as_ref(), self.links);
        ui::draw_hints(
            buf,
            hints_area,
            self.status.as_ref().map(|(m, _)| m.as_str()),
        );
        if self.help {
            ui::draw_help(buf, area);
        }
        if self.scan.prog.done.load(std::sync::atomic::Ordering::Relaxed)
            && self.status.is_none()
            && self.scan.prog.errors.load(std::sync::atomic::Ordering::Relaxed) > 0
        {
            ui::draw_hints(buf, hints_area, Some("some entries were unreadable (permissions?)"));
        }
        if !self.scan.prog.done.load(std::sync::atomic::Ordering::Relaxed) {
            self.spin = self.spin.wrapping_add(1);
        }
    }

    fn rebuild(&mut self, area: Rect) {
        {
            let arena = self.scan.arena_lock();
            self.view = TreemapView::build(&arena, self.zoom, area, self.max_depth);
            let wanted = self.selected_key;
            // Keep the user's selection across live rebuilds; if it is gone
            // (rescanned, layout changed), fall back to the biggest region.
            let idx = wanted
                .and_then(|(node, is_rest)| {
                    self.view
                        .regions
                        .iter()
                        .position(|r| r.node == node && r.is_rest == is_rest)
                })
                .or_else(|| {
                    self.view
                        .regions
                        .iter()
                        .enumerate()
                        .max_by_key(|(_, r)| r.size)
                        .map(|(i, _)| i)
                });
            drop(arena);
            self.view_area = area;
            self.selected = idx;
        }
        self.last_build = Instant::now();
        self.last_entries = self.scan.prog.entries.load(std::sync::atomic::Ordering::Relaxed);
        self.dirty = false;
        self.refresh_detail();
    }

    fn refresh_detail(&mut self) {
        let Some(i) = self.selected else {
            self.detail = None;
            self.selected_key = None;
            return;
        };
        let Some(region) = self.view.regions.get(i) else {
            self.detail = None;
            return;
        };
        let (node, is_rest, rest_count, size, color) =
            (region.node, region.is_rest, region.rest_count, region.size, region.color);
        self.selected_key = Some((node, is_rest));
        let arena = self.scan.arena_lock();
        let n = &arena.nodes[node as usize];
        let parent_size = n
            .parent
            .map(|p| arena.nodes[p as usize].size)
            .unwrap_or(size);
        let path = arena.path_string(node);
        let (files, mtime, is_dir) = (n.files, n.mtime, n.is_dir);
        drop(arena);
        self.detail = Some(DetailOwned {
            path,
            color,
            size,
            parent_size,
            files,
            mtime,
            is_dir,
            note: if is_rest {
                format!(
                    "aggregated region · {} entries too small to draw",
                    fmt::group(rest_count)
                )
            } else {
                String::new()
            },
        });
    }

    fn set_selected(&mut self, idx: Option<usize>) {
        self.selected = idx;
        self.refresh_detail();
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.help {
            match k.code {
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter => self.help = false,
                _ => {}
            }
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('u') => self.go_up(),
            KeyCode::Enter => self.descend(),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(Dir::Up),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(Dir::Down),
            KeyCode::Left | KeyCode::Char('h') => self.move_selection(Dir::Left),
            KeyCode::Right | KeyCode::Char('l') => self.move_selection(Dir::Right),
            KeyCode::Char('[') => {
                self.max_depth = self.max_depth.saturating_sub(1).max(1);
                self.dirty = true;
            }
            KeyCode::Char(']') => {
                self.max_depth = (self.max_depth + 1).min(12);
                self.dirty = true;
            }
            KeyCode::Char(' ') => {
                self.frozen = !self.frozen;
                self.status(if self.frozen {
                    "live updates frozen (space to resume)"
                } else {
                    "live updates resumed"
                });
            }
            KeyCode::Char('y') => self.yank(),
            KeyCode::Char('r') => self.rescan(),
            _ => {}
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let inside = m.column >= self.view_area.x
            && m.row >= self.view_area.y
            && m.column < self.view_area.x + self.view_area.width
            && m.row < self.view_area.y + self.view_area.height;
        match m.kind {
            MouseEventKind::Moved => {
                if self.last_hover.elapsed() < Duration::from_millis(25) {
                    return;
                }
                self.last_hover = Instant::now();
                let hit = if inside {
                    self.view
                        .hit(m.column - self.view_area.x, m.row - self.view_area.y)
                } else {
                    None
                };
                if hit != self.hover {
                    self.hover = hit;
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    return;
                }
                let hit = self
                    .view
                    .hit(m.column - self.view_area.x, m.row - self.view_area.y);
                if let Some(i) = hit {
                    if self.selected == Some(i) {
                        self.descend();
                    } else {
                        self.set_selected(Some(i));
                    }
                }
            }
            MouseEventKind::ScrollDown => self.move_selection(Dir::Down),
            MouseEventKind::ScrollUp => self.move_selection(Dir::Up),
            _ => {}
        }
    }

    fn move_selection(&mut self, dir: Dir) {
        let n = self.view.regions.len();
        if n == 0 {
            return;
        }
        let cur = self.selected.unwrap_or(0).min(n - 1);
        let centers: Vec<(f32, f32)> = (0..n).map(|i| self.view.region_center(i)).collect();
        let (cx, cy) = centers[cur];
        let mut best: Option<(f32, usize)> = None;
        for (i, (x, y)) in centers.iter().enumerate() {
            if i == cur {
                continue;
            }
            let (dx, dy) = (x - cx, y - cy);
            let (primary, secondary) = match dir {
                Dir::Left => (-dx, dy.abs()),
                Dir::Right => (dx, dy.abs()),
                Dir::Up => (-dy, dx.abs()),
                Dir::Down => (dy, dx.abs()),
            };
            if primary <= 0.5 {
                continue;
            }
            let score = primary + secondary * 2.5;
            if best.is_none_or(|(b, _)| score < b) {
                best = Some((score, i));
            }
        }
        if let Some((_, i)) = best {
            self.set_selected(Some(i));
        }
    }

    fn descend(&mut self) {
        let Some(i) = self.selected else {
            return;
        };
        let Some(region) = self.view.regions.get(i) else {
            return;
        };
        if region.is_rest {
            self.status("aggregated region — open a directory instead");
            return;
        }
        let node = region.node;
        let is_dir = {
            let arena = self.scan.arena_lock();
            arena.nodes[node as usize].is_dir
        };
        if !is_dir {
            self.status("that is a file, not a directory");
            return;
        }
        self.history.push(self.zoom);
        self.zoom = node;
        self.selected_key = None;
        self.dirty = true;
    }

    fn go_up(&mut self) {
        if let Some(prev) = self.history.pop() {
            self.zoom = prev;
        } else {
            let parent = {
                let arena = self.scan.arena_lock();
                arena.nodes[self.zoom as usize].parent
            };
            match parent {
                Some(p) => self.zoom = p,
                None => {
                    self.status("already at the scan root");
                    return;
                }
            }
        }
        self.selected_key = None;
        self.dirty = true;
    }

    fn yank(&mut self) {
        let Some(path) = self.detail.as_ref().map(|d| d.path.clone()) else {
            self.status("nothing selected to copy");
            return;
        };
        let payload = base64(path.as_bytes());
        let mut out = io::stdout();
        let _ = write!(out, "\x1b]52;c;{payload}\x07");
        let _ = out.flush();
        self.status(format!("copied to clipboard: {}", fmt::truncate_start(&path, 60)));
    }

    fn rescan(&mut self) {
        match Scan::new(&self.root, self.opts.clone()) {
            Ok(scan) => {
                scan.start_thread();
                self.scan = scan;
                self.zoom = 0;
                self.history.clear();
                self.selected_key = None;
                self.last_entries = u64::MAX;
                self.dirty = true;
                self.status("rescanning…");
            }
            Err(e) => self.status(format!("rescan failed: {e}")),
        }
    }
}

fn crumb_chain(arena: &Arena, node: NodeId) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur = Some(node);
    while let Some(c) = cur {
        let n = &arena.nodes[c as usize];
        out.push((n.name.clone(), arena.path_string(c)));
        cur = n.parent;
    }
    out.reverse();
    if out.len() > 6 {
        let tail = out.split_off(out.len() - 5);
        out = vec![("…".to_string(), tail[0].1.clone())];
        out.extend(tail);
    }
    out
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// OSC 8 hyperlinks: only where they are known not to be mangled.
pub fn hyperlinks_supported() -> bool {
    if let Ok(v) = std::env::var("TERM_PROGRAM") {
        let v = v.to_ascii_lowercase();
        if [
            "iterm.app",
            "wezterm",
            "vscode",
            "apple_terminal",
            "ghostty",
            "hyper",
            "tabby",
            "warpterminal",
        ]
        .iter()
        .any(|k| v.contains(k))
        {
            return true;
        }
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var_os("WT_SESSION").is_some()
        || std::env::var_os("VTE_VERSION")
            .map(|v| v.to_string_lossy().parse::<u32>().unwrap_or(0) >= 6000)
            .unwrap_or(false)
    {
        return true;
    }
    let term = std::env::var("TERM").unwrap_or_default().to_ascii_lowercase();
    ["kitty", "wezterm", "foot", "ghostty", "alacritty"]
        .iter()
        .any(|k| term.contains(k))
}

/// True when stdout can host a TUI.
pub fn interactive() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    !term.is_empty() && term != "dumb"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"/home/hannes"), "L2hvbWUvaGFubmVz");
    }

    fn frame_to_text(buf: &Buffer) -> String {
        let area = buf.area;
        let mut out = String::new();
        for y in 0..area.height {
            let mut line = String::new();
            for x in 0..area.width {
                let sym = buf[(x, y)].symbol();
                if sym.is_empty() {
                    continue; // continuation cell of a wide glyph
                }
                line.push_str(sym);
            }
            out.push_str(&line.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn renders_a_full_frame() {
        let base = std::env::temp_dir().join(format!("bloat-frame-{}", std::process::id()));
        let (alpha, beta) = (base.join("alpha"), base.join("beta"));
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();
        std::fs::write(alpha.join("big.bin"), vec![0u8; 200_000]).unwrap();
        std::fs::write(beta.join("small.bin"), vec![0u8; 4_096]).unwrap();
        std::fs::write(base.join("loose.txt"), vec![0u8; 1_024]).unwrap();

        let opts = Options::default();
        let scan = Scan::new(&base, opts.clone()).unwrap();
        scan.start_thread();
        scan.wait();
        let mut app = App::new(scan, base.clone(), opts, true, 5);

        let area = Rect::new(0, 0, 100, 24);
        let mut buf = Buffer::empty(area);
        app.draw_into(&mut buf, area);
        let text = frame_to_text(&buf);
        println!("{text}");

        assert!(text.contains("alpha"), "header/breadcrumb missing root");
        assert!(text.contains('▀'), "no treemap pixels");
        assert!(text.contains("quit"), "key hints missing");
        assert!(text.contains("big.bin") || text.contains("beta"), "labels missing");
        assert!(app.detail.is_some(), "a region should be selected");

        // selection survives a live rebuild and descend/up cycles
        app.set_selected(Some(0));
        let first = app.selected;
        app.descend();
        app.draw_into(&mut buf, area);
        assert!(app.detail.is_some(), "detail lost after descend");
        app.draw_into(&mut buf, area);
        assert!(app.detail.is_some(), "detail lost after rebuild");
        app.go_up();
        app.draw_into(&mut buf, area);
        assert!(app.detail.is_some(), "detail lost after going up");
        let _ = first;

        // help overlay renders without touching the layout
        app.help = true;
        app.draw_into(&mut buf, area);
        let text = frame_to_text(&buf);
        assert!(text.contains("bloat — keys"));
        app.help = false;
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn crumbs_are_trimmed() {
        let mut a = Arena::new_root("/".into());
        let mut cur = 0;
        for name in ["a", "b", "c", "d", "e", "f", "g"] {
            cur = a.add(cur, name.into(), true, 0, 0);
        }
        let chain = crumb_chain(&a, cur);
        assert_eq!(chain.len(), 6);
        assert_eq!(chain[0].0, "…");
        assert_eq!(chain[1].0, "c");
        assert_eq!(chain.last().unwrap().0, "g");
    }
}