//! Talks to a real llama-server. Run with: cargo test --test live -- --ignored

use bridle::llm::client::{LlamaClient, Llm, Streamed, chat_request};
use bridle::llm::types::Message;
use serde_json::json;

const URL: &str = "http://127.0.0.1:9931";

#[test]
#[ignore]
fn live_server_streams_a_tool_call() {
    let client = LlamaClient::new(URL, true);
    let (model, n_ctx) = client.probe().expect("server running?");
    assert!(n_ctx > 0);
    let messages = vec![
        Message::System { content: "You are a coding assistant. Use tools.".into() },
        Message::User { content: "Read the file Cargo.toml. Think briefly.".into() },
    ];
    let tools = json!([{"type": "function", "function": {
        "name": "read",
        "description": "Read a text file",
        "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}
    }}]);
    let request = chat_request(&model, &messages, &tools);
    let Streamed::Complete(c) = client.stream(&request, &mut |_| true).unwrap() else { panic!() };
    assert_eq!(c.tool_calls.len(), 1, "{c:?}");
    assert_eq!(c.tool_calls[0].function.name, "read");
    assert!(c.tool_calls[0].function.arguments.contains("Cargo.toml"));
    assert!(!client.take_raw().is_empty());
}

#[test]
#[ignore]
fn live_server_reports_http_errors_with_body() {
    let client = LlamaClient::new(URL, false);
    let err = client.stream(&serde_json::json!({"messages": "nope"}), &mut |_| true).unwrap_err();
    assert!(err.to_string().starts_with("HTTP 400"), "{err}");
}
