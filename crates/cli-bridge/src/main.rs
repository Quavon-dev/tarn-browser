#![forbid(unsafe_code)]

#[cfg(unix)]
mod unix;

#[cfg(unix)]
fn main() -> std::process::ExitCode {
    unix::main()
}

#[cfg(not(unix))]
fn main() -> std::process::ExitCode {
    std::process::ExitCode::FAILURE
}
