mod common;
use bridle::tools::edit::plan_edit;
use bridle::tools::write::{apply_write, diff_stats, unified_diff};

#[test]
fn replaces_unique_match() {
    assert_eq!(plan_edit("f", "a\nb\nc\n", "b\n", "B\nB2\n").unwrap(), "a\nB\nB2\nc\n");
}

#[test]
fn no_match_says_to_reread() {
    let err = plan_edit("f.rs", "a\n", "zzz", "y").unwrap_err();
    assert!(err.starts_with("old_text not found in f.rs."), "{err}");
}

#[test]
fn multiple_matches_lists_lines() {
    let err = plan_edit("f", "x\ny\nx\ny\nx\n", "x", "z").unwrap_err();
    assert_eq!(
        err,
        "old_text matches 3 times (lines 1, 3, 5). Include more surrounding lines to make it unique."
    );
}

#[test]
fn empty_old_text_is_rejected() {
    assert!(plan_edit("f", "abc", "", "x").is_err());
}

#[test]
fn diff_and_stats() {
    let d = unified_diff("f.txt", "a\nb\n", "a\nc\nd\n");
    assert!(d.starts_with("--- f.txt\n+++ f.txt\n@@"), "{d}");
    assert!(d.contains("-b\n") && d.contains("+c\n"));
    assert_eq!(diff_stats("a\nb\n", "a\nc\nd\n"), (1, 2));
}

#[test]
fn apply_write_creates_new_file_and_parents() {
    let dir = common::temp_dir();
    let p = dir.join("x/y/new.txt");
    apply_write(&p, None, "hi\n").unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "hi\n");
}

#[test]
fn apply_write_refuses_when_file_changed() {
    let dir = common::temp_dir();
    let p = dir.join("f.txt");
    std::fs::write(&p, "changed\n").unwrap();
    let err = apply_write(&p, Some("original\n"), "new\n").unwrap_err();
    assert!(err.contains("changed since the change was proposed"));
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "changed\n");
}

#[test]
fn apply_write_refuses_when_new_file_appeared() {
    let dir = common::temp_dir();
    let p = dir.join("f.txt");
    std::fs::write(&p, "someone else\n").unwrap();
    assert!(apply_write(&p, None, "mine\n").is_err());
}
