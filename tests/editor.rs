use bridle::ui::editor::Editor;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}
fn meta(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Editor with `text`, cursor placed at the `|` marker.
fn ed(text: &str) -> Editor {
    let at = text.find('|').unwrap();
    let mut e = Editor::new();
    e.set_text(&text[..at]);
    let after = &text[at + 1..];
    e.insert_str(after);
    for _ in 0..after.chars().count() {
        e.handle_key(ctrl('b'));
    }
    e
}

/// Text with `|` at the cursor.
fn show(e: &Editor) -> String {
    let (row, col) = e.cursor_pos();
    let mut lines = e.lines();
    let line: Vec<char> = lines[row].chars().collect();
    lines[row] =
        format!("{}|{}", line[..col].iter().collect::<String>(), line[col..].iter().collect::<String>());
    lines.join("\n")
}

fn apply(text: &str, keys: &[KeyEvent]) -> String {
    let mut e = ed(text);
    for k in keys {
        e.handle_key(*k);
    }
    show(&e)
}

#[test]
fn typing_inserts_at_cursor() {
    assert_eq!(apply("ac|", &[ctrl('b'), key(KeyCode::Char('b'))]), "ab|c");
}

#[test]
fn char_motion() {
    assert_eq!(apply("ab|c", &[ctrl('f')]), "abc|");
    assert_eq!(apply("abc|", &[ctrl('f')]), "abc|");
    assert_eq!(apply("a|bc", &[ctrl('b'), ctrl('b')]), "|abc");
    assert_eq!(apply("a|bc", &[key(KeyCode::Right)]), "ab|c");
}

#[test]
fn word_motion_uses_whitespace() {
    assert_eq!(apply("|foo.bar  baz", &[meta('f')]), "foo.bar|  baz");
    assert_eq!(apply("|foo.bar  baz", &[meta('f'), meta('f')]), "foo.bar  baz|");
    assert_eq!(apply("foo bar|", &[meta('b')]), "foo |bar");
    assert_eq!(apply("foo ba|r", &[meta('b'), meta('b')]), "|foo bar");
}

#[test]
fn line_start_and_end_in_multiline() {
    assert_eq!(apply("one\ntw|o\nthree", &[ctrl('a')]), "one\n|two\nthree");
    assert_eq!(apply("one\nt|wo\nthree", &[ctrl('e')]), "one\ntwo|\nthree");
}

#[test]
fn previous_and_next_line_keep_column() {
    assert_eq!(apply("abcd\nxy|z", &[ctrl('p')]), "ab|cd\nxyz");
    assert_eq!(apply("abc|d\nxy", &[ctrl('n')]), "abcd\nxy|");
    assert_eq!(apply("ab|c", &[ctrl('p')]), "ab|c");
}

#[test]
fn deletion() {
    assert_eq!(apply("a|bc", &[ctrl('d')]), "a|c");
    assert_eq!(apply("ab|c", &[key(KeyCode::Backspace)]), "a|c");
    assert_eq!(apply("ab|c", &[ctrl('h')]), "a|c");
    assert_eq!(apply("ab\n|c", &[key(KeyCode::Backspace)]), "ab|c");
}

#[test]
fn kill_and_yank() {
    assert_eq!(apply("foo |bar baz", &[meta('d')]), "foo | baz");
    assert_eq!(apply("foo bar| baz", &[KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT)]), "foo | baz");
    assert_eq!(apply("ab|cd\nef", &[ctrl('k')]), "ab|\nef");
    assert_eq!(apply("ab|\nef", &[ctrl('k')]), "ab|ef");
    assert_eq!(apply("ab|cd", &[ctrl('u')]), "|cd");
    assert_eq!(apply("ab|cd", &[ctrl('k'), ctrl('a'), ctrl('y')]), "cd|ab");
}

#[test]
fn unknown_keys_are_not_handled() {
    let mut e = Editor::new();
    assert!(!e.handle_key(ctrl('x')));
    assert!(!e.handle_key(key(KeyCode::Enter)));
}

#[test]
fn multibyte_characters_are_single_steps() {
    assert_eq!(apply("é漢|字", &[ctrl('b')]), "é|漢字");
    assert_eq!(apply("naïve café|", &[meta('b')]), "naïve |café");
    assert_eq!(apply("👍|x", &[key(KeyCode::Backspace)]), "|x");
}
