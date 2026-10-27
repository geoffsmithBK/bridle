//! A small multi-line text editor with emacs/readline keys.
//!
//! The text is a flat `Vec<char>` with newlines in it, and the cursor is an
//! index into it. Lines are computed on demand; that keeps every motion a
//! simple scan.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default, Clone)]
pub struct Editor {
    chars: Vec<char>,
    cursor: usize,
    /// Single-slot kill buffer for Ctrl-Y.
    kill: String,
}

impl Editor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Clear the text (the kill buffer survives).
    pub fn clear(&mut self) {
        self.chars.clear();
        self.cursor = 0;
    }

    /// Replace the text; cursor goes to the end.
    pub fn set_text(&mut self, text: &str) {
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
    }

    /// (row, column) of the cursor.
    pub fn cursor_pos(&self) -> (usize, usize) {
        let row = self.chars[..self.cursor].iter().filter(|&&c| c == '\n').count();
        (row, self.cursor - self.line_start())
    }

    pub fn lines(&self) -> Vec<String> {
        self.text().split('\n').map(str::to_string).collect()
    }

    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.chars.insert(self.cursor, c);
            self.cursor += 1;
        }
    }

    fn line_start(&self) -> usize {
        self.chars[..self.cursor].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.chars[self.cursor..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(self.chars.len(), |i| self.cursor + i)
    }

    fn word_forward_pos(&self) -> usize {
        let mut i = self.cursor;
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        while i < self.chars.len() && !self.chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    fn word_backward_pos(&self) -> usize {
        let mut i = self.cursor;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        i
    }

    /// Remove chars in `from..to`, saving them in the kill buffer.
    fn kill_range(&mut self, from: usize, to: usize) {
        if from < to {
            self.kill = self.chars.drain(from..to).collect();
            self.cursor = from;
        }
    }

    /// Move to the same column on another line (clamped to its length).
    fn move_line(&mut self, down: bool) {
        let col = self.cursor - self.line_start();
        let target_start = if down {
            let end = self.line_end();
            if end == self.chars.len() {
                return;
            }
            end + 1
        } else {
            let start = self.line_start();
            if start == 0 {
                return;
            }
            self.chars[..start - 1].iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1)
        };
        self.cursor = target_start;
        let end = self.line_end();
        self.cursor = (target_start + col).min(end);
    }

    /// Apply an editing key. Returns false if the key isn't an editing key.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char('f') if ctrl => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.chars.len()),
            KeyCode::Char('b') if ctrl => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('f') if alt => self.cursor = self.word_forward_pos(),
            KeyCode::Char('b') if alt => self.cursor = self.word_backward_pos(),
            KeyCode::Char('a') if ctrl => self.cursor = self.line_start(),
            KeyCode::Home => self.cursor = self.line_start(),
            KeyCode::Char('e') if ctrl => self.cursor = self.line_end(),
            KeyCode::End => self.cursor = self.line_end(),
            KeyCode::Char('p') if ctrl => self.move_line(false),
            KeyCode::Up => self.move_line(false),
            KeyCode::Char('n') if ctrl => self.move_line(true),
            KeyCode::Down => self.move_line(true),
            KeyCode::Char('d') if ctrl => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.chars.len() {
                    self.chars.remove(self.cursor);
                }
            }
            KeyCode::Backspace if alt => self.kill_range(self.word_backward_pos(), self.cursor),
            KeyCode::Char('h') if ctrl => self.backspace(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char('d') if alt => self.kill_range(self.cursor, self.word_forward_pos()),
            KeyCode::Char('k') if ctrl => {
                let end = self.line_end();
                if end == self.cursor && end < self.chars.len() {
                    self.kill_range(self.cursor, end + 1); // at end of line: join
                } else {
                    self.kill_range(self.cursor, end);
                }
            }
            KeyCode::Char('u') if ctrl => self.kill_range(self.line_start(), self.cursor),
            KeyCode::Char('y') if ctrl => {
                let kill = self.kill.clone();
                self.insert_str(&kill);
            }
            KeyCode::Char(c) if !ctrl && !alt => self.insert_str(&c.to_string()),
            _ => return false,
        }
        true
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
        }
    }
}
