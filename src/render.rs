//! Cushion treemap: lays the tree out in half-block pixel space, then paints it
//! into ratatui cells with truecolor (`▀` with fg=bottom pixel, bg=top pixel).

use crate::arena::{Arena, NodeId};
use crate::fmt;
use crate::palette::{self, Rgb};
use crate::treemap::{squarify, SubRect};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect as CellRect;
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthChar;

const EMPTY: u32 = u32::MAX;
const BG: Rgb = [16, 16, 20];
/// Upper bound on children drawn per directory; the rest is aggregated.
pub const ITEM_LIMIT: usize = 32;
/// Roughly how many pixels an item needs to stay visible and clickable.
const PIXELS_PER_ITEM: usize = 128;

#[derive(Debug, Clone)]
pub struct Region {
    pub node: NodeId,
    pub rect: SubRect,
    pub is_rest: bool,
    pub rest_count: u64,
    pub size: u64,
    pub color: Rgb,
}

/// One pixel per half-block subcell (1 cell wide, half a cell tall).
pub struct Raster {
    pub w: u32,
    pub h: u32,
    rgb: Vec<Rgb>,
    owner: Vec<u32>,
    bg: Rgb,
}

impl Raster {
    fn new(w: u32, h: u32, bg: Rgb) -> Self {
        let n = (w as usize) * (h as usize);
        Self {
            w,
            h,
            rgb: vec![bg; n],
            owner: vec![EMPTY; n],
            bg,
        }
    }

    #[inline]
    fn at(&self, x: u32, y: u32) -> Rgb {
        if x < self.w && y < self.h {
            self.rgb[(y as usize) * (self.w as usize) + x as usize]
        } else {
            self.bg
        }
    }

    #[inline]
    fn owner_at(&self, x: u32, y: u32) -> u32 {
        if x < self.w && y < self.h {
            self.owner[(y as usize) * (self.w as usize) + x as usize]
        } else {
            EMPTY
        }
    }

    /// Fill with a cushion gradient: brightest at the top-left of the region,
    /// darker towards the bottom-right. Gives the baobab-style 3D look.
    fn fill(&mut self, r: SubRect, color: Rgb, owner: u32) {
        let x0 = r.x.min(self.w);
        let x1 = (r.x + r.w).min(self.w);
        let y0 = r.y.min(self.h);
        let y1 = (r.y + r.h).min(self.h);
        let iw = (r.w.max(2) - 1) as f32;
        let ih = (r.h.max(2) - 1) as f32;
        for y in y0..y1 {
            for x in x0..x1 {
                let nx = (x - r.x) as f32 / iw;
                let ny = (y - r.y) as f32 / ih;
                let f = 1.20 - 0.40 * (0.42 * nx + 0.58 * ny);
                let i = (y as usize) * (self.w as usize) + x as usize;
                self.rgb[i] = palette::shade(color, f);
                self.owner[i] = owner;
            }
        }
    }

    /// One-pixel darker frame, so neighbouring regions stay readable.
    fn outline(&mut self, r: SubRect, color: Rgb) {
        if r.w < 3 || r.h < 3 {
            return;
        }
        let x0 = r.x;
        let x1 = r.x + r.w - 1;
        let y0 = r.y;
        let y1 = r.y + r.h - 1;
        let c = palette::shade(color, 0.45);
        for x in x0..=x1 {
            self.put(x, y0, c);
            self.put(x, y1, c);
        }
        for y in y0..=y1 {
            self.put(x0, y, c);
            self.put(x1, y, c);
        }
    }

    #[inline]
    fn put(&mut self, x: u32, y: u32, c: Rgb) {
        if x < self.w && y < self.h {
            let i = (y as usize) * (self.w as usize) + x as usize;
            self.rgb[i] = c;
        }
    }
}

#[derive(Debug, Clone)]
pub struct CellLabel {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub fg: Color,
    pub modifier: Modifier,
}

pub struct TreemapView {
    pub regions: Vec<Region>,
    pub labels: Vec<CellLabel>,
    pub raster: Raster,
    pub area: CellRect,
    pub max_depth: u16,
}

