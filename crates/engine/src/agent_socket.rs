//! The local channel an AI agent uses to reach a running Tarn (S31): a Unix socket that only the
//! data folder's owner can open, or on Windows a named pipe for the current user that refuses
//! remote clients. `tarn --mcp` relays the agent's stdio to it. One request is answered before the
//! next is read, so neither end reads and writes at once (a synchronous Windows pipe cannot).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

pub use imp::{Listener, Stream, connect};

/// The longest answer the relay passes on, well above a base64 screenshot.
const MAX_ANSWER_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_CONNECTIONS: usize = 8;

/// Numbers connections across every server of this run, so no two share a number.
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);
static OPEN_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// Accepts agents on the data folder's socket, each on a thread of its own, until `stop`.
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: std::thread::JoinHandle<()>,
    data_dir: PathBuf,
}

impl Server {
    /// `serve` answers one connection with its number; one over `MAX_CONNECTIONS` gets `busy` as
    /// an error and is closed.
    pub fn start(data_dir: &Path, busy: &str, serve: impl Fn(u64, Stream) + Send + Sync + 'static) -> io::Result<Server> {
        let listener = Listener::bind(data_dir)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let busy = format!("{}\n", offline_error("", busy));
        let serve = Arc::new(serve);
        let thread = std::thread::spawn(move || {
            while let Ok(stream) = listener.accept() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                if OPEN_CONNECTIONS.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    OPEN_CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
                    let _ = (&stream).write_all(busy.as_bytes());
                    continue;
                }
                let conn = NEXT_CONNECTION.fetch_add(1, Ordering::SeqCst);
                let serve = serve.clone();
                std::thread::spawn(move || {
                    serve(conn, stream);
                    OPEN_CONNECTIONS.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        Ok(Server { stop, thread, data_dir: data_dir.to_owned() })
    }

    /// Closes the socket once the accepting thread has woken and ended; connections already
    /// open keep running until their agent goes.
    pub fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        if connect(&self.data_dir).is_ok() {
            let _ = self.thread.join();
        }
    }
}

/// FNV-1a of the data folder's lowercased path, which names its Tarn the same in every build;
/// Windows paths ignore case.
#[cfg(windows)]
pub(crate) fn data_dir_hash(data_dir: &str) -> u64 {
    fnv(data_dir.to_lowercase().as_bytes())
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// Whether the server answers `line`: everything but a notification (a method without an id)
/// and a response (no method), which the server never sends back.
pub fn expects_reply(line: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(msg)) => msg.contains_key("method") && msg.contains_key("id"),
        _ => true,
    }
}

/// Copies the agent's requests from stdin to Tarn and each answer back to stdout, until either
/// side closes. Without a running Tarn, every request is answered with `offline`.
pub fn relay(data_dir: &Path, offline: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    let stream = connect(data_dir).ok();
    let mut channel = match &stream {
        Some(s) => Some((BufReader::new(s.try_clone()?), s.try_clone()?)),
        None => None,
    };
    // The agent's input is read on its own thread so that the agent going away ends the relay at
    // once, also while a call waits for the user; Tarn then sees the connection close.
    let (lines, input) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
        std::process::exit(0);
    });
    for line in input {
        if line.trim().is_empty() {
            continue;
        }
        let reply = expects_reply(&line);
        let answer = match channel.as_mut() {
            Some((reader, writer)) => {
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
                if !reply {
                    continue;
                }
                let mut answer = String::new();
                loop {
                    answer.clear();
                    if reader.by_ref().take(MAX_ANSWER_BYTES).read_line(&mut answer)? == 0 {
                        return Ok(());
                    }
                    if answer.trim_end() != HEARTBEAT {
                        break;
                    }
                }
                answer
            }
            None if reply => format!("{}\n", offline_error(&line, offline)),
            None => continue,
        };
        stdout.write_all(answer.as_bytes())?;
        stdout.flush()?;
    }
    Ok(())
}

/// What Tarn sends while a call waits, so that a failed write tells it the agent has gone; the
/// relay drops it.
pub const HEARTBEAT: &str = r#"{"jsonrpc":"2.0","method":"tarn/alive"}"#;

