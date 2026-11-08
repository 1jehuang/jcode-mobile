//! Input policy for a transcript pinned to its newest content.
//!
//! Tail positioning itself is handled by GPUI's layout-time `scroll_to_bottom`,
//! so content growth and shrinking activity rows cannot start a competing scroll
//! animation against stale, previous-frame geometry.

/// Update follow mode from deliberate vertical input.
/// Positive deltas move toward older content. Even a small upward drag must
/// release the tail immediately, otherwise following fights the reader.
/// Horizontal gestures and touch-end events must not change follow mode.
pub fn follow_after_scroll(follow: bool, delta_y: f32, distance_to_bottom: f32) -> bool {
    if delta_y > 0.0 {
        false
    } else if delta_y < 0.0 {
        follow || distance_to_bottom <= 56.0
    } else {
        follow
    }
}

#[cfg(test)]
mod tests {
    use super::follow_after_scroll;

    #[test]
    fn upward_drag_releases_follow_even_at_the_tail() {
        assert!(!follow_after_scroll(true, 1.0, 0.0));
        assert!(!follow_after_scroll(true, 8.0, 30.0));
    }

    #[test]
    fn downward_input_keeps_follow_during_streaming_lag() {
        assert!(follow_after_scroll(true, -4.0, 200.0));
        assert!(!follow_after_scroll(false, -4.0, 200.0));
        assert!(follow_after_scroll(false, -4.0, 55.0));
    }

    #[test]
    fn horizontal_and_gesture_end_do_not_repin_or_unpin() {
        assert!(follow_after_scroll(true, 0.0, 200.0));
        assert!(!follow_after_scroll(false, 0.0, 0.0));
    }
}
