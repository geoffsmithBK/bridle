use bridle::llm::types::{FunctionCall, Message, ToolCall};
use serde_json::json;

#[test]
fn messages_serialize_to_the_openai_format() {
    let call = ToolCall {
        id: "c1".into(),
        kind: "function".into(),
        function: FunctionCall { name: "read".into(), arguments: r#"{"path":"a"}"#.into() },
    };
    let msgs = vec![
        Message::System { content: "sys".into() },
        Message::User { content: "hi".into() },
        Message::Assistant { content: "".into(), tool_calls: vec![call] },
        Message::Tool { tool_call_id: "c1".into(), content: "data".into() },
        Message::Assistant { content: "done".into(), tool_calls: vec![] },
    ];
    assert_eq!(
        serde_json::to_value(&msgs).unwrap(),
        json!([
            {"role": "system", "content": "sys"},
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "read", "arguments": "{\"path\":\"a\"}"}}
            ]},
            {"role": "tool", "tool_call_id": "c1", "content": "data"},
            {"role": "assistant", "content": "done"}
        ])
    );
}
