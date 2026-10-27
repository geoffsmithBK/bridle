use bridle::events::{AgentEvent, ApprovalRequest, ToolStatus};
use bridle::llm::types::Usage;
use bridle::ui::app::{App, Block};
use bridle::ui::{input::handle_key, render::draw};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
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

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Render to a w×h text grid.
fn screen(a: &mut App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw(f, a)).unwrap();
    let buf = t.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn renders_conversation_and_status() {
    let mut a = app();
    a.blocks.push(Block::User("add a flag".into()));
    a.on_agent_event(AgentEvent::ReasoningDelta("let me look".into()));
    a.on_agent_event(AgentEvent::TextDelta("Looking.".into()));
    a.on_agent_event(AgentEvent::ToolCallParsed {
        id: "1".into(),
        name: "read".into(),
        summary: "read  src/main.rs".into(),
    });
    a.on_agent_event(AgentEvent::ToolResult {
        id: "1".into(),
        status: ToolStatus::Ok,
        content: "a\nb\n".into(),
    });
    a.on_agent_event(AgentEvent::TurnDone {
        usage: Some(Usage { prompt_tokens: 0, completion_tokens: 0, total_tokens: 14200 }),
        tokens_per_sec: Some(31.0),
        interrupted: false,
    });
    let s = screen(&mut a, 70, 16);
    assert!(s.contains("› add a flag"), "{s}");
    assert!(s.contains("▸ thought for 0s"), "{s}");
    assert!(!s.contains("let me look"), "{s}");
    assert!(s.contains("Looking."), "{s}");
    assert!(s.contains("✓ read  src/main.rs  (2 lines)"), "{s}");
    assert!(s.contains("Qwen3.8-27B · ctx 14.2k/262.1k · 31 tok/s"), "{s}");
}

#[test]
fn renders_approval_overlay_with_diff_and_flags() {
    let mut a = app();
    a.on_agent_event(AgentEvent::ApprovalNeeded(ApprovalRequest {
        title: "Approve write?".into(),
        subject: "/etc/x".into(),
        body: "--- x\n+++ x\n@@ -1 +1 @@\n-old\n+new\n".into(),
        outside_project: true,
        overwrite: true,
    }));
    let s = screen(&mut a, 80, 20);
    assert!(s.contains("Approve write?"), "{s}");
    assert!(s.contains("OVERWRITE") && s.contains("OUTSIDE PROJECT"), "{s}");
    assert!(s.contains("+new") && s.contains("-old"), "{s}");
    assert!(s.contains("[y] approve   [n] deny   [r] deny with reason"), "{s}");
}

#[test]
fn follows_output_and_scrolls_by_half_screens() {
    let mut a = app();
    for i in 0..100 {
        a.blocks.push(Block::Info(format!("line {i}")));
    }
    let s = screen(&mut a, 40, 22); // conversation pane = 22 - 2 input - 1 status = 19 rows
    assert!(s.contains("line 99"), "{s}");
    handle_key(&mut a, KeyEvent::new(KeyCode::Char('v'), KeyModifiers::ALT));
    assert_eq!(a.scroll_back, 9);
    let s = screen(&mut a, 40, 22);
    assert!(!s.contains("line 99") && s.contains("line 94"), "{s}");
    handle_key(&mut a, ctrl('v'));
    assert_eq!(a.scroll_back, 0);
}

#[test]
fn long_lines_wrap_without_losing_the_bottom() {
    let mut a = app();
    a.blocks.push(Block::Assistant("word ".repeat(200)));
    a.blocks.push(Block::Info("THE END".into()));
    let s = screen(&mut a, 30, 12);
    assert!(s.contains("THE END"), "{s}");
}

#[test]
fn thinking_is_hidden_while_streaming_and_after() {
    let mut a = app();
    a.busy = true;
    a.on_agent_event(AgentEvent::ReasoningDelta("secret plans unfold".into()));
    let s = screen(&mut a, 60, 10);
    assert!(s.contains("▸ thinking… 0s"), "{s}");
    assert!(!s.contains("secret"), "{s}");
    a.on_agent_event(AgentEvent::TextDelta("Answer.".into()));
    a.on_agent_event(AgentEvent::Idle);
    let s = screen(&mut a, 60, 10);
    assert!(s.contains("▸ thought for 0s") && !s.contains("secret"), "{s}");
    a.show_thinking = true;
    let s = screen(&mut a, 60, 10);
    assert!(s.contains("▾ thought for 0s") && s.contains("secret plans unfold"), "{s}");
}

#[test]
fn thinking_timer_counts_seconds() {
    let mut a = app();
    let started = Instant::now().checked_sub(Duration::from_secs(7)).unwrap();
    a.blocks.push(Block::Thinking { text: "x".into(), started, elapsed: None });
    assert!(screen(&mut a, 60, 8).contains("▸ thinking… 7s"));
    a.blocks[0] = Block::Thinking { text: "x".into(), started, elapsed: Some(Duration::from_secs(12)) };
    assert!(screen(&mut a, 60, 8).contains("▸ thought for 12s"));
}

#[test]
fn info_events_show_as_dim_lines() {
    let mut a = app();
    a.on_agent_event(AgentEvent::Info("loaded AGENTS.md (2 lines)".into()));
    assert!(screen(&mut a, 60, 8).contains("loaded AGENTS.md (2 lines)"));
}
