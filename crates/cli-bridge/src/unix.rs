use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

const MAX_INPUT_BYTES: u64 = 1024 * 1024;
const MAX_STDOUT_BYTES: usize = 256 * 1024;
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Every phrase must appear in `--help`, or the CLI cannot prove tools and MCP are off.
const REQUIRED_HELP: &[&str] = &[
    "--print",
    "--tools",
    r#"Use "" to disable all tools"#,
    "--strict-mcp-config",
    "--mcp-config",
    "--permission-mode",
    "dontAsk",
    "--disable-slash-commands",
    "--safe-mode",
    "--no-session-persistence",
    "--output-format",
    "--model",
];

const FIXED_ARGS: &[&str] = &[
    "--print",
    "--tools",
    "",
    "--strict-mcp-config",
    "--mcp-config",
    r#"{"mcpServers":{}}"#,
    "--permission-mode",
    "dontAsk",
    "--disable-slash-commands",
    "--safe-mode",
    "--no-session-persistence",
    "--output-format",
    "text",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BridgeRequest {
    cli_path: PathBuf,
    model: Option<String>,
    prompt: String,
    timeout_secs: Option<u64>,
}

pub(crate) fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin().take(MAX_INPUT_BYTES).read_to_string(&mut input).map_err(|e| format!("reading request: {e}"))?;
    let req: BridgeRequest = serde_json::from_str(&input).map_err(|e| format!("invalid request: {e}"))?;
    let cli = validate_cli(&req.cli_path)?;
    let model = req.model.as_deref().map(validate_model).transpose()?;
    let timeout = req.timeout_secs.map_or(MAX_TIMEOUT, |s| Duration::from_secs(s).min(MAX_TIMEOUT));

    probe(&cli)?;

    let mut args: Vec<&str> = FIXED_ARGS.to_vec();
    if let Some(m) = model {
        args.extend(["--model", m]);
    }
    let status = run_cli(&cli, &args, req.prompt.as_bytes(), timeout, &mut std::io::stdout())?;
    if status { Ok(()) } else { Err("claude cli failed".into()) }
}

fn validate_cli(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("cli path must be absolute".into());
    }
    let real = path.canonicalize().map_err(|e| format!("cli path: {e}"))?;
    let meta = real.metadata().map_err(|e| format!("cli path: {e}"))?;
    if !meta.is_file() {
        return Err("cli path is not a regular file".into());
    }
    let me = tempfile::tempdir().and_then(|dir| dir.path().metadata()).map_err(|e| format!("temp dir: {e}"))?.uid();
    // The binary and every directory above it (as given and as resolved) must be owned by the
    // user or root and writable by nobody else, or another account could swap the CLI. Group
    // write is allowed only for `wheel` and `admin` (Homebrew's bin is 775 admin), whose members
    // can become root anyway; every local user is in `staff`.
    let mut checks: Vec<PathBuf> = real.ancestors().map(Path::to_path_buf).collect();
    checks.extend(path.ancestors().skip(1).map(Path::to_path_buf));
    for p in checks {
        let meta = p.metadata().map_err(|e| format!("{}: {e}", p.display()))?;
        let mode = meta.permissions().mode();
        let foreign = meta.uid() != me && meta.uid() != 0;
        let sticky_dir = meta.is_dir() && mode & 0o1000 != 0;
        let group_write = mode & 0o020 != 0 && !matches!(meta.gid(), 0 | 80);
        if foreign || group_write || (mode & 0o002 != 0 && !sticky_dir) {
            return Err(format!("not safe to run, writable by others: {}", p.display()));
        }
    }
    Ok(real)
}

/// Kills the CLI and everything it started (it runs in its own process group).
fn kill_group(child: &mut Child) {
    let _ = Command::new("/bin/kill").args(["-KILL", "--", &format!("-{}", child.id())]).stderr(Stdio::null()).status();
    let _ = child.kill();
    let _ = child.wait();
}

