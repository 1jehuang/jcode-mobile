//! Pure overscroll-to-record recognition. Distances are visible rubber-band pixels.
//! Routing, capture ownership, haptics, and wheel idle fallback belong to the caller.

pub const START: f32 = 32.0;
pub const STOP: f32 = 16.0;
pub const LOCK: f32 = 96.0;
pub const LOCK_REST: f32 = 64.0;
pub const MAX_PULL: f32 = 160.0;
const MAX_RAW: f32 = MAX_PULL * 1024.0;
// Exact critical damping reaches visual rest in about 400 ms.
const SPRING_FREQUENCY: f32 = 28.0;
const MAX_DT: f32 = 0.05;
const MAX_VELOCITY: f32 = 2000.0;
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
    /// Displayed displacement, directly finger-owned until release.
    pub pull: f32,
    raw: f32,
    target: f32,
    velocity: f32,
    held: bool,
}

fn rubber(raw: f32) -> f32 {
    raw / (1.0 + raw / MAX_PULL)
}

fn inverse(pull: f32) -> f32 {
    if !pull.is_finite() {
        return 0.0;
    }
    let pull = pull.clamp(0.0, rubber(MAX_RAW));
    (pull / (1.0 - pull / MAX_PULL)).min(MAX_RAW)
}

impl VoiceGesture {
    pub fn held(&self) -> bool {
        self.held
    }

    /// Deterministic untimed input. An end event applies its delta first.
    pub fn feed(&mut self, dy: f32, ended: bool) -> Vec<Action> {
        self.feed_timed(dy, ended, 0.0)
    }

    /// Track a bounded, filtered visible velocity when a sensible event interval
    /// is supplied. Invalid deltas never acquire finger ownership.
    pub fn feed_timed(&mut self, dy: f32, ended: bool, dt: f32) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.state != State::Finished && dy.is_finite() {
            if !self.held {
                // Reconstruct displacement rather than reviving a released raw
                // value. This also makes grabbing a settling affordance seamless.
                self.raw = inverse(self.pull);
                self.held = true;
                self.velocity = 0.0;
            }
            let previous = self.pull;
            self.raw = (self.raw - dy).clamp(0.0, MAX_RAW);
            self.pull = rubber(self.raw);
            self.target = self.pull;
            if dt.is_finite() && (0.001..=0.25).contains(&dt) {
                let measured = ((self.pull - previous) / dt).clamp(-MAX_VELOCITY, MAX_VELOCITY);
                let alpha = 1.0 - (-dt / 0.04).exp();
                self.velocity += alpha * (measured - self.velocity);
            } else {
                self.velocity = 0.0;
            }
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

    /// Sticky lock survives release. Repeated release emits no duplicate actions
    /// and does not overwrite a caller's subsequent rest target.
    pub fn release(&mut self) -> Vec<Action> {
        if !self.held && self.state != State::Finished {
            return Vec::new();
        }
        let finish = self.state == State::Recording;
        self.held = false;
        self.raw = 0.0;
        if self.state == State::Locked {
            self.target = LOCK_REST;
        } else {
            self.state = State::Idle;
            self.target = 0.0;
        }
        if finish {
            vec![Action::Finish]
        } else {
            Vec::new()
        }
    }

    /// Cancel capture, never transcribe/send. Unlike dismiss this reports the
    /// capture cancellation, but both preserve visual position and momentum.
    pub fn cancel(&mut self) -> Option<Action> {
        let recording = matches!(self.state, State::Recording | State::Locked);
        self.dismiss();
        recording.then_some(Action::Cancel)
    }

    /// Consume the remaining finger tail after capture startup failure.
    pub fn abort(&mut self) {
        self.state = State::Finished;
        self.raw = 0.0;
        self.target = 0.0;
    }

    /// Finish only the visual gesture, without emitting capture actions.
    pub fn dismiss(&mut self) {
        self.state = State::Idle;
        self.raw = 0.0;
        self.held = false;
        self.target = 0.0;
    }

    /// Retarget only released or already-finished visuals. Never move the
    /// current pull or change capture state/ownership. Ignore nonfinite heights.
    pub fn rest_at(&mut self, height: f32) {
        if (!self.held || self.state == State::Finished) && height.is_finite() {
            self.target = height.clamp(0.0, MAX_PULL);
        }
    }

    /// Honor Reduce Motion without changing capture or gesture ownership.
    pub fn snap_to_target(&mut self) {
        self.pull = self.target;
        self.velocity = 0.0;
    }

    /// Hard reset, reserved for explicitly discarding the entire gesture.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn active(&self) -> bool {
        self.state != State::Idle
    }

