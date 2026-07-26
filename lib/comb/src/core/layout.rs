//! Constraint-based layout: split a [`Rect`] into rows or columns using
//! declarative constraints, ratatui-style but without the allocator ceremony.
//!
//! ```ignore
//! use comb::{Rect, layout};
//! let area = Rect::new(0, 0, 80, 24);
//! let [header, body, footer] = layout::vertical(area, [
//!     layout::Length(3),
//!     layout::Min(0),
//!     layout::Length(1),
//! ]);
//! ```

use crate::core::geom::Rect;

/// A sizing constraint for one segment of a layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    /// Exactly `n` cells.
    Length(u16),
    /// At least `n` cells; grows to fill surplus space.
    Min(u16),
    /// At most `n` cells; shrinks when space is tight.
    Max(u16),
    /// Proportional share — `n` parts out of the total.
    Ratio(u16, u16),
    /// Whatever remains after other constraints settle.
    Fill,
    /// A percentage of the available space (0..=100).
    Percentage(u16),
}

impl Constraint {
    /// `Constraint::Length(n)` shorthand.
    pub const fn len(n: u16) -> Self {
        Constraint::Length(n)
    }

    /// `Constraint::Min(n)` shorthand.
    pub const fn min(n: u16) -> Self {
        Constraint::Min(n)
    }

    /// `Constraint::Max(n)` shorthand.
    pub const fn max(n: u16) -> Self {
        Constraint::Max(n)
    }

    /// `Constraint::Fill` shorthand.
    pub const fn fill() -> Self {
        Constraint::Fill
    }
}

/// Compute the size (width or height) each constraint gets from `total`.
fn resolve(constraints: &[Constraint], total: u16) -> Vec<u16> {
    let n = constraints.len();
    if n == 0 || total == 0 {
        return vec![0; n];
    }

    let mut sizes = vec![0u16; n];
    let mut remaining = total;

    // Pass 1: fixed and capped constraints.
    for (i, c) in constraints.iter().enumerate() {
        let want = match c {
            Constraint::Length(v) => *v,
            Constraint::Max(v) => remaining.min(*v),
            Constraint::Percentage(p) => ((total as u32 * *p as u32) / 100) as u16,
            Constraint::Ratio(num, den) => {
                if *den == 0 {
                    0
                } else {
                    ((total as u32 * *num as u32) / *den as u32) as u16
                }
            }
            Constraint::Min(v) => remaining.min(*v),
            Constraint::Fill => 0,
        };
        sizes[i] = want.min(remaining);
        remaining = remaining.saturating_sub(sizes[i]);
    }

    // Pass 2: distribute surplus to Min and Fill constraints.
    if remaining > 0 {
        let flex: Vec<usize> = constraints
            .iter()
            .enumerate()
            .filter(|(_, c)| matches!(c, Constraint::Min(_) | Constraint::Fill))
            .map(|(i, _)| i)
            .collect();
        if !flex.is_empty() {
            let per = remaining / flex.len() as u16;
            let mut leftover = remaining;
            for (k, &i) in flex.iter().enumerate() {
                let share = if k + 1 == flex.len() {
                    leftover
                } else {
                    per.min(leftover)
                };
                sizes[i] += share;
                leftover -= share;
            }
        }
    }

    sizes
}

/// Split `area` into horizontal rows top-to-bottom.
pub fn vertical(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    let heights = resolve(constraints, area.height);
    let mut y = area.y;
    heights
        .into_iter()
        .map(|h| {
            let r = Rect::new(area.x, y, area.width, h);
            y = y.saturating_add(h);
            r
        })
        .collect()
}

/// Split `area` into vertical columns left-to-right.
pub fn horizontal(area: Rect, constraints: &[Constraint]) -> Vec<Rect> {
    let widths = resolve(constraints, area.width);
    let mut x = area.x;
    widths
        .into_iter()
        .map(|w| {
            let r = Rect::new(x, area.y, w, area.height);
            x = x.saturating_add(w);
            r
        })
        .collect()
}

/// Convenience: vertical split returning a fixed-size array.
/// Panics if `constraints.len()` != `N`.
pub fn vertical_arr<const N: usize>(
    area: Rect,
    constraints: [Constraint; N],
) -> [Rect; N] {
    let v = vertical(area, &constraints);
    v.try_into().unwrap_or_else(|_| [Rect::default(); N])
}

/// Convenience: horizontal split returning a fixed-size array.
/// Panics if `constraints.len()` != `N`.
pub fn horizontal_arr<const N: usize>(
    area: Rect,
    constraints: [Constraint; N],
) -> [Rect; N] {
    let v = horizontal(area, &constraints);
    v.try_into().unwrap_or_else(|_| [Rect::default(); N])
}

/// Margin applied symmetrically to a [`Rect`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub horizontal: u16,
    pub vertical: u16,
}

impl Margin {
    pub const fn new(h: u16, v: u16) -> Self {
        Margin {
            horizontal: h,
            vertical: v,
        }
    }
}

/// Shrink `area` by `margin` on each side.
pub fn inner(area: Rect, margin: Margin) -> Rect {
    let dx = margin.horizontal.min(area.width / 2);
    let dy = margin.vertical.min(area.height / 2);
    Rect::new(
        area.x + dx,
        area.y + dy,
        area.width - dx * 2,
        area.height - dy * 2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_fixed_and_fill() {
        let area = Rect::new(0, 0, 80, 24);
        let r = vertical(area, &[Constraint::Length(3), Constraint::Fill, Constraint::Length(1)]);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].height, 3);
        assert_eq!(r[1].height, 20);
        assert_eq!(r[2].height, 1);
        assert_eq!(r[0].y, 0);
        assert_eq!(r[1].y, 3);
        assert_eq!(r[2].y, 23);
    }

    #[test]
    fn horizontal_percentage() {
        let area = Rect::new(0, 0, 100, 10);
        let r = horizontal(area, &[Constraint::Percentage(25), Constraint::Fill]);
        assert_eq!(r[0].width, 25);
        assert_eq!(r[1].width, 75);
    }

    #[test]
    fn min_grows_to_fill() {
        let area = Rect::new(0, 0, 40, 10);
        let r = vertical(area, &[Constraint::Length(2), Constraint::Min(1)]);
        assert_eq!(r[0].height, 2);
        assert_eq!(r[1].height, 8);
    }

    #[test]
    fn array_form() {
        let area = Rect::new(0, 0, 80, 24);
        let [a, b] = vertical_arr(area, [Constraint::Length(10), Constraint::Fill]);
        assert_eq!(a.height, 10);
        assert_eq!(b.height, 14);
    }

    #[test]
    fn inner_margin() {
        let area = Rect::new(5, 5, 20, 10);
        let inner = inner(area, Margin::new(2, 1));
        assert_eq!(inner, Rect::new(7, 6, 16, 8));
    }

    #[test]
    fn ratio() {
        let area = Rect::new(0, 0, 90, 10);
        let r = horizontal(area, &[Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]);
        assert_eq!(r[0].width, 30);
        assert_eq!(r[1].width, 60);
    }
}
