use bridle::events::{AgentEvent, ApprovalRequest, Command};
use bridle::ui::app::{App, Block};
use bridle::ui::input::{handle_key, handle_paste};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn app() -> App {
    let mut a = App::new(PathBuf::from("/proj"));
    a.on_agent_event(AgentEvent::ServerStatus {
        online: true,
        model: "models/Qwen3.8-27B.gguf".into(),
        n_ctx: 262144,
    });
    a
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Open an approval as if it has been on screen for a second.
fn open_approval(a: &mut App, req: ApprovalRequest) {
    a.on_agent_event(AgentEvent::ApprovalNeeded(req));
    a.pending.as_mut().unwrap().opened = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
}

fn type_text(a: &mut App, s: &str) {
    for c in s.chars() {
        handle_key(a, key(KeyCode::Char(c)));
    }
}

#[test]
fn enter_submits_and_marks_busy() {
    let mut a = app();
    type_text(&mut a, "hello");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::Submit("hello".into())));
    assert!(a.busy);
    assert!(a.input.is_empty());
    assert_eq!(a.blocks, vec![Block::User("hello".into())]);
    // While busy, Enter does nothing and the text stays.
    type_text(&mut a, "next");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), None);
    assert_eq!(a.input.text(), "next");
    a.on_agent_event(AgentEvent::Idle);
    assert!(!a.busy);
}

#[test]
fn bang_runs_a_user_shell_command() {
    let mut a = app();
    type_text(&mut a, "!ls -la");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::UserShell("ls -la".into())));
    a.on_agent_event(AgentEvent::ToolOutput { id: "!".into(), line: "file".into() });
    a.on_agent_event(AgentEvent::ShellResult {
        command: "ls -la".into(),
        output: "file\n[exit code: 0]".into(),
    });
    assert_eq!(
        a.blocks,
        vec![Block::Shell {
            command: "ls -la".into(),
            live: vec![],
            output: Some("file\n[exit code: 0]".into())
        }]
    );
}

#[test]
fn slash_commands() {
    let mut a = app();
    a.blocks.push(Block::User("old".into()));
    type_text(&mut a, "/new");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::NewSession));
    assert!(a.blocks.is_empty());
    a.on_agent_event(AgentEvent::Idle);
    type_text(&mut a, "/bogus");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), None);
    assert!(matches!(&a.blocks[0], Block::Info(t) if t.contains("unknown command /bogus")));
    type_text(&mut a, "/quit");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::Quit));
    assert!(a.quit);
}

#[test]
fn multiline_input() {
    let mut a = app();
    type_text(&mut a, "a");
    handle_key(&mut a, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    type_text(&mut a, "b");
    handle_key(&mut a, ctrl('j'));
    type_text(&mut a, "c");
    assert_eq!(a.input.text(), "a\nb\nc");
}

#[test]
fn ctrl_c_interrupts_clears_then_quits() {
    let mut a = app();
    a.busy = true;
    assert_eq!(handle_key(&mut a, ctrl('c')), Some(Command::Interrupt));
    a.busy = false;
    type_text(&mut a, "draft");
    assert_eq!(handle_key(&mut a, ctrl('c')), None);
    assert!(a.input.is_empty());
    assert_eq!(handle_key(&mut a, ctrl('c')), None);
    assert!(a.quit_armed);
    assert_eq!(handle_key(&mut a, ctrl('c')), Some(Command::Quit));
}

#[test]
fn approval_keys() {
    let req = ApprovalRequest {
        title: "Approve bash?".into(),
        subject: "".into(),
        body: "ls".into(),
        outside_project: false,
        overwrite: false,
    };
    let mut a = app();
    open_approval(&mut a, req.clone());
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('y'))), Some(Command::Approve));
    assert!(a.pending.is_none());

    open_approval(&mut a, req.clone());
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('n'))), Some(Command::Deny(None)));

    open_approval(&mut a, req);
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('r'))), None);
    type_text(&mut a, "use cargo"); // 'y' and 'n' here are text, not decisions
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::Deny(Some("use cargo".into()))));
}

#[test]
fn ctrl_x_prefix_toggles() {
    let mut a = app();
    handle_key(&mut a, ctrl('x'));
    assert!(a.ctrl_x);
    handle_key(&mut a, key(KeyCode::Char('t')));
    assert!(a.show_thinking && !a.ctrl_x);
    handle_key(&mut a, ctrl('x'));
    handle_key(&mut a, key(KeyCode::Char('o')));
    assert!(a.show_tools);
    handle_key(&mut a, ctrl('x'));
    handle_key(&mut a, key(KeyCode::Char('w')));
    assert_eq!(a.wire_view, Some(0));
    handle_key(&mut a, key(KeyCode::Esc));
    assert_eq!(a.wire_view, None);
}