    /// Exact critically damped settling, with no autonomous held-finger motion.
    pub fn tick(&mut self, dt: f32) -> bool {
        if self.held && self.state != State::Finished {
            return false;
        }
        // pull is public: contain even malformed caller-provided visual values.
        if !self.pull.is_finite() {
            self.pull = self.target;
            self.velocity = 0.0;
        }
        self.pull = self.pull.clamp(0.0, MAX_PULL);
        let moving = (self.pull - self.target).abs() > SETTLE_EPSILON
            || self.velocity.abs() > VELOCITY_EPSILON;
        if !dt.is_finite() || dt <= 0.0 {
            return moving;
        }
        let dt = dt.min(MAX_DT);
        let displacement = self.pull - self.target;
        // A critical oscillator can still cross its target with excessive input
        // momentum. Project momentum into the non-crossing, monotone cone only
        // when integrating, never at release/abort/retarget boundaries.
        let limit = SPRING_FREQUENCY * displacement.abs();
        let velocity = if displacement > 0.0 {
            self.velocity.clamp(-limit, 0.0)
        } else if displacement < 0.0 {
            self.velocity.clamp(0.0, limit)
        } else {
            0.0
        };
        let c = velocity + SPRING_FREQUENCY * displacement;
        let envelope = (-SPRING_FREQUENCY * dt).exp();
        self.pull = (self.target + (displacement + c * dt) * envelope).clamp(0.0, MAX_PULL);
        self.velocity = (velocity - SPRING_FREQUENCY * c * dt) * envelope;
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

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 0.002, "{a} != {b}");
    }

    #[test]
    fn unchanged_thresholds_without_position_jumps() {
        assert_eq!((START, STOP, LOCK), (32.0, 16.0, 96.0));
        let mut g = VoiceGesture::default();
        assert!(g.feed(-39.99, false).is_empty());
        assert_eq!(g.feed(-0.01, false), vec![Action::Start]);
        assert_close(g.pull, START);
        assert_eq!(g.feed(-200.0, false), vec![Action::Lock]);
        assert_close(g.pull, LOCK);
        assert!(g.held());
        assert!(!g.tick(1.0));
        assert_close(g.pull, LOCK);
    }

    #[test]
    fn lock_stays_sticky_but_tracks_hand_until_release() {
        let mut g = VoiceGesture::default();
        assert_eq!(g.feed(-240.0, false), vec![Action::Start, Action::Lock]);
        for delta in [-100.0, 200.0, 500.0, -20.0] {
            let expected = rubber((g.raw - delta).clamp(0.0, MAX_RAW));
            assert!(g.feed(delta, false).is_empty());
            assert_eq!(g.pull, expected);
            assert!(!g.tick(0.016));
            assert_eq!(g.pull, expected);
            assert_eq!(g.state, State::Locked);
        }
        let before = g.pull;
        assert!(g.release().is_empty());
        assert_eq!(g.pull, before);
        assert!(!g.held());
        assert_eq!(g.target, LOCK_REST);
        g.rest_at(50.0);
        assert!(g.release().is_empty());
        assert_eq!(g.target, 50.0);
    }

    #[test]
    fn exact_nonoscillating_settling_at_30_60_120_hz() {
        fn sample(hz: u32, frames: u32) -> VoiceGesture {
            let mut g = VoiceGesture::default();
            g.feed(-240.0, true);
            for _ in 0..frames {
                let old = g.pull;
                g.tick(1.0 / hz as f32);
                assert!((LOCK_REST..=old).contains(&g.pull));
            }
            g
        }
        for frames in [3, 6, 9, 12, 15] {
            let reference = sample(30, frames);
            for multiplier in [2, 4] {
                let actual = sample(30 * multiplier, frames * multiplier);
                assert_close(reference.pull, actual.pull);
                assert!((reference.velocity - actual.velocity).abs() < 0.01);
            }
        }
        assert!(sample(60, 18).pull > LOCK_REST);
        assert_eq!(sample(60, 30).pull, LOCK_REST);
    }

    #[test]
    fn release_and_regrab_preserve_position_not_stale_raw() {
        let mut g = VoiceGesture::default();
        assert_eq!(g.feed(-40.0, true), vec![Action::Start, Action::Finish]);
        assert_eq!(g.pull, START);
        assert_eq!(g.raw, 0.0);
        assert!(g.release().is_empty());
        g.tick(0.05);
        let before = g.pull;
        assert!(g.feed(0.0, false).is_empty());
        assert_close(g.pull, before);
        assert!(g.held());
        let expected = rubber(inverse(before) + 1.0);
        g.feed(-1.0, false);
        assert_close(g.pull, expected);
    }

    #[test]
    fn retargeting_preserves_position_velocity_state_and_ownership() {
        let mut g = VoiceGesture::default();
        g.feed_timed(-240.0, false, 0.016);
        let before = (g.pull, g.velocity);
        g.rest_at(10.0);
        assert_eq!(g.target, g.pull);
        assert_eq!((g.pull, g.velocity), before);
        g.release();
        assert_eq!((g.pull, g.velocity), before);
        g.tick(0.016);
        let before = (g.pull, g.velocity);
        g.rest_at(20.0);
        assert_eq!((g.pull, g.velocity), before);
        assert_eq!(g.state, State::Locked);
        assert!(!g.held());
        g.abort();
        assert_eq!((g.pull, g.velocity), before);
        g.rest_at(10.0);
        assert_eq!(g.target, 10.0);
    }

    #[test]
    fn reverse_stop_and_abort_consume_tail_exactly_once() {
        for abort in [false, true] {
            let mut g = VoiceGesture::default();
            g.feed(-40.0, false);
            if abort {
                g.abort();
            } else {
                // Isolate the inclusive STOP boundary from subtraction rounding.
                g.raw = inverse(STOP);
                assert_eq!(g.feed(0.0, false), vec![Action::Finish]);
            }
            assert_eq!(g.state, State::Finished);
            assert!(g.held());
            let before = g.pull;
            g.rest_at(0.0);
            assert_eq!(g.pull, before);
            assert!(g.feed(-1000.0, false).is_empty());
            for _ in 0..60 {
                g.tick(1.0 / 60.0);
            }
            assert_eq!(g.pull, 0.0);
            assert!(g.active());
            assert!(g.feed(-1000.0, true).is_empty());
            assert!(!g.active());
            assert_eq!(g.feed(-40.0, true), vec![Action::Start, Action::Finish]);
        }
    }

    #[test]
    fn dismiss_is_soft_and_silent_cancel_reports_capture_only() {
        for distance in [10.0, 40.0, 240.0] {
            let mut g = VoiceGesture::default();
            g.feed_timed(-distance, false, 0.016);
            let before = (g.pull, g.velocity);
            let expected = (distance >= 40.0).then_some(Action::Cancel);
            assert_eq!(g.cancel(), expected);
            assert_eq!((g.pull, g.velocity), before);
            assert!(!g.active());
            assert!(!g.held());
            assert_eq!(g.raw, 0.0);
            assert_eq!(g.cancel(), None);
            g.dismiss();
            assert_eq!((g.pull, g.velocity), before);
            for _ in 0..60 {
                g.tick(1.0 / 60.0);
            }
            assert_eq!(g.pull, 0.0);
        }
    }

    #[test]
    fn finite_guards_bounded_velocity_and_suspended_frame_cap() {
        let mut g = VoiceGesture::default();
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(g.feed_timed(bad, false, 0.016).is_empty());
            assert!(!g.held());
            g.rest_at(bad);
            assert_eq!(g.target, 0.0);
        }
        g.feed_timed(-f32::MAX, false, 0.001);
        assert!(g.pull.is_finite() && g.pull <= MAX_PULL);
        assert!(g.velocity.is_finite() && g.velocity.abs() <= MAX_VELOCITY);
        g.release();
        let before = g.pull;
        for bad in [f32::NAN, f32::INFINITY, -1.0, 0.0] {
            g.tick(bad);
            assert_eq!(g.pull, before);
        }
        g.tick(f32::MAX);
        assert!(g.pull > LOCK_REST && g.pull < before);
        for _ in 0..60 {
            g.tick(1.0 / 60.0);
        }
        assert_eq!(g.pull, LOCK_REST);
        g.pull = f32::NAN;
        g.tick(0.016);
        assert!(g.pull.is_finite());
    }

    #[test]
    fn reduce_motion_and_reset_are_explicit() {
        let mut g = VoiceGesture::default();
        g.feed(-240.0, false);
        g.snap_to_target();
        assert_eq!(g.pull, LOCK);
        assert!(g.held());
        g.release();
        g.snap_to_target();
        assert_eq!(g.pull, LOCK_REST);
        assert_eq!(g.state, State::Locked);
        g.abort();
        g.snap_to_target();
        assert_eq!(g.pull, 0.0);
        assert_eq!(g.state, State::Finished);
        g.reset();
        assert!(!g.active());
        assert!(!g.held());
        assert!(!g.tick(1.0));
    }
}
