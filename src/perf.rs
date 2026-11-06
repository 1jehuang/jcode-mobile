//! Opt-in, local CPU-span instrumentation. See `docs/performance.md`.
use std::{
    collections::{BTreeMap, VecDeque},
    fs::{File, OpenOptions},
    io::Write,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const INTERVAL: Duration = Duration::from_secs(5);
const BUDGET: Duration = Duration::from_micros(16_670);
const SAMPLE_LIMIT: usize = 256;
const ACTION_LIMIT: usize = 64;
static RECORDER: OnceLock<Option<Arc<Recorder>>> = OnceLock::new();

/// Measures elapsed CPU-side wall time until dropped, including early returns.
/// Disabled spans do not read the clock or acquire a lock.
#[must_use = "keep the span alive for the operation being measured"]
pub struct Span {
    action: &'static str,
    started: Option<Instant>,
}

/// Start a span with a fixed, non-sensitive developer-defined action label.
pub fn span(action: &'static str) -> Span {
    Span {
        action,
        started: start(),
    }
}

/// Return a start timestamp only when cached instrumentation is enabled.
pub fn start() -> Option<Instant> {
    recorder().map(|_| Instant::now())
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            record(self.action, started.elapsed());
        }
    }
}

/// Record an externally measured CPU-side duration, not a GPU frame duration.
pub fn record(action: &'static str, duration: Duration) {
    let Some(recorder) = recorder() else { return };
    let Ok(mut state) = recorder.state.try_lock() else {
        return;
    };
    // Retain a fixed set of labels for the process lifetime. Unknown labels above
    // the cap are dropped rather than allowing unbounded cardinality.
    if state.actions.len() < ACTION_LIMIT || state.actions.contains_key(action) {
        state.actions.entry(action).or_default().add(duration);
    }
}

fn recorder() -> Option<&'static Arc<Recorder>> {
    RECORDER
        .get_or_init(|| {
            if std::env::var_os("JCODE_MOBILE_PERF").as_deref() != Some(std::ffi::OsStr::new("1")) {
                return None;
            }
            let file = std::env::var_os("JCODE_MOBILE_PERF_LOG").and_then(|path| {
                match OpenOptions::new().create(true).append(true).open(path) {
                    Ok(file) => Some(file),
                    Err(error) => {
                        // Do not echo the potentially private path.
                        let _ = writeln!(
                            std::io::stderr(),
                            "jcode_mobile_perf: cannot open optional log ({:?})",
                            error.kind()
                        );
                        None
                    }
                }
            });
            let recorder = Arc::new(Recorder {
                state: Mutex::new(State {
                    actions: BTreeMap::new(),
                    last_flush: Instant::now(),
                    file,
                }),
            });
            let worker = Arc::clone(&recorder);
            if std::thread::Builder::new()
                .name("mobile-perf".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(INTERVAL);
                        if let Ok(mut state) = worker.state.lock() {
                            worker.flush(&mut state);
                        }
                    }
                })
                .is_err()
            {
                let _ = writeln!(
                    std::io::stderr(),
                    "jcode_mobile_perf: periodic worker unavailable, instrumentation disabled"
                );
                return None;
            }
            Some(recorder)
        })
        .as_ref()
}

struct Recorder {
    state: Mutex<State>,
}
struct State {
    actions: BTreeMap<&'static str, Aggregate>,
    last_flush: Instant,
    file: Option<File>,
}

