//! Pure overscroll-to-record recognition. Distances are visible rubber-band pixels.
//! Routing, capture ownership, haptics, and wheel idle fallback belong to the caller.

pub const START: f32 = 32.0;
pub const STOP: f32 = 16.0;
pub const LOCK: f32 = 96.0;
pub const LOCK_REST: f32 = LOCK;
const LOCK_IMPULSE: f32 = 120.0;
const MAX_PULL: f32 = 160.0;
const MAX_RAW: f32 = MAX_PULL * 1024.0;
// Underdamped spring: enough rebound to feel physical without a long oscillation.
const SPRING_FREQUENCY: f32 = 22.0;
const SPRING_DAMPING: f32 = 0.55;
const MAX_DT: f32 = 0.25;
const SETTLE_EPSILON: f32 = 0.05;
const VELOCITY_EPSILON: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Finish,
    Lock,
    Cancel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Pulling,
    Recording,
    Locked,
    /// A reverse-to-stop already finished capture. Ignore travel until release.
    Finished,
}

#[derive(Debug, Default)]
pub struct VoiceGesture {
    pub state: State,
    /// Displayed displacement, including the animated lock/rest and release snap.
    pub pull: f32,
    raw: f32,
    target: f32,
    velocity: f32,
}

fn rubber(raw: f32) -> f32 {
    raw / (1.0 + raw / MAX_PULL)
}

impl VoiceGesture {
    /// Negative deltas pull beyond the latest content. An end event still applies
    /// its delta first, allowing a quick complete gesture to start and finish.
    pub fn feed(&mut self, dy: f32, ended: bool) -> Vec<Action> {
        let mut actions = Vec::new();
        if !matches!(self.state, State::Locked | State::Finished) && dy.is_finite() {
            self.raw = (self.raw - dy).clamp(0.0, MAX_RAW);
            self.pull = rubber(self.raw);
            self.target = self.pull;
            self.velocity = 0.0;
            if self.state == State::Idle && self.raw > 0.0 {
                self.state = State::Pulling;
            }
            if self.state == State::Pulling && self.pull >= START {
                self.state = State::Recording;
                actions.push(Action::Start);
            }
            if self.state == State::Recording {
                if self.pull >= LOCK {
                    self.state = State::Locked;
                    self.target = LOCK_REST;
                    self.velocity = LOCK_IMPULSE;
                    actions.push(Action::Lock);
                } else if self.pull <= STOP {
                    self.state = State::Finished;
                    self.target = 0.0;
                    actions.push(Action::Finish);
                }
            }
        }
        if ended {
            actions.extend(self.release());
        }
        actions
    }

    /// End finger ownership, but retain a locked recording until explicit reset
    /// or cancellation. Calling this repeatedly never emits duplicate actions.
    pub fn release(&mut self) -> Vec<Action> {
        if self.state == State::Locked {
            self.target = LOCK_REST;
            return Vec::new();
        }
        let finish = self.state == State::Recording;
        self.state = State::Idle;
        self.raw = 0.0;
        self.target = 0.0;
        if finish {
            vec![Action::Finish]
        } else {
            Vec::new()
        }
    }

    /// Cancel capture, never transcribe/send. A completed recording cannot be
    /// canceled a second time while its release is still being consumed.
    pub fn cancel(&mut self) -> Option<Action> {
        let recording = matches!(self.state, State::Recording | State::Locked);
        self.reset();
        recording.then_some(Action::Cancel)
    }

    /// Native capture startup failed. Collapse the affordance without sending
    /// anything, but consume the remaining finger travel until its end event.
    pub fn abort(&mut self) {
        self.state = State::Finished;
        self.raw = 0.0;
        self.target = 0.0;
    }

    /// Honor Reduce Motion without changing capture or gesture ownership.
    pub fn snap_to_target(&mut self) {
        self.pull = self.target;
        self.velocity = 0.0;
    }

    /// Called after the owner's stop callback or when changing routes.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Includes a held, already-finished gesture to prevent re-routing its tail.
    /// Locked capture remains active even after finger release.
    pub fn active(&self) -> bool {
        self.state != State::Idle
    }

