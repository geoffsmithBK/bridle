//! The `bash` tool: run a command in its own session, stream its output.

use crate::tools::guard;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: u64 = 120;
pub const MAX_TIMEOUT: u64 = 600;
pub const MAX_OUTPUT_CHARS: usize = 30_000;
/// How much of a command's output is kept: the first HEAD_KEEP and last
/// TAIL_KEEP bytes. Everything between is counted and dropped as it arrives,
/// so a runaway command can't exhaust memory.
const HEAD_KEEP: usize = 5_000;
const TAIL_KEEP: usize = 24_000;
/// At most this many lines go to `on_line` per 50 ms poll; the UI only shows
/// the last few, and a flood of lines would only slow it down.
const LINES_PER_POLL: usize = 8;
/// A line with no newline yet is shown once it reaches this length.
const MAX_LINE: usize = 2_000;

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

/// Largest index <= `i` that is on a char boundary of `s`.
fn floor_boundary(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Collects output: a bounded head/tail capture for the model, plus complete
/// lines waiting to be shown live.
#[derive(Default)]
struct Collector {
    head: String,
    tail: String,
    omitted: usize,
    line: String,
    lines: Vec<String>,
}

impl Collector {
    fn push(&mut self, text: &str) {
        let cut = floor_boundary(text, HEAD_KEEP.saturating_sub(self.head.len()));
        self.head.push_str(&text[..cut]);
        self.tail.push_str(&text[cut..]);
        if self.tail.len() > 2 * TAIL_KEEP {
            self.trim_tail();
        }
        for c in text.chars() {
            if c == '\n' || self.line.len() >= MAX_LINE {
                self.lines.push(std::mem::take(&mut self.line));
                if self.lines.len() > LINES_PER_POLL {
                    self.lines.remove(0);
                }
            }
            if c != '\n' {
                self.line.push(c);
            }
        }
    }

    fn trim_tail(&mut self) {
        if self.tail.len() > TAIL_KEEP {
            let cut = floor_boundary(&self.tail, self.tail.len() - TAIL_KEEP);
            self.omitted += self.tail[..cut].chars().count();
            self.tail.drain(..cut);
        }
    }

    /// Hand the waiting lines to the UI.
    fn flush_lines(&mut self, on_line: &mut dyn FnMut(&str)) {
        for l in self.lines.drain(..) {
            on_line(&l);
        }
    }

    fn finish(mut self, on_line: &mut dyn FnMut(&str)) -> String {
        if !self.line.is_empty() {
            let last = std::mem::take(&mut self.line);
            self.lines.push(last);
        }
        self.flush_lines(on_line);
        self.trim_tail();
        if self.omitted == 0 {
            self.head + &self.tail
        } else {
            format!("{}\n[… {} chars omitted …]\n{}", self.head, self.omitted, self.tail)
        }
    }
}

/// Decode bytes as UTF-8, holding back an incomplete character at the end
/// (in `carry`) so a character split across two reads isn't mangled.
fn decode(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
    carry.extend_from_slice(bytes);
    let valid = match std::str::from_utf8(carry) {
        Ok(_) => carry.len(),
        Err(e) if e.error_len().is_none() => e.valid_up_to(),
        Err(_) => carry.len(),
    };
    let text = String::from_utf8_lossy(&carry[..valid]).into_owned();
    carry.drain(..valid);
    text
}

/// Run `command` with `bash -c` in `cwd`.
///
/// - New session (`setsid`): no controlling terminal, own process group.
/// - stdin is /dev/null; stdout and stderr share one pipe, so they interleave
///   in the order written.
/// - `on_line` sees output lines as they arrive (at most a few per 50 ms; a
///   flood is thinned to its latest lines).
/// - `cancelled` is polled every 50 ms; true kills the process group.
/// - When the command exits, any leftover background processes in its group
///   are killed too.
/// - Memory stays bounded however much the command prints.
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

    // A bounded channel: if we fall behind, the reader (and so the command) waits.
    let (tx, rx) = mpsc::sync_channel::<String>(64);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 8192];
        let mut carry = Vec::new();
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(decode(&mut carry, &buf[..n])).is_err() {
                break;
            }
        }
        if !carry.is_empty() {
            let _ = tx.send(String::from_utf8_lossy(&carry).into_owned());
        }
    });

    let deadline = Instant::now() + Duration::from_secs(timeout_secs.clamp(1, MAX_TIMEOUT));
    let mut out = Collector::default();
    let exit = loop {
        while let Ok(text) = rx.try_recv() {
            out.push(&text);
        }
        out.flush_lines(on_line);
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
    // Collect what is left. A process that escaped the group could keep the
    // pipe busy forever, so stop after 200 ms of silence or 1 s in total.
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until && !cancelled() {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(text) => out.push(&text),
            Err(_) => break,
        }
    }
    // Dropping `rx` makes the reader thread's next send fail, so it exits.
    Ok(BashResult { output: out.finish(on_line), exit })
}

fn kill_group(pgid: libc::pid_t) {
    // SAFETY: plain syscall; an already-dead group just returns ESRCH.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}
