mod common;
use bridle::config::Resume;
use bridle::llm::types::{FunctionCall, Message, ToolCall};
use bridle::log::SessionLog;
use bridle::session::{self, EXITED, SavedSession, VERSION};

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: "function".into(),
        function: FunctionCall { name: "read".into(), arguments: "{}".into() },
    }
}

#[test]
fn save_and_load_round_trip_without_leaving_temp_files() {
    let dir = common::temp_dir();
    let path = dir.join("s.session.json");
    let saved = SavedSession {
        version: VERSION,
        cwd: dir.clone(),
        messages: vec![
            Message::User { content: "hi".into() },
            Message::Assistant { content: "yo".into(), tool_calls: vec![] },
        ],
    };
    session::save(&path, &saved).unwrap();
    assert_eq!(session::load(&path).unwrap(), saved);
    let names: Vec<String> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
    assert_eq!(names, ["s.session.json"]);
}

#[test]
fn load_rejects_unknown_versions_and_garbage() {
    let dir = common::temp_dir();
    let path = dir.join("s.session.json");
    std::fs::write(&path, r#"{"version":99,"cwd":"/","messages":[]}"#).unwrap();
    assert!(session::load(&path).unwrap_err().contains("version 99"));
    std::fs::write(&path, "{not json").unwrap();
    assert!(session::load(&path).is_err());
}

#[test]
fn repair_answers_calls_left_unanswered_by_a_crash() {
    let mut m = vec![
        Message::User { content: "go".into() },
        Message::Assistant { content: "".into(), tool_calls: vec![call("a"), call("b"), call("c")] },
        Message::Tool { tool_call_id: "a".into(), content: "done".into() },
    ];
    assert_eq!(session::repair(&mut m), 2);
    assert_eq!(m[3], Message::Tool { tool_call_id: "b".into(), content: EXITED.into() });
    assert_eq!(m[4], Message::Tool { tool_call_id: "c".into(), content: EXITED.into() });
    assert_eq!(session::repair(&mut m), 0);
}

#[test]
fn resolve_finds_latest_or_named_session() {
    let root = common::temp_dir();
    assert!(session::resolve(&root, &Resume::Latest).unwrap_err().contains("no saved sessions"));
    for stamp in ["2026-10-26T09-00-00", "2026-10-27T10-00-00", "2026-10-26T23-00-00"] {
        let mut log = SessionLog::create(&root, stamp).unwrap();
        log.save_session(&root, &[]);
    }
    assert_eq!(session::list(&root), ["2026-10-26T09-00-00", "2026-10-26T23-00-00", "2026-10-27T10-00-00"]);
    assert_eq!(session::resolve(&root, &Resume::Latest).unwrap(), "2026-10-27T10-00-00");
    assert_eq!(
        session::resolve(&root, &Resume::Stamp("2026-10-26T09-00-00".into())).unwrap(),
        "2026-10-26T09-00-00"
    );
    let err = session::resolve(&root, &Resume::Stamp("nope".into())).unwrap_err();
    assert!(err.contains("no saved session nope") && err.contains("2026-10-27T10-00-00"), "{err}");
}

#[test]
fn a_second_session_in_the_same_second_gets_its_own_files() {
    let root = common::temp_dir();
    let mut a = SessionLog::create(&root, "2026-10-26T09-00-00").unwrap();
    a.md("first");
    let b = SessionLog::create(&root, "2026-10-26T09-00-00").unwrap();
    assert_eq!(a.stamp.as_deref(), Some("2026-10-26T09-00-00"));
    assert_eq!(b.stamp.as_deref(), Some("2026-10-26T09-00-00-2"));
    let md = std::fs::read_to_string(root.join(".bridle/sessions/2026-10-26T09-00-00.md")).unwrap();
    assert_eq!(md, "first\n");
}
