//! `TARN_STARTUP_TRACE=1` prints to stderr when each startup step first finished, and
//! `TARN_FRAME_TRACE=1` prints how long each part of the shell's frames, each tab and space switch
//! and each palette query took, and each time a web view's native frame moved or was resized. Steps and sections are fixed names, so the output never carries a URL or
//! anything else from the profile.

use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Trace {
    start: Instant,
    seen: Mutex<Vec<&'static str>>,
}

impl Trace {
    fn new() -> Self {
        Trace { start: Instant::now(), seen: Mutex::default() }
    }

    /// `tarn-startup <unix ms> <ms since the first mark> <step>`, or `None` for a step already seen.
    fn record(&self, step: &'static str, unix_ms: u128) -> Option<String> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.contains(&step) {
            return None;
        }
        seen.push(step);
        Some(format!("tarn-startup {unix_ms} {} {step}", self.start.elapsed().as_millis()))
    }
}

fn enabled(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|v| v == "1")
}

static TRACE: LazyLock<Option<Trace>> = LazyLock::new(|| enabled(std::env::var_os("TARN_STARTUP_TRACE")).then(Trace::new));

/// Records `step` once; the first call starts the clock, so `main` marks first.
pub fn mark(step: &'static str) {
    let Some(trace) = &*TRACE else { return };
    let unix_ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
    if let Some(line) = trace.record(step, unix_ms) {
        eprintln!("{line}");
    }
}

static FRAMES: LazyLock<bool> = LazyLock::new(|| enabled(std::env::var_os("TARN_FRAME_TRACE")));

/// `tarn-frame <section> <µs>`: a fixed name and a number only.
fn frame_line(section: &'static str, took: Duration) -> String {
    format!("tarn-frame {section} {}", took.as_micros())
}

/// Whether `TARN_FRAME_TRACE` is on, so callers skip timing frames otherwise.
pub fn frames_traced() -> bool {
    *FRAMES
}

/// Prints how long `section` of a frame took.
pub fn frame(section: &'static str, took: Duration) {
    if *FRAMES {
        eprintln!("{}", frame_line(section, took));
    }
}

pub const TAB_SWITCH: &str = "tab-switch";
pub const PALETTE_QUERY: &str = "palette-query";
pub const SIDEBAR_FRAME: &str = "sidebar";
pub const SPACE_SWITCH: &str = "space-switch";

pub struct Span {
    section: &'static str,
    started: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        frame(self.section, self.started.elapsed());
    }
}

/// While `TARN_FRAME_TRACE` is on, a guard that prints how long it lived as `section`.
pub fn span(section: &'static str) -> Option<Span> {
    frames_traced().then(|| Span { section, started: Instant::now() })
}

/// Each section's sample count, median and 95th percentile in µs from `tarn-frame` lines, in the
/// order the sections first appear.
pub fn summarise(log: &str) -> Vec<(&str, usize, u64, u64)> {
    let mut sections: Vec<(&str, Vec<u64>)> = Vec::new();
    let samples = log.lines().filter_map(|line| {
        let mut words = line.strip_prefix("tarn-frame ")?.split(' ');
        Some((words.next()?, words.next()?.parse::<u64>().ok()?))
    });
    for (section, micros) in samples {
        match sections.iter_mut().find(|(s, _)| *s == section) {
            Some((_, found)) => found.push(micros),
            None => sections.push((section, vec![micros])),
        }
    }
    sections
        .into_iter()
        .map(|(section, mut samples)| {
            samples.sort_unstable();
            let at = |p: usize| samples[(samples.len() * p).div_ceil(100).max(1) - 1];
            (section, samples.len(), at(50), at(95))
        })
        .collect()
}

/// Prints a change of a web view's native frame: `resize` when its size changed, else `move`.
pub fn frame_set(resized: bool) {
    if *FRAMES {
        eprintln!("tarn-frame {}", if resized { "resize" } else { "move" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_lines_carry_a_section_name_and_a_number() {
        assert_eq!(frame_line("sidebar", Duration::from_micros(1234)), "tarn-frame sidebar 1234");
    }

    #[test]
    fn summaries_take_nearest_rank_percentiles_per_section() {
        let mut log: String = (1..=100).rev().map(|n| format!("tarn-frame tab-switch {n}\n")).collect();
        log.push_str("tarn-frame sidebar 7\ntarn-frame move\ntarn-startup 1 2 main\ntarn-frame sidebar x\n");
        assert_eq!(summarise(&log), [("tab-switch", 100, 50, 95), ("sidebar", 1, 7, 7)]);
        assert!(summarise("").is_empty());
    }

    #[test]
    fn only_one_turns_tracing_on() {
        assert!(enabled(Some("1".into())));
        for off in [None, Some("0".into()), Some("".into()), Some("true".into())] {
            assert!(!enabled(off));
        }
    }

    #[test]
    fn each_step_is_printed_once() {
        let trace = Trace::new();
        let line = trace.record("store open", 42).unwrap();
        assert!(line.starts_with("tarn-startup 42 ") && line.ends_with(" store open"), "{line}");
        assert_eq!(trace.record("store open", 43), None);
        assert!(trace.record("first window", 44).is_some());
    }
}
