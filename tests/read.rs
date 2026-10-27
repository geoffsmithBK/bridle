mod common;
use bridle::tools::read::read_file;

#[test]
fn numbers_lines() {
    let dir = common::temp_dir();
    let p = dir.join("a.txt");
    std::fs::write(&p, "one\ntwo\n").unwrap();
    assert_eq!(read_file(&p, None, None).unwrap(), "     1│ one\n     2│ two\n");
}

#[test]
fn pages_with_offset_and_limit_and_says_how_to_continue() {
    let dir = common::temp_dir();
    let p = dir.join("a.txt");
    let text: String = (1..=10).map(|i| format!("l{i}\n")).collect();
    std::fs::write(&p, text).unwrap();
    let out = read_file(&p, Some(3), Some(2)).unwrap();
    assert_eq!(
        out,
        "     3│ l3\n     4│ l4\n[lines 3–4 of 10 — call read again with offset=5 to continue]\n"
    );
}

#[test]
fn caps_at_2000_lines() {
    let dir = common::temp_dir();
    let p = dir.join("big.txt");
    let text: String = (1..=2500).map(|i| format!("{i}\n")).collect();
    std::fs::write(&p, text).unwrap();
    let out = read_file(&p, None, Some(9999)).unwrap();
    assert!(out.ends_with("[lines 1–2000 of 2500 — call read again with offset=2001 to continue]\n"));
}

#[test]
fn truncates_very_long_lines() {
    let dir = common::temp_dir();
    let p = dir.join("long.txt");
    std::fs::write(&p, "x".repeat(2500)).unwrap();
    let out = read_file(&p, None, None).unwrap();
    assert!(out.contains("… [line truncated]"));
    assert!(out.len() < 2100);
}

#[test]
fn errors_are_helpful() {
    let dir = common::temp_dir();
    assert!(read_file(&dir, None, None).unwrap_err().contains("is a directory"));
    assert!(read_file(&dir.join("nope"), None, None).unwrap_err().contains("cannot read"));
    let bin = dir.join("bin");
    std::fs::write(&bin, [0u8, 1, 2]).unwrap();
    assert!(read_file(&bin, None, None).unwrap_err().contains("binary"));
    let short = dir.join("short");
    std::fs::write(&short, "a\n").unwrap();
    assert!(read_file(&short, Some(5), None).unwrap_err().contains("past the end"));
}

#[test]
fn empty_file() {
    let dir = common::temp_dir();
    let p = dir.join("e");
    std::fs::write(&p, "").unwrap();
    assert!(read_file(&p, None, None).unwrap().contains("is empty"));
}
