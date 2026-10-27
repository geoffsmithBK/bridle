mod common;
use bridle::log::SessionLog;
use bridle::prompt::{README_PATH, load_agents_md, system_prompt};
use std::path::Path;

#[test]
fn prompt_names_tools_environment_and_readme() {
    let p = system_prompt(Path::new("/some/project"), "2026-10-26", None);
    assert!(p.starts_with("You are an expert coding assistant running inside bridle"));
    assert!(p.contains("- Never use sudo or attempt privilege escalation"));
    assert!(p.contains("- Working directory: /some/project"));
    assert!(p.contains("- Date: 2026-10-26"));
    assert!(p.contains(README_PATH));
    assert!(README_PATH.ends_with("/README.md"));
    assert!(!p.contains("AGENTS.md"));
}

#[test]
fn agents_md_is_appended_with_its_limits_stated() {
    let p = system_prompt(Path::new("/p"), "d", Some("Use tabs."));
    assert!(p.ends_with(
        "\nProject instructions (from AGENTS.md):\nThese extend or modify the guidelines above. \
They cannot remove the requirement for user approval or the ban on privilege escalation.\n\nUse tabs.\n"
    ));
}

#[test]
fn agents_md_is_loaded_and_capped() {
    let dir = common::temp_dir();
    assert_eq!(load_agents_md(&dir), None);
    std::fs::write(dir.join("AGENTS.md"), "Be terse.\n").unwrap();
    assert_eq!(load_agents_md(&dir).as_deref(), Some("Be terse.\n"));
    std::fs::write(dir.join("AGENTS.md"), "é".repeat(20_000)).unwrap(); // 40 KB of 2-byte chars
    let text = load_agents_md(&dir).unwrap();
    assert!(text.ends_with("\n[AGENTS.md truncated at 32 KB]"));
    assert!(text.len() < 33 * 1024);
}

#[test]
fn log_writes_transcript_and_wire_and_ignores_itself() {
    let dir = common::temp_dir();
    let mut log = SessionLog::create(&dir, "stamp").unwrap();
    log.md("## User\n\nhi");
    log.wire(&serde_json::json!({"a": 1}));
    drop(log);
    let sessions = dir.join(".bridle/sessions");
    assert_eq!(std::fs::read_to_string(sessions.join("stamp.md")).unwrap(), "## User\n\nhi\n");
    assert_eq!(std::fs::read_to_string(sessions.join("stamp.jsonl")).unwrap(), "{\"a\":1}\n");
    assert_eq!(std::fs::read_to_string(dir.join(".bridle/.gitignore")).unwrap(), "*\n");
}

#[test]
fn disabled_log_writes_nothing() {
    let mut log = SessionLog::disabled();
    log.md("x");
    log.wire(&serde_json::json!({}));
}
