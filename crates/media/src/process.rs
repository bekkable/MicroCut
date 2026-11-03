//! Running a helper program with a time limit and bounded output.

use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::MediaError;

/// Captured result of a finished program.
pub(crate) struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Limits for [`run`].
pub(crate) struct Limits {
    pub timeout: Duration,
    pub max_stdout: u64,
    pub max_stderr: u64,
}

/// Runs `program args…` without a console window, stdin closed. Output beyond the limits is
/// read and discarded (so the child never blocks on a full pipe). Kills the child on timeout.
pub(crate) fn run<I, S>(
    tool: &'static str,
    program: &Path,
    args: I,
    limits: &Limits,
) -> Result<Output, MediaError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);

    let mut child = command
        .spawn()
        .map_err(|source| MediaError::Spawn { tool, source })?;
    let stdout = child
        .stdout
        .take()
        .and_then(|p| spawn_reader(p, limits.max_stdout));
    let stderr = child
        .stderr
        .take()
        .and_then(|p| spawn_reader(p, limits.max_stderr));

    let status = wait_with_timeout(&mut child, limits.timeout).map_err(|err| match err {
        WaitError::Timeout => MediaError::Timeout {
            tool,
            secs: limits.timeout.as_secs(),
        },
        WaitError::Io(source) => MediaError::Spawn { tool, source },
    })?;

    Ok(Output {
        status,
        stdout: join_reader(stdout),
        stderr: join_reader(stderr),
    })
}

enum WaitError {
    Timeout,
    Io(io::Error),
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, WaitError> {
    let start = Instant::now();
    let mut pause = Duration::from_millis(2);
    loop {
        if let Some(status) = child.try_wait().map_err(WaitError::Io)? {
            return Ok(status);
        }
        if start.elapsed() >= timeout {
            // Best effort: the child may have exited in between.
            let _ = child.kill();
            let _ = child.wait();
            return Err(WaitError::Timeout);
        }
        thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// Drains `pipe` on a thread. If no thread can be created the pipe is dropped, which ends the
/// child's writes with a broken-pipe error instead of blocking it.
fn spawn_reader<R: Read + Send + 'static>(mut pipe: R, cap: u64) -> Option<JoinHandle<Vec<u8>>> {
    thread::Builder::new()
        .name("microcut-pipe".to_owned())
        .spawn(move || {
            let mut buf = Vec::new();
            // Errors only cut the output short; the exit status tells what happened.
            let _ = (&mut pipe).take(cap).read_to_end(&mut buf);
            let _ = io::copy(&mut pipe, &mut io::sink());
            buf
        })
        .ok()
}

fn join_reader(handle: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}

#[cfg(windows)]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console_window(_command: &mut Command) {}

/// Last non-empty line of `stderr`, trimmed and capped, for error messages.
pub(crate) fn last_line(stderr: &[u8]) -> String {
    const MAX_CHARS: usize = 300;
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("no error message");
    line.chars().take(MAX_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_line_skips_blank_lines_and_caps_length() {
        assert_eq!(last_line(b"first\nInvalid data\n\n  \n"), "Invalid data");
        assert_eq!(last_line(b""), "no error message");
        assert_eq!(last_line(&[b'x'; 1000]).len(), 300);
        assert_eq!(last_line(&[0xff, 0xfe, b'\n']), "\u{fffd}\u{fffd}");
    }
}
