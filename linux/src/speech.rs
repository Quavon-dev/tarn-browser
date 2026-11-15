use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

const PROGRAM: &str = "spd-say";
pub const SPEEDS: [f32; 5] = [1., 1.25, 1.5, 2., 0.75];

static SPEAKING: Mutex<Option<Child>> = Mutex::new(None);

fn slot() -> std::sync::MutexGuard<'static, Option<Child>> {
    SPEAKING.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn find(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path?).map(|dir| dir.join(PROGRAM)).find(|p| Path::new(p).is_file())
}

pub fn available() -> bool {
    find(std::env::var_os("PATH").as_deref()).is_some()
}

pub fn rate(speed: f32) -> i32 {
    (((speed - 1.) * 100.).round() as i32).clamp(-100, 100)
}

pub fn speaking() -> bool {
    slot().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
}

pub fn speak(text: &str, speed: f32) -> bool {
    stop();
    let Some(program) = find(std::env::var_os("PATH").as_deref()) else { return false };
    let spawned = Command::new(program)
        .args(["--wait", "--pipe-mode", "--rate", &rate(speed).to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = spawned else { return false };
    let text = text.to_owned();
    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || drop(stdin.write_all(text.as_bytes())));
    }
    *slot() = Some(child);
    true
}

pub fn stop() {
    let Some(mut child) = slot().take() else { return };
    child.kill().ok();
    std::thread::spawn(move || child.wait());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speeds_map_to_speech_dispatcher_rates_and_the_program_is_found_on_path() {
        assert_eq!(SPEEDS.map(rate), [0, 25, 50, 100, -25]);
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find(Some(dir.path().as_os_str())), None);
        std::fs::write(dir.path().join(PROGRAM), b"").unwrap();
        assert_eq!(find(Some(dir.path().as_os_str())), Some(dir.path().join(PROGRAM)));
        assert_eq!(find(None), None);
    }

    #[test]
    fn stopping_on_another_thread_kills_only_our_own_speaker() {
        let child = Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        std::thread::spawn(move || *slot() = Some(child)).join().unwrap();
        assert!(speaking());
        stop();
        assert!(!speaking());
        let gone = (0..100).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(20));
            Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| !s.success())
        });
        assert!(gone);
    }
}