/// Waits for the answer to a call, writing `HEARTBEAT` to `stream` every second meanwhile;
/// `None` once the agent's end of the connection has closed or the answer can never come.
pub fn await_answer<T>(answer: &std::sync::mpsc::Receiver<T>, stream: &Stream) -> Option<T> {
    loop {
        match answer.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(value) => return Some(value),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let mut out = stream;
                if out.write_all(format!("{HEARTBEAT}\n").as_bytes()).and_then(|()| out.flush()).is_err() {
                    return None;
                }
            }
        }
    }
}

fn offline_error(line: &str, message: &str) -> String {
    let id = serde_json::from_str::<serde_json::Value>(line).ok().and_then(|v| v.get("id").cloned()).unwrap_or_default();
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": message } }).to_string()
}

#[cfg(unix)]
mod imp {
    use std::fs;
    use std::io;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

    pub type Stream = UnixStream;

    const SOCKET: &str = "agent.sock";

    /// In the data folder, or for a folder whose path is too long for a socket address, in the
    /// temporary folder under a name of its own; `ours` refuses one another user made there.
    fn path(data_dir: &Path) -> io::Result<PathBuf> {
        let fits = |p: &PathBuf| p.as_os_str().len() < 100;
        let inside = data_dir.join(SOCKET);
        if fits(&inside) {
            return Ok(inside);
        }
        let hash = super::fnv(std::os::unix::ffi::OsStrExt::as_bytes(data_dir.as_os_str()));
        let temporary = std::env::temp_dir().join(format!("tarn-agent-{hash:016x}.sock"));
        if fits(&temporary) { Ok(temporary) } else { Err(io::Error::other("no path short enough for the agent socket")) }
    }

    /// A socket at `path` that the data folder's owner made and only they can open.
    fn ours(path: &Path, data_dir: &Path) -> io::Result<bool> {
        let meta = fs::symlink_metadata(path)?;
        let owner = fs::metadata(data_dir)?.uid();
        Ok(meta.file_type().is_socket() && meta.uid() == owner && meta.mode() & 0o077 == 0)
    }

