//! `TARN_STARTUP_TRACE=1` prints to stderr when each startup step first finished, and
//! `TARN_FRAME_TRACE=1` prints how long each part of the shell's frames took and each time a web
//! view's native frame moved or was resized. Steps and sections are fixed names, so the output never carries a URL or
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
