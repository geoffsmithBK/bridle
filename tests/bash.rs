mod common;
use bridle::tools::bash::{BashResult, Exit, run, truncate_middle};
use bridle::tools::guard::{check_command, make_shim_dir};
use std::time::Instant;

fn sh(cmd: &str, timeout: u64) -> (BashResult, Vec<String>) {
    let dir = common::temp_dir();
    let shim = make_shim_dir().unwrap();
    let mut lines = vec![];
    let r = run(cmd, &dir, &shim, timeout, &mut |l| lines.push(l.to_string()), &mut || false).unwrap();
    (r, lines)
}

#[test]
fn captures_stdout_stderr_and_exit_code() {
    let (r, lines) = sh("echo out; echo err >&2; exit 3", 10);
    assert_eq!(r.exit, Exit::Code(3));
    assert_eq!(lines, vec!["out", "err"]);
    assert_eq!(r.for_model(), "out\nerr\n[exit code: 3]");
}

#[test]
fn runs_in_cwd_without_a_tty_or_stdin() {
    let dir = common::temp_dir();
    let r = run("pwd; tty; cat", &dir, &make_shim_dir().unwrap(), 10, &mut |_| {}, &mut || false).unwrap();
    assert!(r.output.starts_with(&format!("{}\nnot a tty\n", dir.display())), "{}", r.output);
    assert_eq!(r.exit, Exit::Code(0)); // cat got EOF from /dev/null instead of hanging
}

#[test]
fn sudo_on_path_is_the_shim() {
    let (r, _) = sh("sudo true", 10);
    assert_eq!(r.exit, Exit::Code(1));
    assert!(r.output.contains("sudo is disabled inside bridle"));
}

#[test]
fn timeout_kills_the_command() {
    let start = Instant::now();
    let (r, _) = sh("sleep 30", 1);
    assert_eq!(r.exit, Exit::Timeout);
    assert!(start.elapsed().as_secs() < 5);
    assert!(r.for_model().ends_with("[killed: timeout]"));
}

#[test]
fn cancel_kills_the_command() {
    let dir = common::temp_dir();
    let start = Instant::now();
    let mut polls = 0;
    let r = run("sleep 30", &dir, &make_shim_dir().unwrap(), 60, &mut |_| {}, &mut || {
        polls += 1;
        polls > 3
    })
    .unwrap();
    assert_eq!(r.exit, Exit::Interrupted);
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn background_jobs_do_not_hang_the_tool() {
    let start = Instant::now();
    let (r, _) = sh("sleep 30 & echo started", 10);
    assert_eq!(r.exit, Exit::Code(0));
    assert!(start.elapsed().as_secs() < 5);
}

#[test]
fn truncation_keeps_head_and_tail() {
    let s: String = (0..100).map(|i| format!("{i:03}")).collect(); // 300 chars
    let t = truncate_middle(&s, 100);
    assert!(t.starts_with("000"));
    assert!(t.ends_with("099"));
    assert!(t.contains("[… 200 chars omitted …]"));
    assert_eq!(truncate_middle("short", 100), "short");
}

#[test]
fn guard_blocks_privilege_escalation() {
    for cmd in [
        "sudo rm -rf /",
        "ls && sudo ls",
        "/usr/bin/sudo -n true",
        "echo x | su root",
        "doas sh",
        "pkexec bash",
        "run0 pacman -Syu",
        "osascript -e 'do shell script \"x\" with administrator privileges'",
        "SUDO ls",
    ] {
        assert!(check_command(cmd).is_err(), "should block: {cmd}");
    }
    for cmd in ["ls", "cargo build", "grep -r subdir .", "echo pseudo", "git log --format=%s"] {
        assert!(check_command(cmd).is_ok(), "should allow: {cmd}");
    }
}

#[test]
fn non_utf8_output_is_replaced_not_fatal() {
    let (r, _) = sh(r"printf 'ok\xff\xfe\n'", 10);
    assert_eq!(r.exit, Exit::Code(0));
    assert!(r.output.starts_with("ok"));
    assert!(r.output.contains('\u{FFFD}'));
}

#[test]
fn one_huge_line_is_truncated_for_the_model() {
    let (r, _) = sh("head -c 200000 /dev/zero | tr '\\0' x", 20);
    let text = r.for_model();
    assert!(text.chars().count() < 30_200, "{}", text.len());
    assert!(text.contains("chars omitted"));
    assert!(text.ends_with("[exit code: 0]"));
}

#[test]
fn runaway_output_keeps_memory_bounded() {
    let mut calls = 0;
    let dir = common::temp_dir();
    let r =
        run("yes | head -c 20000000", &dir, &make_shim_dir().unwrap(), 30, &mut |_| calls += 1, &mut || {
            false
        })
        .unwrap();
    assert_eq!(r.exit, Exit::Code(0));
    assert!(r.output.len() < 40_000, "{}", r.output.len());
    assert!(r.output.contains("chars omitted"));
    assert!(calls < 100_000, "{calls} UI line callbacks");
}

#[test]
fn escaped_background_writer_does_not_hang_the_tool() {
    // A process that leaves the process group keeps writing to the pipe for 4 s.
    let script = "python3 -c 'import os,time\nos.setsid()\nend=time.time()+4\nwhile time.time()<end: print(\"x\", flush=True)' & echo started";
    let start = Instant::now();
    let (r, _) = sh(script, 10);
    assert_eq!(r.exit, Exit::Code(0));
    assert!(start.elapsed().as_secs_f64() < 3.0, "{:?}", start.elapsed());
}

#[test]
fn shim_dirs_are_private_and_unique() {
    use std::os::unix::fs::PermissionsExt;
    let a = make_shim_dir().unwrap();
    let b = make_shim_dir().unwrap();
    assert_ne!(a, b);
    assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
}
