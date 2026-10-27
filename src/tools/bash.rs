//! The `bash` tool: run a command in its own session, stream its output.

use crate::tools::guard;
use std::io::BufRead;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: u64 = 120;
pub const MAX_TIMEOUT: u64 = 600;
pub const MAX_OUTPUT_CHARS: usize = 30_000;

#[derive(Debug, Clone, PartialEq)]
pub enum Exit {
    Code(i32),
    Signal,
    Timeout,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BashResult {
    pub output: String,
    pub exit: Exit,
}

impl BashResult {
    /// The text the model sees: (truncated) output plus how it ended.
    pub fn for_model(&self) -> String {
        let status = match self.exit {
            Exit::Code(n) => format!("[exit code: {n}]"),
            Exit::Signal => "[killed by signal]".to_string(),
            Exit::Timeout => "[killed: timeout]".to_string(),
            Exit::Interrupted => "[killed: interrupted]".to_string(),
        };
        let out = truncate_middle(&self.output, MAX_OUTPUT_CHARS);
        if out.is_empty() { status } else { format!("{}\n{status}", out.trim_end_matches('\n')) }
    }
}

/// Keep the start and end of `s` (20% / 80%) when it exceeds `max` chars.
pub fn truncate_middle(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    let head = max / 5;
    let tail = max - head;
    let omitted = total - head - tail;
    let start: String = s.chars().take(head).collect();
    let end: String = s.chars().skip(total - tail).collect();
    format!("{start}\n[… {omitted} chars omitted …]\n{end}")
}

/// Run `command` with `bash -c` in `cwd`.
///
/// - New session (`setsid`): no controlling terminal, own process group.
/// - stdin is /dev/null; stdout and stderr share one pipe, so they interleave
///   in the order written.
/// - `on_line` sees each output line as it arrives.
/// - `cancelled` is polled every 50 ms; true kills the process group.
/// - When the command exits, any leftover background processes in its group
///   are killed too.
pub fn run(
    command: &str,
    cwd: &Path,
    shim_dir: &Path,
    timeout_secs: u64,
    on_line: &mut dyn FnMut(&str),
    cancelled: &mut dyn FnMut() -> bool,
) -> std::io::Result<BashResult> {
    let (reader, writer) = std::io::pipe()?;
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("PATH", guard::child_path(shim_dir))
        .env("SUDO_ASKPASS", "/usr/bin/false")
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer);
    // SAFETY: setsid is async-signal-safe and we touch nothing else.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    drop(cmd); // closes our copies of the pipe's write end
    let pgid = child.id() as libc::pid_t;

    let (tx, rx) = mpsc::channel::<String>();
    let reader_thread = std::thread::spawn(move || {
        let mut r = std::io::BufReader::new(reader);
        let mut buf = Vec::new();
        while let Ok(n) = r.read_until(b'\n', &mut buf) {
            if n == 0 {
                break;
            }
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
            buf.clear();
        }
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_secs.clamp(1, MAX_TIMEOUT));
    let mut output = String::new();
    let drain = |output: &mut String, on_line: &mut dyn FnMut(&str)| {
        while let Ok(chunk) = rx.try_recv() {
            on_line(chunk.trim_end_matches('\n'));
            output.push_str(&chunk);
        }
    };
    let exit = loop {
        drain(&mut output, on_line);
        if let Some(status) = child.try_wait()? {
            break match status.code() {
                Some(code) => Exit::Code(code),
                None => Exit::Signal,
            };
        }
        if cancelled() {
            kill_group(pgid);
            let _ = child.wait();
            break Exit::Interrupted;
        }
        if Instant::now() >= deadline {
            kill_group(pgid);
            let _ = child.wait();
            break Exit::Timeout;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // Background jobs may still hold the pipe open; end them so the reader finishes.
    kill_group(pgid);
    // Collect the remaining output. A process that escaped the group could keep
    // the pipe open forever, so stop waiting after 200 ms of silence.
    while let Ok(chunk) = rx.recv_timeout(Duration::from_millis(200)) {
        on_line(chunk.trim_end_matches('\n'));
        output.push_str(&chunk);
    }
    drop(reader_thread);
    Ok(BashResult { output, exit })
}

fn kill_group(pgid: libc::pid_t) {
    // SAFETY: plain syscall; an already-dead group just returns ESRCH.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}
