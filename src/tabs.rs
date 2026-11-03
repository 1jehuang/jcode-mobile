//! Live session tabs, ported from jcode-desktop `live_tabs.rs` (`TabLayout::new`,
//! `geometry`, `paint_order`, `exposed_in`). One row, so no workspace groups.
//! The focused tab is up to 208px or 55% of the track; the rest are 12px
//! narrower and overlap like folder tabs once the track is crowded.

pub const TAB_GAP: f32 = 6.0;
pub const TAB_HEIGHT: f32 = 34.0;
const ACTIVE_MAX: f32 = 208.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabGeometry {
    pub left: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug)]
struct TabLayout {
    start: f32,
    active_width: f32,
    inactive_width: f32,
    step: f32,
}

impl TabLayout {
    fn new(available: f32, count: usize) -> Self {
        let available = available.max(0.0);
        let count = count.max(1);
        let active_width = if count == 1 {
            available.min(ACTIVE_MAX)
        } else {
            (available * 0.55).min(ACTIVE_MAX)
        }
        .floor();
        let inactive_width = (active_width - 12.0).max(active_width * 0.9).floor();
        let step = if count == 1 {
            0.0
        } else {
            ((available - active_width) / (count - 1) as f32).min(inactive_width + TAB_GAP)
        };
        let used = active_width + step * (count - 1) as f32;
        Self { start: (available - used) / 2.0, active_width, inactive_width, step }
    }

    fn geometry(self, position: usize, selected: usize) -> TabGeometry {
        TabGeometry {
            left: self.start
                + self.step * position.min(selected) as f32
                + if position > selected {
                    self.active_width - self.inactive_width + self.step * (position - selected) as f32
                } else {
                    0.0
                },
            width: if position == selected { self.active_width } else { self.inactive_width },
            height: if position == selected { TAB_HEIGHT } else { TAB_HEIGHT - 4.0 },
        }
    }
}

pub fn layout(available: f32, count: usize, selected: usize) -> Vec<TabGeometry> {
    let l = TabLayout::new(available, count);
    (0..count).map(|i| l.geometry(i, selected)).collect()
}

/// Left tabs first, right tabs from the far end inward, focused tab on top.
pub fn paint_order(count: usize, selected: usize) -> Vec<usize> {
    (0..selected).chain((selected + 1..count).rev()).chain(std::iter::once(selected)).collect()
}

/// The part of a tab not covered by tabs painted above it: (left, width).
pub fn exposed(tabs: &[TabGeometry], position: usize, order: &[usize]) -> (f32, f32) {
    let tab = tabs[position];
    let (mut left, mut right) = (tab.left, tab.left + tab.width);
    for other in order.iter().copied().skip_while(|&i| i != position).skip(1).map(|i| tabs[i]) {
        if other.left >= right || other.left + other.width <= left {
            continue;
        }
        if other.left > left {
            right = right.min(other.left);
        } else {
            left = left.max(other.left + other.width).min(right);
        }
    }
    (left, (right - left).max(0.0))
}

/// Critically damped-ish easing toward targets; returns true while moving.
pub fn step_toward(current: &mut TabGeometry, target: TabGeometry, dt: f32) -> bool {
    let k = 1.0 - (-dt * 18.0).exp();
    let mut moving = false;
    for (c, t) in [
        (&mut current.left, target.left),
        (&mut current.width, target.width),
        (&mut current.height, target.height),
    ] {
        let d = t - *c;
        if d.abs() < 0.3 {
            *c = t;
        } else {
            *c += d * k;
            moving = true;
        }
    }
    moving
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_tab_is_capped_and_centered() {
        let g = layout(300.0, 1, 0);
        assert_eq!(g[0].width, 208.0);
        assert_eq!(g[0].left, 46.0);
    }

    #[test]
    fn crowded_tabs_shrink_overlap_and_stay_in_bounds() {
        for n in 2..40 {
            for sel in 0..n {
                let g = layout(290.0, n, sel);
                for t in &g {
                    assert!(t.left >= -0.01 && t.left + t.width <= 290.01, "{n} {sel} {t:?}");
                }
                let order = paint_order(n, sel);
                assert!((exposed(&g, sel, &order).1 - g[sel].width).abs() < 0.01);
            }
        }
        // Few tabs keep a gap, many overlap.
        let g = layout(500.0, 2, 0);
        assert!(g[1].left >= g[0].left + g[0].width);
        let g = layout(290.0, 8, 0);
        assert!(g[1].left < g[0].left + g[0].width);
    }
}
