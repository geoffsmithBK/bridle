//! Saved sessions: the conversation history on disk, so a session can be
//! resumed after bridle restarts (`--continue`, `--resume STAMP`).
//!
//! `<stamp>.session.json` sits next to the session's transcript and wire
//! log. It holds the history without the system prompt, which is rebuilt on
//! resume (fresh date, current AGENTS.md). Nothing about approval is saved:
//! a resumed session always starts with auto-approve off.

use crate::config::Resume;
use crate::llm::types::Message;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;

/// The result given to tool calls that never got one because bridle exited.
pub const EXITED: &str = "Cancelled (bridle exited).";

const SUFFIX: &str = ".session.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSession {
    pub version: u32,
    /// Session working directory (from `!cd`).
    pub cwd: PathBuf,
    /// History after the system prompt.
    pub messages: Vec<Message>,
}

pub fn sessions_dir(project_root: &Path) -> PathBuf {
    project_root.join(".bridle").join("sessions")
}

pub fn session_path(project_root: &Path, stamp: &str) -> PathBuf {
    sessions_dir(project_root).join(format!("{stamp}{SUFFIX}"))
}

/// Write atomically: a temp file, then rename over the old one, so a crash
/// mid-write never leaves a truncated session.
pub fn save(path: &Path, session: &SavedSession) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(session)?)?;
    std::fs::rename(&tmp, path)
}

pub fn load(path: &Path) -> Result<SavedSession, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let version = value["version"].as_u64().unwrap_or(0);
    if version != VERSION as u64 {
        return Err(format!("unsupported session file version {version}"));
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Give every tool call without a result the `EXITED` result, placed right
/// after the results it does have. Returns how many were added.
pub fn repair(messages: &mut Vec<Message>) -> usize {
    let mut added = 0;
    let mut i = 0;
    while i < messages.len() {
        if let Message::Assistant { tool_calls, .. } = &messages[i] {
            let ids: Vec<String> = tool_calls.iter().map(|c| c.id.clone()).collect();
            let mut end = i + 1;
            let mut answered = vec![];
            while let Some(Message::Tool { tool_call_id, .. }) = messages.get(end) {
                answered.push(tool_call_id.clone());
                end += 1;
            }
            for id in ids.into_iter().filter(|id| !answered.contains(id)) {
                messages.insert(end, Message::Tool { tool_call_id: id, content: EXITED.to_string() });
                end += 1;
                added += 1;
            }
            i = end;
        } else {
            i += 1;
        }
    }
    added
}

/// Stamps of the saved sessions in this project, oldest first.
pub fn list(project_root: &Path) -> Vec<String> {
    let mut stamps: Vec<String> = std::fs::read_dir(sessions_dir(project_root))
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().into_string().ok()?.strip_suffix(SUFFIX).map(str::to_string))
        .collect();
    stamps.sort();
    stamps
}

/// Turn `--continue` / `--resume STAMP` into a stamp that exists.
pub fn resolve(project_root: &Path, resume: &Resume) -> Result<String, String> {
    let stamps = list(project_root);
    match resume {
        Resume::Latest => stamps
            .last()
            .cloned()
            .ok_or_else(|| format!("no saved sessions in {}", sessions_dir(project_root).display())),
        Resume::Stamp(stamp) if stamps.contains(stamp) => Ok(stamp.clone()),
        Resume::Stamp(stamp) => {
            let recent: Vec<&str> = stamps.iter().rev().take(10).map(String::as_str).collect();
            Err(format!(
                "no saved session {stamp} in {}; recent sessions:\n  {}",
                sessions_dir(project_root).display(),
                if recent.is_empty() { "(none)".to_string() } else { recent.join("\n  ") }
            ))
        }
    }
}
