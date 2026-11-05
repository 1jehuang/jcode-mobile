//! Horizontal swipe between session tabs, modeled on the desktop workspace
//! pull: the panel follows the finger with resistance, and a release past the
//! threshold slides to the neighbor while the new panel eases in.

use std::time::{Duration, Instant};

/// Distance (px) of finger travel that counts as a committed swipe.
pub const COMMIT: f32 = 90.0;
/// How long the slide-in takes after a commit, or the snap-back after a cancel.
pub const SETTLE: Duration = Duration::from_millis(220);
/// No wheel/touch event for this long ends the gesture (trackpads without phases).
pub const IDLE_END: Duration = Duration::from_millis(160);

#[derive(Default)]
pub struct Swipe {
    /// Raw horizontal travel of the current gesture.
    pub dx: f32,
    /// Vertical travel, so mostly-vertical scrolls never become swipes.
    dy: f32,
    pub active: bool,
    pub last_event: Option<Instant>,
    /// Release animation: (start offset, start time).
    settle: Option<(f32, Instant)>,
}

/// Rubber-band: direct at first, then increasingly resisted, like the desktop pull.
pub fn resist(dx: f32, width: f32) -> f32 {
    let limit = width * 0.6;
    let a = dx.abs();
    let r = limit * (1.0 - (-a / limit).exp());
    r.copysign(dx)
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

pub enum Release {
    /// Go to the previous (-1) or next (+1) tab.
    Switch(i32),
    Cancel,
}

impl Swipe {
    /// Feeds a scroll delta. Returns true when the gesture is horizontal and was consumed.
    pub fn feed(&mut self, dx: f32, dy: f32, now: Instant) -> bool {
        self.last_event = Some(now);
        if !self.active {
            self.dx += dx;
            self.dy += dy;
            if self.dx.abs() > 12.0 && self.dx.abs() > self.dy.abs() * 1.5 {
                self.active = true;
                self.settle = None;
            } else if self.dy.abs() > 12.0 {
                // Committed to vertical scrolling. Reset so the next gesture starts clean.
                self.dx = 0.0;
                self.dy = 0.0;
            }
            return self.active;
        }
        self.dx += dx;
        true
    }

    /// Ends the gesture. `can_prev`/`can_next` say whether a neighbor exists.
    pub fn release(&mut self, width: f32, can_prev: bool, can_next: bool, now: Instant) -> Release {
        let dx = self.dx;
        let was = self.active;
        self.active = false;
        self.dx = 0.0;
        self.dy = 0.0;
        self.last_event = None;
        if !was {
            return Release::Cancel;
        }
        // Content moves with the finger: dragging left (dx < 0) reveals the next tab.
        if dx <= -COMMIT && can_next {
            self.settle = Some((width, now));
            Release::Switch(1)
        } else if dx >= COMMIT && can_prev {
            self.settle = Some((-width, now));
            Release::Switch(-1)
        } else {
            self.settle = Some((resist(dx, width), now));
            Release::Cancel
        }
    }

    /// Current horizontal offset of the chat panel, and whether it is still moving.
    pub fn offset(&mut self, width: f32, can_prev: bool, can_next: bool, now: Instant) -> (f32, bool) {
        if self.active {
            // Edges with no neighbor resist much harder.
            let edge = (self.dx > 0.0 && !can_prev) || (self.dx < 0.0 && !can_next);
            let r = resist(self.dx, width);
            return (if edge { r * 0.25 } else { r }, false);
        }
        if let Some((from, start)) = self.settle {
            let t = now.duration_since(start).as_secs_f32() / SETTLE.as_secs_f32();
            if t >= 1.0 {
                self.settle = None;
                return (0.0, false);
            }
            return (from * (1.0 - ease_out_cubic(t)), true);
        }
        (0.0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_scroll_is_not_a_swipe() {
        let mut s = Swipe::default();
        let now = Instant::now();
        assert!(!s.feed(2.0, 30.0, now));
        assert!(!s.feed(3.0, 20.0, now));
        assert!(!s.active);
    }

    #[test]
    fn far_left_drag_switches_to_next() {
        let mut s = Swipe::default();
        let now = Instant::now();
        assert!(s.feed(-20.0, 0.0, now));
        s.feed(-100.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Switch(1)));
        // New panel starts a full width to the right and eases to 0.
        let (o, moving) = s.offset(390.0, true, true, now);
        assert!(moving && (o - 390.0).abs() < 1.0);
        let (o, moving) = s.offset(390.0, true, true, now + SETTLE);
        assert!(!moving && o == 0.0);
    }

    #[test]
    fn short_drag_or_missing_neighbor_snaps_back() {
        let mut s = Swipe::default();
        let now = Instant::now();
        s.feed(40.0, 0.0, now);
        assert!(matches!(s.release(390.0, true, true, now), Release::Cancel));
        s.feed(200.0, 0.0, now);
        assert!(matches!(s.release(390.0, false, true, now), Release::Cancel));
    }

    #[test]
    fn resistance_is_bounded() {
        assert!(resist(10_000.0, 390.0) < 390.0 * 0.6 + 0.1);
        assert!((resist(5.0, 390.0) - 5.0).abs() < 0.2);
    }
}