impl TreemapView {
    pub fn build(arena: &Arena, root: NodeId, area: CellRect, max_depth: u16) -> Self {
        let w = area.width as u32;
        let h = area.height as u32 * 2;
        let mut view = Self {
            regions: Vec::new(),
            labels: Vec::new(),
            raster: Raster::new(w, h, BG),
            area,
            max_depth,
        };
        if w == 0 || h == 0 {
            return view;
        }
        let full = SubRect { x: 0, y: 0, w, h };
        let root_color = palette::node_color(&arena.path_string(root), 0);
        view.raster.fill(full, palette::shade(root_color, 0.75), EMPTY);
        view.lay(arena, root, full, 0);
        view.make_labels(arena);
        view
    }

    fn lay(&mut self, arena: &Arena, node: NodeId, rect: SubRect, depth: u16) {
        if rect.w < 2 || rect.h < 2 {
            return;
        }
        let budget = (rect.area() as usize / PIXELS_PER_ITEM).clamp(2, ITEM_LIMIT);
        let mut limit = budget;
        let (mut items, mut rects) = (Vec::new(), Vec::new());
        let mut child_count;
        for _attempt in 0..5 {
            let (children, rest) = arena.top_children(node, limit);
            child_count = children.len();
            items = children
                .iter()
                .map(|c| {
                    let n = &arena.nodes[*c as usize];
                    (*c, n.size, false, 0)
                })
                .collect();
            if let Some((size, count, _files)) = rest {
                if size > 0 {
                    items.push((node, size, true, count));
                }
            }
            if items.is_empty() {
                return;
            }
            items.sort_by(|a, b| b.1.cmp(&a.1));
            let weights: Vec<u64> = items.iter().map(|i| i.1.max(1)).collect();
            rects.clear();
            squarify(&weights, rect, &mut rects);
            // Sub-pixel items are folded into the aggregate instead of vanishing.
            let dropped = rects.iter().filter(|r| r.w == 0 || r.h == 0).count();
            if dropped == 0 || limit <= 2 || child_count <= limit {
                break;
            }
            limit = limit.saturating_sub(dropped).max(2);
        }

        for (k, (nid, size, is_rest, rest_count)) in items.iter().enumerate() {
            let Some(r) = rects.get(k).copied() else {
                break;
            };
            if r.w == 0 || r.h == 0 {
                continue;
            }
            let color = if *is_rest {
                palette::shade(palette::node_color(&arena.path_string(node), depth), 0.8)
            } else {
                palette::node_color(&arena.path_string(*nid), depth + 1)
            };
            let idx = self.regions.len() as u32;
            self.regions.push(Region {
                node: *nid,
                rect: r,
                is_rest: *is_rest,
                rest_count: *rest_count,
                size: *size,
                color,
            });
            self.raster.fill(r, color, idx);
            self.raster.outline(r, color);
            if !*is_rest && depth + 1 < self.max_depth {
                let n = &arena.nodes[*nid as usize];
                if n.is_dir && r.w >= 5 && r.h >= 5 {
                    self.lay(arena, *nid, r.inset(1), depth + 1);
                }
            }
        }
    }

    /// Cell-space bounds of a region (local to `area`).
    fn cell_bounds(&self, idx: usize) -> (u16, u16, u16, u16) {
        let Some(r) = self.regions.get(idx).map(|r| r.rect) else {
            return (0, 0, 1, 1);
        };
        let x0 = r.x as u16;
        let y0 = (r.y / 2) as u16;
        let x1 = (r.x + r.w) as u16;
        let y1 = ((r.y + r.h + 1) / 2) as u16;
        (x0, y0, x1.max(x0 + 1), y1.max(y0 + 1))
    }