impl Recorder {
    fn flush(&self, state: &mut State) {
        let window = state.last_flush.elapsed().as_secs_f64();
        let mut output = String::new();
        use std::fmt::Write as _;
        for (action, aggregate) in &mut state.actions {
            let Some(summary) = aggregate.summary() else {
                continue;
            };
            // Restrict labels to short printable identifiers, preventing log injection.
            let label: String = action
                .chars()
                .take(80)
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let _ = writeln!(
                output,
                "jcode_mobile_perf action={label} window_s={window:.3} count={} mean_ms={:.3} p95_ms={:.3} max_ms={:.3} over_budget={} budget_ms=16.670 samples={}",
                summary.count,
                summary.mean_ms,
                summary.p95_ms,
                summary.max_ms,
                summary.over_budget,
                summary.samples
            );
            *aggregate = Aggregate::default();
        }
        state.last_flush = Instant::now();
        if output.is_empty() {
            return;
        }
        let _ = std::io::stderr().lock().write_all(output.as_bytes());
        if let Some(file) = &mut state.file {
            if file
                .write_all(output.as_bytes())
                .and_then(|_| file.flush())
                .is_err()
            {
                let _ = writeln!(
                    std::io::stderr(),
                    "jcode_mobile_perf: optional log write failed, continuing on stderr"
                );
                state.file = None;
            }
        }
    }
}

#[derive(Default)]
struct Aggregate {
    count: u64,
    total_ms: f64,
    max: Duration,
    over_budget: u64,
    samples: VecDeque<Duration>,
}
struct Summary {
    count: u64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    over_budget: u64,
    samples: usize,
}
impl Aggregate {
    fn add(&mut self, duration: Duration) {
        self.count += 1;
        self.total_ms += duration.as_secs_f64() * 1000.0;
        self.max = self.max.max(duration);
        self.over_budget += u64::from(duration > BUDGET);
        if self.samples.len() == SAMPLE_LIMIT {
            self.samples.pop_front();
        }
        self.samples.push_back(duration);
    }
    fn summary(&self) -> Option<Summary> {
        if self.count == 0 {
            return None;
        }
        let mut samples: Vec<_> = self.samples.iter().copied().collect();
        samples.sort_unstable();
        let rank = (samples.len() * 95).div_ceil(100) - 1;
        Some(Summary {
            count: self.count,
            mean_ms: self.total_ms / self.count as f64,
            p95_ms: samples[rank].as_secs_f64() * 1000.0,
            max_ms: self.max.as_secs_f64() * 1000.0,
            over_budget: self.over_budget,
            samples: samples.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_has_no_summary() {
        assert!(Aggregate::default().summary().is_none());
    }

    #[test]
    fn aggregates_count_mean_nearest_rank_p95_max_and_budget() {
        let mut aggregate = Aggregate::default();
        for millis in 1..=100 {
            aggregate.add(Duration::from_millis(millis));
        }
        let summary = aggregate.summary().unwrap();
        assert_eq!(summary.count, 100);
        assert!((summary.mean_ms - 50.5).abs() < 1e-9);
        assert_eq!(summary.p95_ms, 95.0);
        assert_eq!(summary.max_ms, 100.0);
        assert_eq!(summary.over_budget, 84);
    }

    #[test]
    fn bounded_samples_keep_latest_but_total_statistics_cover_window() {
        let mut aggregate = Aggregate::default();
        aggregate.add(Duration::from_secs(1));
        for _ in 0..SAMPLE_LIMIT {
            aggregate.add(Duration::from_millis(1));
        }
        let summary = aggregate.summary().unwrap();
        assert_eq!(summary.samples, SAMPLE_LIMIT);
        assert_eq!(summary.count, 257);
        assert_eq!(summary.p95_ms, 1.0);
        assert_eq!(summary.max_ms, 1000.0);
        assert_eq!(summary.over_budget, 1);
        assert!((summary.mean_ms - 1256.0 / 257.0).abs() < 1e-9);
    }

    #[test]
    fn budget_is_strictly_greater_and_single_sample_percentile_works() {
        let mut aggregate = Aggregate::default();
        aggregate.add(BUDGET);
        assert_eq!(aggregate.summary().unwrap().over_budget, 0);
        assert_eq!(aggregate.summary().unwrap().p95_ms, 16.67);
        aggregate.add(BUDGET + Duration::from_nanos(1));
        assert_eq!(aggregate.summary().unwrap().over_budget, 1);
    }
}
