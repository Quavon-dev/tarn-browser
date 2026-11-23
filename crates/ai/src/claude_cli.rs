use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use serde_json::json;

use crate::error::AiError;
use crate::provider::{Provider, Request};

const HELPER: &str = "tarn-cli-bridge";

/// Talks to the `tarn-cli-bridge` helper, which runs the user's Claude CLI with tools disabled.
#[derive(Debug, Clone)]
pub struct ClaudeCli {
    cli_path: PathBuf,
    model: Option<String>,
    helper: PathBuf,
}

impl ClaudeCli {
    pub fn new(cli_path: PathBuf, model: Option<String>) -> Self {
        let helper = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join(HELPER))).unwrap_or_default();
        Self { cli_path, model, helper }
    }
}

impl Provider for ClaudeCli {
    fn stream(&self, req: &Request, cancel: &AtomicBool, on_text: &mut dyn FnMut(&str)) -> Result<(), AiError> {
        let cli_err = |e: std::io::Error| AiError::Cli(e.to_string());
        let input = json!({
            "cli_path": self.cli_path,
            "model": self.model,
            "prompt": format!("{}\n\n{}", req.system, req.user_message()),
        });
        let mut child = helper_command(&self.helper).spawn().map_err(cli_err)?;
        let mut stdin = child.stdin.take().ok_or_else(|| AiError::Cli("no stdin".into()))?;
        stdin.write_all(input.to_string().as_bytes()).map_err(cli_err)?;
        drop(stdin);

        let mut stdout = child.stdout.take().ok_or_else(|| AiError::Cli("no stdout".into()))?;
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        let mut pending = Vec::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                // shortcut: the CLI runs in the bridge's own process group and outlives this kill
                // until its next write fails; send the bridge a cancel once the XPC helper exists.
                let _ = child.kill();
                let _ = child.wait();
                return Err(AiError::Cancelled);
            }
            let chunk = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(chunk) => chunk,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            pending.extend_from_slice(&chunk);
            let valid = match std::str::from_utf8(&pending) {
                Ok(s) => s.len(),
                Err(e) => e.valid_up_to(),
            };
            if valid > 0 {
                on_text(std::str::from_utf8(&pending[..valid]).unwrap_or_default());
                pending.drain(..valid);
            }
        }
        let mut stderr = String::new();
        if let Some(e) = child.stderr.take() {
            let _ = e.take(1024).read_to_string(&mut stderr);
        }
        let status = child.wait().map_err(cli_err)?;
        if status.success() { Ok(()) } else { Err(AiError::Cli(stderr.trim().chars().filter(|c| !c.is_control()).take(300).collect())) }
    }
}

/// The helper gets only `HOME`, which the CLI needs for its login.
fn helper_command(helper: &Path) -> Command {
    let mut command = Command::new(helper);
    command.env_clear().envs(std::env::var_os("HOME").map(|home| ("HOME", home))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn streams_helper_stdout_and_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("helper");
        std::fs::write(&helper, "#!/bin/sh\ncat\n").unwrap();
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let cli = ClaudeCli { cli_path: "/x/claude".into(), model: None, helper: helper.clone() };
        let req = Request { system: "S".into(), page: None, question: "ünïcode?".into() };
        let mut out = String::new();
        cli.stream(&req, &AtomicBool::new(false), &mut |t| out.push_str(t)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["cli_path"], "/x/claude");
        assert_eq!(v["prompt"], "S\n\nünïcode?");

        std::fs::write(&helper, "#!/bin/sh\necho refused >&2\nexit 2\n").unwrap();
        let e = cli.stream(&req, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(matches!(e, AiError::Cli(ref m) if m == "refused"));

        std::fs::write(&helper, "#!/bin/sh\nprintf %s \"$HOME|$USER\"\n").unwrap();
        let mut env = String::new();
        cli.stream(&req, &AtomicBool::new(false), &mut |t| env.push_str(t)).unwrap();
        assert_eq!(env, format!("{}|", std::env::var("HOME").unwrap()));
    }
}