#[test]
fn restore_input_after_failed_request() {
    let mut a = app();
    type_text(&mut a, "hello");
    handle_key(&mut a, key(KeyCode::Enter));
    a.on_agent_event(AgentEvent::Error("server offline".into()));
    a.on_agent_event(AgentEvent::RestoreInput("hello".into()));
    a.on_agent_event(AgentEvent::Idle);
    assert_eq!(a.input.text(), "hello");
    assert!(!a.blocks.contains(&Block::User("hello".into())));
}

#[test]
fn paste_inserts_newlines_without_submitting() {
    let mut a = app();
    handle_paste(&mut a, "fn main() {\r\n    x();\r\n}");
    assert_eq!(a.input.text(), "fn main() {\n    x();\n}");
    assert!(!a.busy);
    assert!(a.blocks.is_empty());
}

#[test]
fn paste_during_approval_never_answers_it() {
    let mut a = app();
    let req = ApprovalRequest {
        title: "Approve bash?".into(),
        subject: "".into(),
        body: "ls".into(),
        outside_project: false,
        overwrite: false,
    };
    open_approval(&mut a, req);
    handle_paste(&mut a, "y");
    assert!(a.pending.is_some());
    assert!(a.input.is_empty());
    handle_key(&mut a, key(KeyCode::Char('r')));
    handle_paste(&mut a, "too\nrisky");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::Deny(Some("too risky".into()))));
}

#[test]
fn keys_typed_as_the_approval_opens_are_ignored() {
    let mut a = app();
    let req = ApprovalRequest {
        title: "Approve bash?".into(),
        subject: "".into(),
        body: "rm -rf build".into(),
        outside_project: false,
        overwrite: false,
    };
    a.on_agent_event(AgentEvent::ApprovalNeeded(req));
    for c in ['y', 'n', 'r'] {
        assert_eq!(handle_key(&mut a, key(KeyCode::Char(c))), None);
    }
    assert!(a.pending.as_ref().unwrap().reason.is_none());
    assert_eq!(handle_key(&mut a, ctrl('c')), Some(Command::Interrupt)); // escape still works
}

#[test]
fn second_ctrl_c_while_interrupting_quits() {
    let mut a = app();
    a.busy = true;
    assert_eq!(handle_key(&mut a, ctrl('c')), Some(Command::Interrupt));
    assert_eq!(handle_key(&mut a, ctrl('c')), Some(Command::Quit));
    assert!(a.quit);
}

#[test]
fn live_output_is_capped_and_cleared_when_the_result_arrives() {
    let mut a = app();
    a.on_agent_event(AgentEvent::ToolCallParsed {
        id: "1".into(),
        name: "bash".into(),
        summary: "bash  yes".into(),
    });
    for i in 0..5000 {
        a.on_agent_event(AgentEvent::ToolOutput { id: "1".into(), line: format!("{i}") });
    }
    let Block::Tool { live, .. } = &a.blocks[0] else { panic!() };
    assert!(live.len() <= 200, "{}", live.len());
    assert_eq!(live.last().map(String::as_str), Some("4999"));
    a.on_agent_event(AgentEvent::ToolResult {
        id: "1".into(),
        status: bridle::events::ToolStatus::Ok,
        content: "done".into(),
    });
    let Block::Tool { live, .. } = &a.blocks[0] else { panic!() };
    assert!(live.is_empty());
}

fn bash_req() -> ApprovalRequest {
    ApprovalRequest {
        title: "Approve bash?".into(),
        subject: "".into(),
        body: "ls".into(),
        outside_project: false,
        overwrite: false,
    }
}

#[test]
fn a_approves_all_but_not_during_the_grace_period() {
    let mut a = app();
    a.on_agent_event(AgentEvent::ApprovalNeeded(bash_req()));
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('a'))), None);
    a.pending.as_mut().unwrap().opened = Instant::now().checked_sub(Duration::from_secs(1)).unwrap();
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('a'))), Some(Command::ApproveAll));
    assert!(a.pending.is_none());
}

#[test]
fn ctrl_x_a_turns_auto_approve_off_only_when_on() {
    let mut a = app();
    handle_key(&mut a, ctrl('x'));
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('a'))), None); // can't turn it on from here
    a.on_agent_event(AgentEvent::AutoApprove(true));
    assert!(a.auto_approve);
    handle_key(&mut a, ctrl('x'));
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('a'))), Some(Command::AutoApproveOff));
    a.on_agent_event(AgentEvent::AutoApprove(false));
    assert!(!a.auto_approve);
}
