//! Running a helper program: once with a time limit and bounded output ([`run`]), or as a
//! stream that is read while it runs ([`Streaming`]).

use std::ffi::OsStr;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
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

/// How long a streaming program may take to exit after its output ended.
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// A running program whose stdout is read as it comes (playback). The program is killed when
/// the value is dropped, so stopping playback never leaves ffmpeg running.
pub(crate) struct Streaming {
    tool: &'static str,
    child: Child,
    stdout: Option<ChildStdout>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

/// Result of [`Streaming::fill`].
pub(crate) enum Fill {
    /// The buffer is full.
    Full,
    /// The output ended after this many bytes (less than the buffer, possibly zero).
    Ended(usize),
}

impl Streaming {
    /// Starts `program args…` without a console window, stdin closed; stderr is kept up to
    /// `max_stderr` bytes for error messages.
    pub(crate) fn spawn<I, S>(
        tool: &'static str,
        program: &Path,
        args: I,
        max_stderr: u64,
    ) -> Result<Streaming, MediaError>
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
        let stdout = child.stdout.take();
        let stderr = child
            .stderr
            .take()
            .and_then(|p| spawn_reader(p, max_stderr));
        Ok(Streaming {
            tool,
            child,
            stdout,
            stderr,
        })
    }

    /// Reads until `buf` is full or the output ends. Blocks while the program works.
    pub(crate) fn fill(&mut self, buf: &mut [u8]) -> Result<Fill, MediaError> {
        let tool = self.tool;
        let Some(stdout) = &mut self.stdout else {
            return Ok(Fill::Ended(0));
        };
        let mut filled = 0;
        while let Some(rest) = buf.get_mut(filled..) {
            if rest.is_empty() {
                return Ok(Fill::Full);
            }
            match stdout.read(rest) {
                Ok(0) => return Ok(Fill::Ended(filled)),
                Ok(n) => filled += n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(source) => return Err(MediaError::Spawn { tool, source }),
            }
        }
        Ok(Fill::Full)
    }

    /// Reads what is available (at least one byte, blocking until then); `0` at the end.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> Result<usize, MediaError> {
        let tool = self.tool;
        let Some(stdout) = &mut self.stdout else {
            return Ok(0);
        };
        loop {
            match stdout.read(buf) {
                Ok(n) => return Ok(n),
                Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
                Err(source) => return Err(MediaError::Spawn { tool, source }),
            }
        }
    }

    /// After the output ended: waits for the program to exit and returns its status and error
    /// output.
    pub(crate) fn finish(&mut self) -> Result<(ExitStatus, Vec<u8>), MediaError> {
        let tool = self.tool;
        self.stdout = None;
        let status = wait_with_timeout(&mut self.child, EXIT_TIMEOUT).map_err(|err| match err {
            WaitError::Timeout => MediaError::Timeout {
                tool,
                secs: EXIT_TIMEOUT.as_secs(),
            },
            WaitError::Io(source) => MediaError::Spawn { tool, source },
        })?;
        Ok((status, join_reader(self.stderr.take())))
    }
}

impl Drop for Streaming {
    fn drop(&mut self) {
        // Best effort: the program may have exited already.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
