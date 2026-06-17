//! Priority list of rects threaded through the reveal pass.

use crate::prelude::*;

/// Priority list of rects collected during a [`Widget::reveal`] pass.
#[derive(Clone)]
pub struct Revelation {
    rects: Vec<Rect<i32, u16>>,
}

impl Default for Revelation {
    fn default() -> Self {
        Self::new()
    }
}

impl Revelation {
    /// Creates an empty revelation.
    pub const fn new() -> Self {
        Self { rects: Vec::new() }
    }

    /// Appends a rect to the list as the lowest priority entry.
    pub fn push(&mut self, r: Rect<i32, u16>) {
        self.rects.push(r);
    }

    /// Translates every entry by `by`.
    pub fn translate(&mut self, by: Vec2<i32>) {
        for r in &mut self.rects {
            r.pos += by;
        }
    }

    /// Clips every entry's `axis` extent to the half-open range `[lo, hi)`.
    pub fn clip_axis(&mut self, axis: Axis2D, lo: i32, hi: i32) {
        self.rects.retain_mut(|r| {
            let s = r.pos[axis];
            let e = s + r.size[axis] as i32;
            let new_s = s.max(lo);
            let new_e = e.min(hi);
            if new_s >= new_e {
                return false;
            }
            r.pos[axis] = new_s;
            r.size[axis] = (new_e - new_s) as u16;
            true
        });
    }

    /// Returns the rects in priority order (earliest = highest priority).
    pub fn get_rects(&self) -> &[Rect<i32, u16>] {
        &self.rects
    }

    /// Removes every entry.
    pub fn clear(&mut self) {
        self.rects.clear();
    }
}

/// Returns the scroll-offset delta that brings the first fitting `(start, size)` entry into view.
pub fn resolve_revelation_axis(
    passes: impl IntoIterator<Item = (i32, i32)>,
    viewport: i32,
    align: Option<Align>,
    scrolloff: u16,
) -> i32 {
    let (start, size) = match passes.into_iter().find(|&(_, s)| s <= viewport) {
        Some(p) => p,
        None => return 0,
    };
    let end = start + size;
    match align {
        Some(Align::Start) => start - scrolloff as i32,
        Some(Align::Center) => start + size / 2 - viewport / 2,
        Some(Align::End) => end - viewport + scrolloff as i32,
        None => {
            let max_so = (viewport - size).max(0) / 2;
            let so = (scrolloff as i32).min(max_so);
            if end + so > viewport {
                end + so - viewport
            } else if start - so < 0 {
                start - so
            } else {
                0
            }
        }
    }
}
