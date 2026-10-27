//! HTTP to llama-server: build requests, read the SSE stream.

use crate::llm::stream::{Accumulator, Delta};
use crate::llm::types::{Completion, Message};
use serde_json::{Value, json};
use std::io::BufRead;

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
    /// Could not connect, or the connection dropped.
    Offline(String),
    /// The server answered with a non-2xx status.
    Http { status: u16, body: String },
    /// The server's stream did not make sense.
    Protocol(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Offline(e) => write!(f, "server offline: {e}"),
            LlmError::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            LlmError::Protocol(e) => write!(f, "protocol error: {e}"),
        }
    }
}

/// How a stream ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Streamed {
    Complete(Completion),
    /// `on_delta` returned false; this is whatever arrived before that.
    Aborted(Completion),
}

/// Anything that can stream a chat completion. `LlamaClient` talks to the
/// real server; tests use a scripted fake.
pub trait Llm: Send {
    /// Stream one completion for `request`. `on_delta` is called for each
    /// displayable piece; returning false aborts the stream.
    fn stream(&self, request: &Value, on_delta: &mut dyn FnMut(Delta) -> bool) -> Result<Streamed, LlmError>;

    /// Ask the server which model it serves and its context size.
    fn probe(&self) -> Result<(String, u64), LlmError>;

    /// Raw SSE lines from the most recent stream, if they were kept.
    fn take_raw(&self) -> Vec<String> {
        vec![]
    }
}

/// The request body for one model call.
pub fn chat_request(model: &str, messages: &[Message], tools: &Value) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "tools": tools,
        "stream": true,
        "stream_options": { "include_usage": true },
    })
}

/// Read an SSE stream to the end (or until `on_delta` returns false).
/// If `raw` is given, every non-blank line is copied into it.
pub fn read_stream(
    reader: impl BufRead,
    mut raw: Option<&mut Vec<String>>,
    on_delta: &mut dyn FnMut(Delta) -> bool,
) -> Result<Streamed, LlmError> {
    let mut acc = Accumulator::new();
    for line in reader.lines() {
        let line = line.map_err(|e| LlmError::Offline(e.to_string()))?;
        if line.is_empty() {
            continue;
        }
        if let Some(raw) = raw.as_deref_mut() {
            raw.push(line.clone());
        }
        let deltas = acc.push_line(&line).map_err(|e| LlmError::Protocol(e.to_string()))?;
        for d in deltas {
            if !on_delta(d) {
                return Ok(Streamed::Aborted(acc.finish()));
            }
        }
        if acc.is_done() {
            break;
        }
    }
    Ok(Streamed::Complete(acc.finish()))
}

pub struct LlamaClient {
    agent: ureq::Agent,
    base_url: String,
    /// Keep raw SSE lines of each stream for the wire log (`--wire-raw`).
    keep_raw: bool,
    last_raw: std::sync::Mutex<Vec<String>>,
}

impl LlamaClient {
    pub fn new(base_url: &str, keep_raw: bool) -> Self {
        // Non-2xx responses are returned as responses, so we can show their body.
        let agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
        Self {
            agent,
            base_url: base_url.trim_end_matches('/').to_string(),
            keep_raw,
            last_raw: Default::default(),
        }
    }

    fn fetch_models(&self) -> Result<(String, u64), LlmError> {
        let resp = self
            .agent
            .get(format!("{}/v1/models", self.base_url))
            .call()
            .map_err(|e| LlmError::Offline(e.to_string()))?;
        let body = resp.into_body().read_to_string().map_err(|e| LlmError::Offline(e.to_string()))?;
        let v: Value = serde_json::from_str(&body).map_err(|e| LlmError::Protocol(e.to_string()))?;
        let model =
            v["data"][0]["id"].as_str().ok_or_else(|| LlmError::Protocol("no models listed".into()))?;
        let n_ctx = v["data"][0]["meta"]["n_ctx"].as_u64().unwrap_or(0);
        Ok((model.to_string(), n_ctx))
    }
}

impl Llm for LlamaClient {
    fn stream(&self, request: &Value, on_delta: &mut dyn FnMut(Delta) -> bool) -> Result<Streamed, LlmError> {
        let resp = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .send(request.to_string())
            .map_err(|e| LlmError::Offline(e.to_string()))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let body = resp.into_body().read_to_string().unwrap_or_default();
            return Err(LlmError::Http { status, body });
        }
        let reader = std::io::BufReader::new(resp.into_body().into_reader());
        let mut raw = Vec::new();
        let result = read_stream(reader, self.keep_raw.then_some(&mut raw), on_delta);
        *self.last_raw.lock().unwrap() = raw;
        result
    }

    fn probe(&self) -> Result<(String, u64), LlmError> {
        self.fetch_models()
    }

    fn take_raw(&self) -> Vec<String> {
        std::mem::take(&mut *self.last_raw.lock().unwrap())
    }
}