    /// Binds in a new folder only this user can enter, makes the socket 0600 there, then moves
    /// it into place, so it is never reachable with the umask's permissions.
    fn bind_private(path: &Path) -> io::Result<(UnixListener, u64)> {
        use std::os::unix::fs::DirBuilderExt;
        let parent = path.parent().ok_or_else(|| io::Error::other("the agent socket has no folder"))?;
        let staging = parent.join(format!(".t{}", std::process::id()));
        let staged = staging.join("s");
        if staged.as_os_str().len() >= 100 {
            return Err(io::Error::other("no path short enough for the agent socket"));
        }
        let _ = fs::remove_dir_all(&staging);
        fs::DirBuilder::new().mode(0o700).create(&staging)?;
        let bound = UnixListener::bind(&staged).and_then(|listener| {
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))?;
            fs::rename(&staged, path)?;
            Ok((listener, fs::symlink_metadata(path)?.ino()))
        });
        let _ = fs::remove_dir_all(&staging);
        bound
    }

    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
        ino: u64,
    }

    impl Listener {
        pub fn bind(data_dir: &Path) -> io::Result<Listener> {
            let path = path(data_dir)?;
            if fs::symlink_metadata(&path).is_ok() {
                if UnixStream::connect(&path).is_ok() {
                    return Err(io::ErrorKind::AddrInUse.into());
                }
                let meta = fs::symlink_metadata(&path)?;
                if !meta.file_type().is_socket() || meta.uid() != fs::metadata(data_dir)?.uid() {
                    return Err(io::Error::other("something else is at the agent socket's path"));
                }
                fs::remove_file(&path)?;
            }
            let (inner, ino) = bind_private(&path)?;
            Ok(Listener { inner, path, ino })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            self.inner.accept().map(|(stream, _)| stream)
        }
    }

    /// Removes the socket unless a newer listener made its own at the same path.
    impl Drop for Listener {
        fn drop(&mut self) {
            if fs::symlink_metadata(&self.path).is_ok_and(|m| m.ino() == self.ino) {
                let _ = fs::remove_file(&self.path);
            }
        }
    }

    pub fn connect(data_dir: &Path) -> io::Result<Stream> {
        let path = path(data_dir)?;
        if !ours(&path, data_dir)? {
            return Err(io::Error::other("the agent socket is not Tarn's"));
        }
        UnixStream::connect(&path)
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;
    use std::sync::Mutex;

    use windows_sys::Win32::Foundation::{ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    use crate::host_windows::wide;

    pub type Stream = File;

    /// Full access for the pipe's owner, the user Tarn runs as, and no one else.
    const OWNER_ONLY: &str = "D:P(A;;GA;;;OW)";

    fn name(data_dir: &Path) -> String {
        format!(r"\\.\pipe\Tarn.Agent.{:016x}", super::data_dir_hash(&data_dir.to_string_lossy()))
    }

    fn instance(name: &[u16], first: bool) -> io::Result<OwnedHandle> {
        let sddl = wide(OWNER_ONLY);
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `sddl` is a NUL-terminated wide string and `descriptor` a valid out-parameter; the
        // descriptor is freed with `LocalFree` below.
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut descriptor, std::ptr::null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor, bInheritHandle: 0 };
        let open = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: `name` is NUL-terminated and `attributes` points at the descriptor made above,
        // which stays alive for the call.
        let handle = unsafe { CreateNamedPipeW(name.as_ptr(), open, mode, PIPE_UNLIMITED_INSTANCES, 64 * 1024, 64 * 1024, 0, &attributes) };
        let error = io::Error::last_os_error();
        // SAFETY: `descriptor` came from `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
        unsafe { LocalFree(descriptor) };
        if handle == INVALID_HANDLE_VALUE {
            return Err(error);
        }
        // SAFETY: a valid pipe handle that nothing else owns.
        Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
    }

    pub struct Listener {
        name: Vec<u16>,
        next: Mutex<Option<OwnedHandle>>,
    }

    impl Listener {
        /// Fails when another program already made the pipe, so none can pose as Tarn's.
        pub fn bind(data_dir: &Path) -> io::Result<Listener> {
            let name = wide(&name(data_dir));
            let first = instance(&name, true)?;
            Ok(Listener { name, next: Mutex::new(Some(first)) })
        }

        pub fn accept(&self) -> io::Result<Stream> {
            let mut next = self.next.lock().map_err(|_| io::Error::other("poisoned"))?;
            loop {
                let handle = match next.take() {
                    Some(handle) => handle,
                    None => instance(&self.name, false)?,
                };
                // SAFETY: a pipe instance this listener made; blocks until a client connects.
                let connected = unsafe { ConnectNamedPipe(handle.as_raw_handle(), std::ptr::null_mut()) } != 0;
                // SAFETY: reads the calling thread's last error, set by the call above.
                let error = if connected { 0 } else { unsafe { GetLastError() } };
                // A client that came and went before the connect leaves a dead instance. Its
                // replacement exists before it closes, so the name is never free for another program.
                if error == ERROR_NO_DATA {
                    *next = Some(instance(&self.name, false)?);
                    drop(handle);
                    continue;
                }
                if error != 0 && error != ERROR_PIPE_CONNECTED {
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                *next = instance(&self.name, false).ok();
                return Ok(File::from(handle));
            }
        }
    }

    /// Only a pipe this very executable serves, so another program that made it first is refused.
    pub fn connect(data_dir: &Path) -> io::Result<Stream> {
        let open = || OpenOptions::new().read(true).write(true).security_qos_flags(SECURITY_IDENTIFICATION).open(name(data_dir));
        let mut pipe = open();
        for _ in 0..50 {
            match &pipe {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    pipe = open();
                }
                _ => break,
            }
        }
        let pipe = pipe?;
        let mut pid = 0;
        // SAFETY: a pipe handle `pipe` owns and a valid out-parameter.
        let known = unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } != 0;
        if !known || !crate::instance::runs_this_exe(pid) {
            return Err(io::Error::other("the agent pipe is not Tarn's"));
        }
        Ok(pipe)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn a_server_numbers_connections_turns_away_the_surplus_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = Server::start(dir.path(), "busy", move |conn, stream| {
            tx.send(conn).unwrap();
            let mut line = String::new();
            let _ = BufReader::new(stream).read_line(&mut line);
        })
        .unwrap();
        let clients: Vec<_> = (0..MAX_CONNECTIONS).map(|_| connect(dir.path()).unwrap()).collect();
        let mut seen: Vec<u64> = (0..MAX_CONNECTIONS).map(|_| rx.recv().unwrap()).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), MAX_CONNECTIONS);
        let mut answer = String::new();
        BufReader::new(connect(dir.path()).unwrap()).read_line(&mut answer).unwrap();
        assert!(answer.contains("busy"), "{answer}");
        drop(clients);
        server.stop();
        assert!(!dir.path().join("agent.sock").exists());
        assert!(connect(dir.path()).is_err());
        let again = Server::start(dir.path(), "busy", |_, _| {}).unwrap();
        again.stop();
    }

    #[test]
    fn a_wait_notices_the_agent_going_away() {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        let (_keep, answer) = std::sync::mpsc::channel::<u8>();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(&theirs).read_line(&mut line).unwrap();
            assert_eq!(line.trim_end(), HEARTBEAT);
            drop(theirs);
        });
        assert_eq!(await_answer(&answer, &ours), None);
        reader.join().unwrap();
    }

    #[test]
    fn requests_and_notifications_are_told_apart() {
        assert!(expects_reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#));
        assert!(!expects_reply(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#));
        assert!(!expects_reply(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#));
        assert!(expects_reply("not json"));
        assert!(expects_reply("[]"));
    }

    #[test]
    fn a_round_trip_through_the_socket_and_cleanup_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = Listener::bind(dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join("agent.sock")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let server = std::thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            (&stream).write_all(format!("echo {line}").as_bytes()).unwrap();
            listener
        });
        let mut client = connect(dir.path()).unwrap();
        client.write_all(b"hello\n").unwrap();
        let mut answer = String::new();
        BufReader::new(client).read_line(&mut answer).unwrap();
        assert_eq!(answer, "echo hello\n");
        let listener = server.join().unwrap();
        assert_eq!(Listener::bind(dir.path()).err().map(|e| e.kind()), Some(io::ErrorKind::AddrInUse));
        drop(listener);
        assert!(!dir.path().join("agent.sock").exists());
    }

    #[test]
    fn a_stale_socket_is_replaced_and_a_foreign_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let stale = std::os::unix::net::UnixListener::bind(dir.path().join("agent.sock")).unwrap();
        drop(stale);
        drop(Listener::bind(dir.path()).unwrap());
        std::fs::write(dir.path().join("agent.sock"), "not a socket").unwrap();
        assert!(Listener::bind(dir.path()).is_err());
        assert!(dir.path().join("agent.sock").exists());
    }

    #[test]
    fn a_socket_others_can_open_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let listener = Listener::bind(dir.path()).unwrap();
        std::fs::set_permissions(dir.path().join("agent.sock"), std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(connect(dir.path()).is_err());
        drop(listener);
        assert!(connect(dir.path()).is_err());
    }

    #[test]
    fn a_data_folder_too_deep_for_a_socket_uses_the_temporary_folder() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("d".repeat(120));
        std::fs::create_dir(&deep).unwrap();
        let listener = Listener::bind(&deep).unwrap();
        assert!(!deep.join("agent.sock").exists());
        let mut client = connect(&deep).unwrap();
        client.write_all(b"x\n").unwrap();
        drop(listener.accept().unwrap());
        drop(listener);
        assert!(connect(&deep).is_err());
    }

    #[test]
    fn offline_errors_keep_the_request_id() {
        let e: serde_json::Value = serde_json::from_str(&offline_error(r#"{"jsonrpc":"2.0","id":"x","method":"initialize"}"#, "off")).unwrap();
        assert_eq!(e["id"], "x");
        assert_eq!(e["error"]["message"], "off");
    }
}
