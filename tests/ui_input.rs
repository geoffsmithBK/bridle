use bridle::events::{AgentEvent, ApprovalRequest, Command};
use bridle::ui::app::{App, Block};
use bridle::ui::input::{handle_key, handle_paste};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

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
            live: vec!["file".into()],
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
    a.on_agent_event(AgentEvent::ApprovalNeeded(req.clone()));
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('y'))), Some(Command::Approve));
    assert!(a.pending.is_none());

    a.on_agent_event(AgentEvent::ApprovalNeeded(req.clone()));
    assert_eq!(handle_key(&mut a, key(KeyCode::Char('n'))), Some(Command::Deny(None)));

    a.on_agent_event(AgentEvent::ApprovalNeeded(req));
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
    a.on_agent_event(AgentEvent::ApprovalNeeded(req));
    handle_paste(&mut a, "y");
    assert!(a.pending.is_some());
    assert!(a.input.is_empty());
    handle_key(&mut a, key(KeyCode::Char('r')));
    handle_paste(&mut a, "too\nrisky");
    assert_eq!(handle_key(&mut a, key(KeyCode::Enter)), Some(Command::Deny(Some("too risky".into()))));
}
