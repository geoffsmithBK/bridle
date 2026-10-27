//! Keys → UI changes and Commands for the agent.

use crate::events::Command;
use crate::ui::app::{App, Block};
use crate::ui::editor::Editor;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(app: &mut App, key: KeyEvent) -> Option<Command> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    if let Some(scroll) = app.wire_view.as_mut() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.wire_view = None,
            KeyCode::Char('j') | KeyCode::Down => *scroll = scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => *scroll = scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => *scroll = scroll.saturating_add(20),
            KeyCode::PageUp => *scroll = scroll.saturating_sub(20),
            _ => {}
        }
        return None;
    }

    if app.pending.is_some() {
        return approval_key(app, key, ctrl);
    }

    if app.ctrl_x {
        app.ctrl_x = false;
        match key.code {
            KeyCode::Char('o') => app.show_tools = !app.show_tools,
            KeyCode::Char('t') => app.show_thinking = !app.show_thinking,
            KeyCode::Char('w') => app.wire_view = Some(0),
            _ => {}
        }
        return None;
    }

    let is_ctrl_c = ctrl && key.code == KeyCode::Char('c');
    if !is_ctrl_c {
        app.quit_armed = false;
    }
    let half = (app.view_height / 2).max(1) as usize;
    let page = app.view_height.max(1) as usize;
    match key.code {
        KeyCode::Char('c') if ctrl => {
            if app.busy {
                return Some(Command::Interrupt);
            }
            if !app.input.is_empty() {
                app.input.clear();
            } else if app.quit_armed {
                app.quit = true;
                return Some(Command::Quit);
            } else {
                app.quit_armed = true;
            }
        }
        KeyCode::Char('x') if ctrl => app.ctrl_x = true,
        KeyCode::Char('v') if ctrl => app.scroll_back = app.scroll_back.saturating_sub(half),
        KeyCode::Char('v') if alt => app.scroll_back += half,
        KeyCode::PageDown => app.scroll_back = app.scroll_back.saturating_sub(page),
        KeyCode::PageUp => app.scroll_back += page,
        KeyCode::Enter if alt => app.input.insert_str("\n"),
        KeyCode::Char('j') if ctrl => app.input.insert_str("\n"),
        KeyCode::Enter => return submit(app),
        _ => {
            app.input.handle_key(key);
        }
    }
    None
}

/// Bracketed paste: insert text (newlines included) without submitting.
pub fn handle_paste(app: &mut App, text: &str) {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    match app.pending.as_mut() {
        // While typing a deny reason, paste into it (as one line).
        Some(p) => {
            if let Some(reason) = p.reason.as_mut() {
                reason.insert_str(&text.replace('\n', " "));
            }
        }
        None if app.wire_view.is_none() => app.input.insert_str(&text),
        None => {}
    }
}

fn approval_key(app: &mut App, key: KeyEvent, ctrl: bool) -> Option<Command> {
    let pending = app.pending.as_mut()?;
    if let Some(reason) = pending.reason.as_mut() {
        match key.code {
            KeyCode::Enter => {
                let text = reason.text().trim().to_string();
                app.pending = None;
                return Some(Command::Deny(if text.is_empty() { None } else { Some(text) }));
            }
            KeyCode::Esc => pending.reason = None,
            _ => {
                reason.handle_key(key);
            }
        }
        return None;
    }
    match key.code {
        KeyCode::Char('y') => {
            app.pending = None;
            Some(Command::Approve)
        }
        KeyCode::Char('n') => {
            app.pending = None;
            Some(Command::Deny(None))
        }
        KeyCode::Char('c') if ctrl => {
            app.pending = None;
            Some(Command::Interrupt)
        }
        KeyCode::Char('r') => {
            pending.reason = Some(Editor::new());
            None
        }
        KeyCode::Char('j') | KeyCode::Down => {
            pending.scroll = pending.scroll.saturating_add(1);
            None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            pending.scroll = pending.scroll.saturating_sub(1);
            None
        }
        KeyCode::PageDown => {
            pending.scroll = pending.scroll.saturating_add(10);
            None
        }
        KeyCode::PageUp => {
            pending.scroll = pending.scroll.saturating_sub(10);
            None
        }
        _ => None,
    }
}

fn submit(app: &mut App) -> Option<Command> {
    if app.busy {
        return None;
    }
    let text = app.input.text();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    app.scroll_back = 0;
    if let Some(cmd) = trimmed.strip_prefix('/') {
        app.input.clear();
        return match cmd {
            "new" => {
                app.blocks.clear();
                app.wire.clear();
                app.ctx_used = 0;
                app.busy = true;
                Some(Command::NewSession)
            }
            "quit" => {
                app.quit = true;
                Some(Command::Quit)
            }
            other => {
                app.blocks.push(Block::Info(format!("unknown command /{other} (try /new or /quit)")));
                None
            }
        };
    }
    if let Some(cmd) = trimmed.strip_prefix('!') {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            return None;
        }
        app.input.clear();
        app.busy = true;
        app.activity = format!("running !{cmd}");
        app.blocks.push(Block::Shell { command: cmd.clone(), live: vec![], output: None });
        return Some(Command::UserShell(cmd));
    }
    app.input.clear();
    app.busy = true;
    app.activity = "waiting for model".into();
    app.blocks.push(Block::User(text.clone()));
    Some(Command::Submit(text))
}