    fn make_labels(&mut self, arena: &Arena) {
        let mut order: Vec<usize> = (0..self.regions.len()).collect();
        order.sort_by_key(|i| std::cmp::Reverse(self.regions[*i].size));
        let mut occupied: Vec<(u16, u16, u16, u16)> = Vec::new();
        let mut labels: Vec<CellLabel> = Vec::new();

        for idx in order {
            let region = &self.regions[idx];
            let (x0, y0, x1, y1) = self.cell_bounds(idx);
            let w = x1.saturating_sub(x0);
            let h = y1.saturating_sub(y0);
            if w < 6 || h < 2 {
                continue;
            }
            let name = if region.is_rest {
                format!("+{} more", fmt::group(region.rest_count))
            } else {
                arena.basename(region.node).to_string()
            };
            let size = fmt::human(region.size);
            let fg = palette::text_on(region.color);

            for row in y0..(y0 + 3).min(y1) {
                let row_rect = (x0, row, x1, row + 1);
                if occupied.iter().any(|r| overlaps(*r, row_rect)) {
                    continue;
                }
                let inner = w - 1;
                let mut cursor_right = x1 - 1;
                let size_w = fmt::width(&size);
                if inner as usize >= size_w + 4 {
                    let sx = cursor_right - size_w as u16;
                    labels.push(CellLabel {
                        x: sx,
                        y: row,
                        text: size.clone(),
                        fg,
                        modifier: Modifier::empty(),
                    });
                    cursor_right = sx - 1;
                }
                let room = cursor_right.saturating_sub(x0 + 1) as usize;
                if room >= 2 {
                    let text = fmt::truncate(&name, room);
                    labels.push(CellLabel {
                        x: x0 + 1,
                        y: row,
                        text,
                        fg,
                        modifier: Modifier::BOLD,
                    });
                }
                occupied.push((x0, row, x1, row + 1));
                break;
            }
        }
        self.labels = labels;
    }

    /// Centre of a region in cell coordinates, for spatial navigation.
    pub fn region_center(&self, idx: usize) -> (f32, f32) {
        let (x0, y0, x1, y1) = self.cell_bounds(idx);
        ((x0 as f32 + x1 as f32) / 2.0, (y0 as f32 + y1 as f32) / 2.0)
    }

    pub fn hit(&self, cell_x: u16, cell_y: u16) -> Option<usize> {
        if cell_x >= self.area.width || cell_y >= self.area.height {
            return None;
        }
        let lx = cell_x as u32;
        let ly = cell_y as u32 * 2;
        let a = self.raster.owner_at(lx, ly);
        if a != EMPTY {
            return Some(a as usize);
        }
        let b = self.raster.owner_at(lx, ly + 1);
        if b != EMPTY {
            return Some(b as usize);
        }
        None
    }

    /// Paint every cell of the treemap area. `sel`/`hover` are region indices.
    pub fn render_into(&self, buf: &mut Buffer, sel: Option<usize>, hover: Option<usize>) {
        let sel = sel.filter(|i| *i < self.regions.len());
        let hover = hover.filter(|i| *i < self.regions.len());
        let area = self.area;
        let sel_bounds = sel.map(|i| self.cell_bounds(i));
        let hover_bounds = hover.filter(|h| Some(*h) != sel).map(|i| self.cell_bounds(i));

        for cy in 0..area.height {
            for cx in 0..area.width {
                let lx = cx as u32;
                let ly = cy as u32;
                let ot = self.raster.owner_at(lx, ly * 2);
                let ob = self.raster.owner_at(lx, ly * 2 + 1);
                let mut factor = 0.0f32;
                if let (Some(s), Some((x0, y0, x1, y1))) = (sel, sel_bounds) {
                    if ot == s as u32 || ob == s as u32 {
                        let perim =
                            cx <= x0 || cx + 1 >= x1 || cy <= y0 || cy + 1 >= y1;
                        factor = if perim { 0.80 } else { 0.30 };
                    }
                }
                if factor == 0.0 {
                    if let (Some(h), Some((x0, y0, x1, y1))) = (hover, hover_bounds) {
                        if ot == h as u32 || ob == h as u32 {
                            let perim = cx <= x0 || cx + 1 >= x1 || cy <= y0 || cy + 1 >= y1;
                            factor = if perim { 0.55 } else { 0.14 };
                        }
                    }
                }
                let top = self.raster.at(lx, ly * 2);
                let bottom = self.raster.at(lx, ly * 2 + 1);
                let (top, bottom) = if factor > 0.0 {
                    (
                        palette::mix(top, [255, 255, 255], factor),
                        palette::mix(bottom, [255, 255, 255], factor),
                    )
                } else {
                    (top, bottom)
                };
                let x = area.x + cx;
                let y = area.y + cy;
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol("▀");
                    cell.set_fg(Color::Rgb(bottom[0], bottom[1], bottom[2]));
                    cell.set_bg(Color::Rgb(top[0], top[1], top[2]));
                }
            }
        }