fn validate_model(m: &str) -> Result<&str, String> {
    let ok = !m.is_empty() && m.len() <= 64 && !m.starts_with('-') && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '[' | ']'));
    if ok { Ok(m) } else { Err("invalid model name".into()) }
}

fn probe(cli: &Path) -> Result<(), String> {
    let mut version = Vec::new();
    let ok = run_cli(cli, &["--version"], b"", PROBE_TIMEOUT, &mut version)?;
    let version = String::from_utf8_lossy(&version);
    if !ok || !version.chars().any(|c| c.is_ascii_digit()) {
        return Err("could not read the claude cli version".into());
    }
    let mut help = Vec::new();
    let ok = run_cli(cli, &["--help"], b"", PROBE_TIMEOUT, &mut help)?;
    let help = String::from_utf8_lossy(&help).split_whitespace().collect::<Vec<_>>().join(" ");
    let missing: Vec<_> = REQUIRED_HELP.iter().filter(|f| !help.contains(*f)).collect();
    if !ok || !missing.is_empty() {
        return Err(format!("refusing to run: claude cli {} cannot confirm that tools and MCP servers are disabled (missing {missing:?})", version.trim()));
    }
    Ok(())
}

/// Runs the CLI with no shell, a minimal environment and an empty temporary working directory.
fn run_cli(cli: &Path, args: &[&str], stdin: &[u8], timeout: Duration, out: &mut dyn Write) -> Result<bool, String> {
    let cwd = tempfile::tempdir().map_err(|e| format!("temp dir: {e}"))?;
    let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
    let mut child = Command::new(cli)
        .args(args)
        .process_group(0)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "en_US.UTF-8")
        .current_dir(cwd.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("starting claude cli: {e}"))?;

    let mut child_in = child.stdin.take().ok_or("no stdin")?;
    let input = stdin.to_vec();
    thread::spawn(move || {
        let _ = child_in.write_all(&input);
    });
    let mut child_err = child.stderr.take().ok_or("no stderr")?;
    let (err_tx, err_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_err.by_ref().take(4096).read_to_end(&mut buf);
        let _ = std::io::copy(&mut child_err, &mut std::io::sink());
        let _ = err_tx.send(buf);
    });
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let mut child_out = child.stdout.take().ok_or("no stdout")?;
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = child_out.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + timeout;
    let mut written = 0;
    let mut leader_killed = false;
    let result = loop {
        let tick = deadline.saturating_duration_since(Instant::now()).min(Duration::from_millis(100));
        match rx.recv_timeout(tick) {
            Ok(chunk) => {
                written += chunk.len();
                if written > MAX_STDOUT_BYTES {
                    break Err("claude cli output exceeded the size limit".to_string());
                }
                if out.write_all(&chunk).and_then(|_| out.flush()).is_err() {
                    break Err("output closed".to_string());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                break wait(&mut child, deadline);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if Instant::now() >= deadline => {
                break Err("claude cli timed out".to_string());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // The CLI exited but something it started still holds stdout open.
                if !leader_killed && matches!(child.try_wait(), Ok(Some(_))) {
                    kill_group(&mut child);
                    leader_killed = true;
                }
            }
        }
    };
    // Also on success: subprocesses the CLI left behind could hold the pipes open past the
    // deadline. Threads are never joined, so one that escaped the group cannot block us.
    kill_group(&mut child);
    let ok = result?;
    if !ok {
        let stderr = err_rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
        let stderr: String = String::from_utf8_lossy(&stderr).chars().filter(|c| !c.is_control()).take(300).collect();
        if !stderr.is_empty() {
            eprintln!("claude cli: {stderr}");
        }
    }
    Ok(ok)
}

fn wait(child: &mut Child, deadline: Instant) -> Result<bool, String> {
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(status.success());
        }
        if Instant::now() >= deadline {
            return Err("claude cli timed out".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}
