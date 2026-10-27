use bridle::llm::client::{Streamed, read_stream};
use bridle::llm::stream::{Accumulator, Delta};
use std::io::Cursor;

const TOOLS: &str = include_str!("fixtures/sse_tools.txt");
const TEXT: &str = include_str!("fixtures/sse_text.txt");

fn run(fixture: &str) -> (Vec<Delta>, Streamed) {
    let mut deltas = vec![];
    let out = read_stream(Cursor::new(fixture), None, &mut |d| {
        deltas.push(d);
        true
    })
    .unwrap();
    (deltas, out)
}

#[test]
fn text_stream_separates_reasoning_and_content() {
    let (deltas, out) = run(TEXT);
    let Streamed::Complete(c) = out else { panic!("aborted") };
    assert_eq!(c.reasoning, "The user wants");
    assert_eq!(c.content, "Hi there friend.");
    assert!(c.tool_calls.is_empty());
    assert_eq!(c.finish_reason.as_deref(), Some("stop"));
    assert_eq!(c.usage.unwrap().total_tokens, 156);
    assert!(c.tokens_per_sec.unwrap() > 17.0);
    assert_eq!(deltas[0], Delta::Reasoning("The".into()));
    assert_eq!(deltas.last(), Some(&Delta::Text(".".into())));
}

#[test]
fn parallel_tool_calls_are_reassembled_by_index() {
    let (_, out) = run(TOOLS);
    let Streamed::Complete(c) = out else { panic!("aborted") };
    assert_eq!(c.finish_reason.as_deref(), Some("tool_calls"));
    assert_eq!(c.tool_calls.len(), 2);
    assert_eq!(c.tool_calls[0].id, "0Klv1d36wls2jda8jhstQMFNpEXKFNzF");
    assert_eq!(c.tool_calls[0].function.name, "read");
    assert_eq!(c.tool_calls[0].function.arguments, r#"{"path":"a.txt"}"#);
    assert_eq!(c.tool_calls[1].function.arguments, r#"{"path":"b.txt"}"#);
}

#[test]
fn returning_false_aborts_with_partial_content() {
    let mut n = 0;
    let out = read_stream(Cursor::new(TEXT), None, &mut |_| {
        n += 1;
        n < 4
    })
    .unwrap();
    let Streamed::Aborted(c) = out else { panic!("not aborted") };
    assert_eq!(c.reasoning, "The user wants");
    assert_eq!(c.content, "Hi");
}

#[test]
fn raw_lines_are_captured_when_requested() {
    let mut raw = vec![];
    read_stream(Cursor::new(TEXT), Some(&mut raw), &mut |_| true).unwrap();
    assert_eq!(raw.last().map(String::as_str), Some("data: [DONE]"));
    assert_eq!(raw.len(), 11);
}

#[test]
fn ignores_comments_and_blank_lines() {
    let mut acc = Accumulator::new();
    assert!(acc.push_line(": keep-alive").unwrap().is_empty());
    assert!(acc.push_line("").unwrap().is_empty());
    assert!(!acc.is_done());
    acc.push_line("data: [DONE]").unwrap();
    assert!(acc.is_done());
}

#[test]
fn garbage_chunk_is_an_error() {
    let mut acc = Accumulator::new();
    assert!(acc.push_line("data: {not json").is_err());
}

#[test]
fn tool_call_without_id_gets_synthetic_id() {
    let mut acc = Accumulator::new();
    acc.push_line(r#"data: {"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":"{}"}}]}}]}"#).unwrap();
    assert_eq!(acc.finish().tool_calls[0].id, "call_0");
}