    /// Animate only lock/rest and released displacement. A held pull never times
    /// out. Returns whether another frame is needed, not whether capture is live.
    pub fn tick(&mut self, dt: f32) -> bool {
        if !matches!(self.state, State::Idle | State::Locked | State::Finished) {
            return false;
        }
        let moving = (self.pull - self.target).abs() > SETTLE_EPSILON
            || self.velocity.abs() > VELOCITY_EPSILON;
        if !dt.is_finite() || dt <= 0.0 {
            return moving;
        }
        // Exact solution for a damped harmonic oscillator. Unlike an Euler
        // step this preserves the same bounce at 30, 60, and 120 Hz. Clamp
        // suspended-frame time so resuming cannot instantly skip the snap.
        let dt = dt.min(MAX_DT);
        let decay = SPRING_FREQUENCY * SPRING_DAMPING;
        let frequency = SPRING_FREQUENCY * (1.0 - SPRING_DAMPING.powi(2)).sqrt();
        let (sin, cos) = (frequency * dt).sin_cos();
        let envelope = (-decay * dt).exp();
        let displacement = self.pull - self.target;
        let velocity = self.velocity;
        self.pull = self.target
            + envelope * (displacement * cos + (velocity + decay * displacement) / frequency * sin);
        self.velocity = envelope
            * (velocity * cos
                - (decay * velocity + SPRING_FREQUENCY.powi(2) * displacement) / frequency * sin);
        // Content cannot move beyond either end of the visible rubber band.
        if !(0.0..=MAX_PULL).contains(&self.pull) {
            self.pull = self.pull.clamp(0.0, MAX_PULL);
            self.velocity = 0.0;
        }
        if (self.pull - self.target).abs() <= SETTLE_EPSILON
            && self.velocity.abs() <= VELOCITY_EPSILON
        {
            self.pull = self.target;
            self.velocity = 0.0;
            false
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(visible: f32) -> f32 {
        visible / (1.0 - visible / MAX_PULL)
    }

    #[test]
    fn threshold_boundaries() {
        let mut g = VoiceGesture::default();
        assert!(g.feed(-39.99, false).is_empty());
        assert_eq!(g.state, State::Pulling);
        g.reset();
        assert_eq!(g.feed(-40.0, false), vec![Action::Start]);
        assert_eq!(g.pull, START);
        assert!(g.feed(-(239.99 - 40.0), false).is_empty());
        g.reset();
        assert_eq!(g.feed(-240.0, false), vec![Action::Start, Action::Lock]);
        assert_eq!(g.pull, LOCK);
        assert_eq!(g.state, State::Locked);

        g.reset();
        g.feed(-40.0, false);
        // Isolate the inclusive boundary from subtraction rounding.
        g.raw = raw(STOP);
        assert_eq!(g.feed(0.0, false), vec![Action::Finish]);
        assert_eq!(g.pull, STOP);
    }

    #[test]
    fn reverse_before_start_never_captures() {
        let mut g = VoiceGesture::default();
        assert!(g.feed(20.0, false).is_empty());
        assert!(!g.active());
        assert!(g.feed(-20.0, false).is_empty());
        assert!(g.active());
        assert!(g.feed(40.0, false).is_empty());
        assert_eq!(g.pull, 0.0);
        assert!(g.release().is_empty());
        assert!(!g.active());
    }

    #[test]
    fn reverse_stop_finishes_once_and_consumes_tail() {
        let mut g = VoiceGesture::default();
        assert_eq!(g.feed(-40.0, false), vec![Action::Start]);
        assert!(g.feed(40.0 - raw(STOP + 0.01), false).is_empty());
        assert_eq!(g.feed(0.02, false), vec![Action::Finish]);
        assert_eq!(g.state, State::Finished);
        assert!(g.active());
        assert!(g.feed(-1000.0, false).is_empty());
        assert!(g.feed(0.0, true).is_empty());
        assert!(!g.active());
        assert_eq!(g.feed(-40.0, false), vec![Action::Start]);
    }

    #[test]
    fn quick_end_and_repeated_release() {
        let mut g = VoiceGesture::default();
        assert_eq!(g.feed(-40.0, true), vec![Action::Start, Action::Finish]);
        assert_eq!(g.state, State::Idle);
        assert!(g.release().is_empty());
        assert!(g.tick(0.016));
        assert_eq!(g.feed(-10.0, true), Vec::<Action>::new());
    }

    #[test]
    fn lock_is_sticky_and_snap_is_animated() {
        let mut g = VoiceGesture::default();
        assert_eq!(g.feed(-240.0, true), vec![Action::Start, Action::Lock]);
        assert!(g.active());
        assert_eq!(g.pull, LOCK);
        assert!(g.tick(0.016));
        assert!(g.pull > LOCK_REST && g.pull <= MAX_PULL);
        assert!(g.feed(1000.0, true).is_empty());
        assert_eq!(g.state, State::Locked);
        for _ in 0..100 {
            g.tick(0.016);
        }
        assert_eq!(g.pull, LOCK_REST);
        assert!(g.release().is_empty());
    }

    #[test]
    fn cancel_never_finishes_and_reset_is_silent() {
        for distance in [40.0, 240.0] {
            let mut g = VoiceGesture::default();
            g.feed(-distance, false);
            assert_eq!(g.cancel(), Some(Action::Cancel));
            assert_eq!(g.state, State::Idle);
            assert_eq!(g.pull, 0.0);
            assert_eq!(g.cancel(), None);
            assert!(g.release().is_empty());
        }
        let mut g = VoiceGesture::default();
        g.feed(-10.0, false);
        assert_eq!(g.cancel(), None);
        g.feed(-40.0, false);
        g.feed(40.0, false);
        assert_eq!(g.cancel(), None);
        g.feed(-240.0, false);
        g.reset();
        assert!(!g.active());
        assert!(!g.tick(1.0));
    }

    #[test]
    fn lock_snap_overshoots_then_settles() {
        let mut g = VoiceGesture::default();
        g.feed(-240.0, false);
        let mut bounced = false;
        for _ in 0..180 {
            g.tick(1.0 / 120.0);
            bounced |= g.pull > LOCK_REST + 0.5;
            assert!((0.0..=MAX_PULL).contains(&g.pull));
        }
        assert!(bounced, "lock snap must visibly rebound beyond its rest");
        assert_eq!(g.pull, LOCK_REST);
        assert!(!g.tick(1.0 / 120.0));
        assert_eq!(g.state, State::Locked);
    }

    #[test]
    fn spring_is_consistent_across_frame_rates() {
        fn at_time(hz: u32, frames: u32) -> (f32, f32) {
            let mut g = VoiceGesture::default();
            g.feed(-240.0, false);
            for _ in 0..frames {
                g.tick(1.0 / hz as f32);
            }
            (g.pull, g.velocity)
        }
        for frames in [3, 6, 12] {
            let reference = at_time(30, frames);
            for multiplier in [2, 4] {
                let actual = at_time(30 * multiplier, frames * multiplier);
                assert!((reference.0 - actual.0).abs() < 0.001);
                assert!((reference.1 - actual.1).abs() < 0.01);
            }
        }
    }

    #[test]
    fn startup_abort_consumes_tail_without_sending() {
        for distance in [40.0, 240.0] {
            let mut g = VoiceGesture::default();
            g.feed(-distance, false);
            g.abort();
            assert_eq!(g.state, State::Finished);
            assert!(g.active());
            assert!(g.feed(-1000.0, false).is_empty());
            for _ in 0..120 {
                g.tick(1.0 / 60.0);
            }
            assert_eq!(g.pull, 0.0);
            assert!(g.active());
            assert!(g.feed(-1000.0, true).is_empty());
            assert!(!g.active());
            assert_eq!(g.feed(-40.0, false), vec![Action::Start]);
        }
    }

    #[test]
    fn reduce_motion_snaps_without_changing_capture() {
        let mut g = VoiceGesture::default();
        g.feed(-240.0, false);
        g.tick(0.016);
        assert!(g.pull > LOCK_REST);
        g.snap_to_target();
        assert_eq!(g.pull, LOCK);
        assert_eq!(g.state, State::Locked);
        assert!(!g.tick(0.016));
        g.abort();
        g.snap_to_target();
        assert_eq!(g.pull, 0.0);
        assert_eq!(g.state, State::Finished);
        assert!(g.active());
        assert!(!g.tick(0.016));
    }

    #[test]
    fn bounded_physics_and_no_held_timeout() {
        let mut g = VoiceGesture::default();
        g.feed(-40.0, false);
        assert!(!g.tick(1000.0));
        assert_eq!(g.state, State::Recording);
        assert_eq!(g.pull, START);
        for delta in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(g.feed(delta, false).is_empty());
            assert_eq!(g.pull, START);
        }
        g.feed(-f32::MAX, false);
        assert!(g.pull.is_finite() && g.pull <= MAX_PULL);
        for dt in [f32::NAN, f32::INFINITY, -1.0, 0.0, 0.016, f32::MAX] {
            g.tick(dt);
            assert!(g.pull.is_finite() && (0.0..=MAX_PULL).contains(&g.pull));
        }
        // Huge dt is clamped, rather than skipping directly to rest.
        for _ in 0..10 {
            g.tick(f32::MAX);
        }
        assert_eq!(g.pull, LOCK_REST);
        g.reset();
        g.feed(-20.0, true);
        for _ in 0..10 {
            g.tick(f32::MAX);
        }
        assert!(!g.tick(0.016));
        assert_eq!(g.pull, 0.0);
    }
}
