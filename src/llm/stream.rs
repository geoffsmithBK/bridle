//! Assembles a streamed (SSE) chat completion into one `Completion`.
//!
//! The server sends many small `data: {...}` lines. Text and reasoning arrive
//! as fragments to append. Tool calls arrive keyed by `index`: the first
//! fragment for an index carries `id` and `function.name`, later fragments
//! carry pieces of `function.arguments` that must be concatenated. Usage
//! arrives in a final chunk with an empty `choices` array.

use crate::llm::types::{Completion, FunctionCall, ToolCall, Usage};
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;

/// A piece of the response worth showing to the user as it arrives.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    Reasoning(String),
    Text(String),
}

#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
pub struct Accumulator {
    content: String,
    reasoning: String,
    calls: BTreeMap<u64, PartialCall>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    tokens_per_sec: Option<f64>,
    done: bool,
}

impl Accumulator {
    pub fn new() -> Self {
        Self::default()
    }

    /// True once `data: [DONE]` has been seen.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Feed one line of the SSE stream. Returns the displayable pieces it contained.
    pub fn push_line(&mut self, line: &str) -> Result<Vec<Delta>> {
        let line = line.trim_end_matches(['\r', '\n']);
        let Some(data) = line.strip_prefix("data:") else {
            // Blank separators, ": comments", "event:" lines: nothing to do.
            return Ok(vec![]);
        };
        let data = data.trim_start();
        if data == "[DONE]" {
            self.done = true;
            return Ok(vec![]);
        }
        let chunk: Value = serde_json::from_str(data).with_context(|| format!("bad SSE chunk: {data}"))?;

        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = serde_json::from_value(usage.clone()).ok();
        }
        if let Some(tps) = chunk["timings"]["predicted_per_second"].as_f64() {
            self.tokens_per_sec = Some(tps);
        }

        let mut out = vec![];
        let Some(choice) = chunk["choices"].get(0) else {
            return Ok(out);
        };
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish_reason = Some(reason.to_string());
        }
        let delta = &choice["delta"];
        if let Some(r) = delta["reasoning_content"].as_str().filter(|s| !s.is_empty()) {
            self.reasoning.push_str(r);
            out.push(Delta::Reasoning(r.to_string()));
        }
        if let Some(t) = delta["content"].as_str().filter(|s| !s.is_empty()) {
            self.content.push_str(t);
            out.push(Delta::Text(t.to_string()));
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for (pos, call) in calls.iter().enumerate() {
                let index = call["index"].as_u64().unwrap_or(pos as u64);
                let entry = self.calls.entry(index).or_default();
                if let Some(id) = call["id"].as_str() {
                    entry.id = id.to_string();
                }
                if let Some(name) = call["function"]["name"].as_str() {
                    entry.name.push_str(name);
                }
                if let Some(args) = call["function"]["arguments"].as_str() {
                    entry.arguments.push_str(args);
                }
            }
        }
        Ok(out)
    }

    /// The assembled completion. Calls without an id get a synthetic one.
    pub fn finish(self) -> Completion {
        let tool_calls = self
            .calls
            .into_iter()
            .map(|(index, c)| ToolCall {
                id: if c.id.is_empty() { format!("call_{index}") } else { c.id },
                kind: "function".to_string(),
                function: FunctionCall { name: c.name, arguments: c.arguments },
            })
            .collect();
        Completion {
            content: self.content,
            reasoning: self.reasoning,
            tool_calls,
            finish_reason: self.finish_reason,
            usage: self.usage,
            tokens_per_sec: self.tokens_per_sec,
        }
    }
}