        for label in &self.labels {
            let x = area.x + label.x;
            let y = area.y + label.y;
            if y >= area.y + area.height {
                continue;
            }
            let style = Style::new().fg(label.fg).add_modifier(label.modifier);
            let mut col = x;
            for ch in label.text.chars() {
                if col >= area.x + area.width {
                    break;
                }
                let lx = (col - area.x) as u32;
                let ly = (y - area.y) as u32;
                let bg = self.raster.at(lx, ly * 2);
                if let Some(cell) = buf.cell_mut((col, y)) {
                    cell.set_symbol(ch.to_string().as_str());
                    cell.set_fg(style.fg.unwrap_or(Color::Reset));
                    cell.set_bg(Color::Rgb(bg[0], bg[1], bg[2]));
                    if style.add_modifier.contains(Modifier::BOLD) {
                        cell.modifier.insert(Modifier::BOLD);
                    }
                }
                let wide = ch.width().unwrap_or(1) > 1;
                col += 1;
                if wide {
                    if let Some(cell) = buf.cell_mut((col, y)) {
                        cell.set_symbol("");
                    }
                    col += 1;
                }
            }
        }
    }
}

fn overlaps(a: (u16, u16, u16, u16), b: (u16, u16, u16, u16)) -> bool {
    a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> (Arena, NodeId) {
        let mut a = Arena::new_root("/".into());
        let big = a.add(0, "big".into(), true, 0, 0);
        a.add(big, "b1".into(), false, 8_000_000, 0);
        a.add(big, "b2".into(), false, 4_000_000, 0);
        let small = a.add(0, "small".into(), true, 0, 0);
        a.add(small, "s1".into(), false, 1000, 0);
        a.add(0, "tiny.txt".into(), false, 512, 0);
        (a, 0)
    }

    #[test]
    fn lays_out_and_renders() {
        let (arena, root) = tree();
        let area = CellRect::new(0, 0, 60, 20);
        let view = TreemapView::build(&arena, root, area, 4);
        assert_eq!(view.raster.w, 60);
        assert_eq!(view.raster.h, 40);
        assert!(!view.regions.is_empty());
        // the dominant child covers the area; small siblings may be sub-pixel
        assert!(view.regions.iter().any(|r| r.size >= 8_000_000));

        // every pixel is owned by some region
        let owned = view.raster.owner.iter().filter(|o| **o != EMPTY).count();
        assert_eq!(owned, 60 * 40);

        let mut buf = Buffer::empty(area);
        view.render_into(&mut buf, Some(0), None);
        let cell = &buf[(0, 0)];
        assert_eq!(cell.symbol(), "▀");
        let (x0, y0, x1, y1) = view.cell_bounds(0);
        assert!(x1 > x0 && y1 > y0);
        assert!(view.hit(0, 0).is_some());
        assert!(view.hit(59, 19).is_some());
        assert!(view.hit(99, 99).is_none());
        // labels never exceed the area
        for l in &view.labels {
            assert!(l.x < area.width && l.y < area.height);
        }
    }

    #[test]
    fn handles_tiny_terminal() {
        let (arena, root) = tree();
        for (w, h) in [(1u16, 1u16), (2, 1), (3, 3), (4, 2)] {
            let area = CellRect::new(0, 0, w, h);
            let view = TreemapView::build(&arena, root, area, 3);
            let mut buf = Buffer::empty(area);
            view.render_into(&mut buf, Some(0), Some(1));
        }
    }

    #[test]
    fn big_directories_get_aggregated() {
        let mut a = Arena::new_root("/".into());
        for i in 0..100u64 {
            a.add(0, format!("f{i}"), false, (i + 1) * 1000, 0);
        }
        let area = CellRect::new(0, 0, 80, 24);
        let view = TreemapView::build(&a, 0, area, 3);
        assert!(view.regions.iter().any(|r| r.is_rest));
        assert!(view.regions.len() >= 20, "{} regions", view.regions.len());
        assert!(view.regions.iter().all(|r| r.rect.w > 0 && r.rect.h > 0));
    }
}