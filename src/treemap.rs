//! Squarified treemap layout, in half-block "pixel" units.

/// A rectangle in pixel units (one pixel = one half-block subcell: 1 cell wide,
/// half a cell tall, i.e. roughly square on a normal terminal).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl SubRect {
    pub fn inset(self, n: u32) -> Self {
        let twice = n.saturating_mul(2);
        if self.w <= twice || self.h <= twice {
            return Self {
                x: self.x,
                y: self.y,
                w: 0,
                h: 0,
            };
        }
        Self {
            x: self.x + n,
            y: self.y + n,
            w: self.w - twice,
            h: self.h - twice,
        }
    }

    pub fn area(self) -> u64 {
        self.w as u64 * self.h as u64
    }
}

/// Squarified layout of `weights` (descending, all > 0) into `area`.
/// Writes exactly `weights.len()` rectangles, in input order.
pub fn squarify(weights: &[u64], area: SubRect, out: &mut Vec<SubRect>) {
    out.clear();
    let n = weights.len();
    if n == 0 {
        return;
    }
    if area.w == 0 || area.h == 0 {
        out.resize(n, SubRect::default());
        return;
    }
    let total: u64 = weights.iter().sum();
    if total == 0 {
        out.resize(n, SubRect::default());
        return;
    }

    let mut rect = area;
    let mut i = 0usize;
    while i < n {
        if rect.w == 0 || rect.h == 0 {
            out.resize(n, SubRect::default());
            return;
        }
        let rem_total: u64 = weights[i..].iter().sum();
        if rem_total == 0 {
            out.resize(n, SubRect::default());
            return;
        }
        // The row is laid out along the *shorter* side of the remaining rect,
        // which keeps items close to square.
        let vertical = rect.w >= rect.h;
        let side = if vertical {
            rect.h as f64
        } else {
            rect.w as f64
        };
        let avail = rect.area() as f64;

        // Grow the current row while the worst aspect ratio keeps improving.
        let mut row_sum: u64 = 0;
        let mut count = 0usize;
        let mut best = f64::INFINITY;
        while i + count < n {
            let ns = row_sum + weights[i + count];
            let thickness = (((ns as f64 / rem_total as f64) * avail / side).round()).max(1.0);
            let mut worst = 0.0f64;
            for k in 0..=count {
                let len = ((weights[i + k] as f64 / ns as f64) * side).max(1.0);
                let aspect = (len / thickness).max(thickness / len);
                if aspect > worst {
                    worst = aspect;
                }
            }
            if worst <= best {
                best = worst;
                row_sum = ns;
                count += 1;
            } else {
                break;
            }
        }
        if count == 0 {
            count = 1;
        }

        let last_row = i + count >= n;
        let row_area = (row_sum as f64 / rem_total as f64) * avail;
        if vertical {
            let w = if last_row {
                rect.w
            } else {
                ((row_area / rect.h as f64).round() as u32).clamp(1, rect.w)
            };
            let heights = distribute(rect.h, &weights[i..i + count]);
            let mut y = rect.y;
            for h in heights.iter() {
                out.push(SubRect {
                    x: rect.x,
                    y,
                    w: rect.w.min(w),
                    h: *h,
                });
                y += h;
            }
            let w = w.min(rect.w);
            rect.x += w;
            rect.w -= w;
        } else {
            let h = if last_row {
                rect.h
            } else {
                ((row_area / rect.w as f64).round() as u32).clamp(1, rect.h)
            };
            let widths = distribute(rect.w, &weights[i..i + count]);
            let mut x = rect.x;
            for w in widths.iter() {
                out.push(SubRect {
                    x,
                    y: rect.y,
                    w: *w,
                    h: rect.h.min(h),
                });
                x += w;
            }
            let h = h.min(rect.h);
            rect.y += h;
            rect.h -= h;
        }
        i += count;
    }
    out.resize(n, SubRect::default());
}

/// Split `total` proportionally to `weights`, with no rounding drift and every
/// slice at least 1 unit wide (stealing from the biggest).
fn distribute(total: u32, weights: &[u64]) -> Vec<u32> {
    let n = weights.len();
    let mut out = vec![0u32; n];
    if n == 0 {
        return out;
    }
    let sum: u64 = weights.iter().sum();
    if sum == 0 || total == 0 {
        return out;
    }
    let mut acc = 0u64;
    let mut prev = 0u64;
    for i in 0..n {
        acc += weights[i];
        let cum = (acc * total as u64 + sum / 2) / sum;
        out[i] = cum.saturating_sub(prev) as u32;
        prev = cum;
    }
    let assigned: u64 = out.iter().map(|x| *x as u64).sum();
    let delta = total as i64 - assigned as i64;
    if delta != 0 {
        let last = n - 1;
        out[last] = (out[last] as i64 + delta).max(0) as u32;
    }
    if out.iter().all(|w| *w > 0) {
        return out;
    }
    for i in 0..n {
        if out[i] == 0 {
            if let Some(j) = (0..n).max_by_key(|k| out[*k]) {
                if out[j] > 1 {
                    out[j] -= 1;
                    out[i] = 1;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_the_area_exactly() {
        let area = SubRect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let weights = vec![1_000u64, 500, 300, 200, 100, 50, 1];
        let mut out = Vec::new();
        squarify(&weights, area, &mut out);
        assert_eq!(out.len(), weights.len());
        let covered: u64 = out.iter().map(|r| r.area()).sum();
        assert_eq!(covered, area.area(), "rects must tile the area");
        for r in &out {
            assert!(r.x >= area.x && r.y >= area.y);
            assert!(r.x + r.w <= area.x + area.w);
            assert!(r.y + r.h <= area.y + area.h);
            assert!(r.w > 0 && r.h > 0);
        }
    }

    #[test]
    fn distribute_is_exact() {
        for weights in [
            vec![1u64, 1, 1],
            vec![1, 2, 3],
            vec![1_000_000, 1, 1],
            vec![7, 7, 7, 7, 7],
        ] {
            let out = distribute(97, &weights);
            assert_eq!(out.iter().sum::<u32>(), 97);
            assert!(out.iter().all(|x| *x > 0));
        }
    }

    #[test]
    fn handles_tiny_areas() {
        let mut out = Vec::new();
        squarify(&[10, 5], SubRect { x: 0, y: 0, w: 1, h: 1 }, &mut out);
        assert_eq!(out.len(), 2);
        squarify(&[10], SubRect::default(), &mut out);
        assert_eq!(out.len(), 1);
    }
}