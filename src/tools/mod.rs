//! The four tools: their definitions for the model, approval previews, and execution.
//!
//! A tool call goes through two steps:
//! 1. `prepare` parses arguments and works out exactly what will happen
//!    (for edits, the new file content). It decides whether approval is needed.
//! 2. `execute` does it. What executes is exactly what was prepared.

pub mod bash;
pub mod edit;
pub mod guard;
pub mod paths;
pub mod read;
pub mod write;

use crate::events::{ApprovalRequest, ToolStatus};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Where tools run.
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Session working directory (changed only by the user's `!cd`).
    pub cwd: PathBuf,
    /// The directory bridle was launched in.
    pub project_root: PathBuf,
    /// Directory holding the failing `sudo` shim.
    pub shim_dir: PathBuf,
}

/// A fully worked-out action, ready to run.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Read {
        path: PathBuf,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    Bash {
        command: String,
        timeout: u64,
    },
    /// Used by both edit and write.
    WriteFile {
        path: PathBuf,
        expected_old: Option<String>,
        new_content: String,
        edit: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Prepared {
    Ready(Action),
    NeedsApproval(ApprovalRequest, Action),
    /// The message for the model (without the "Error: " prefix).
    Failed(String),
}

/// Tool definitions sent to the model with every request.
pub fn definitions() -> Value {
    json!([
        {"type": "function", "function": {
            "name": "read",
            "description": "Read a text file. Returns numbered lines, at most 2000 per call; use offset to page through longer files.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string", "description": "File path, relative to the working directory or absolute"},
                "offset": {"type": "integer", "description": "1-based line number to start at"},
                "limit": {"type": "integer", "description": "Maximum number of lines to return"}
            }, "required": ["path"]}
        }},
        {"type": "function", "function": {
            "name": "bash",
            "description": "Run a bash command in the working directory and return its combined stdout/stderr and exit code. Each call runs in a fresh shell: cd does not persist. No stdin; interactive programs will not work. Requires user approval.",
            "parameters": {"type": "object", "properties": {
                "command": {"type": "string", "description": "The command to run"},
                "timeout": {"type": "integer", "description": "Seconds before the command is killed (default 120, max 600)"}
            }, "required": ["command"]}
        }},
        {"type": "function", "function": {
            "name": "edit",
            "description": "Replace one exact occurrence of old_text with new_text in a file. old_text must match the file exactly (including whitespace) and must occur exactly once. Requires user approval.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string"},
                "old_text": {"type": "string", "description": "Exact text to replace"},
                "new_text": {"type": "string", "description": "Replacement text"}
            }, "required": ["path", "old_text", "new_text"]}
        }},
        {"type": "function", "function": {
            "name": "write",
            "description": "Create a file, or overwrite it completely, with the given content. Creates parent directories. Requires user approval.",
            "parameters": {"type": "object", "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            }, "required": ["path", "content"]}
        }}
    ])
}

/// One-line description of a call for the UI, e.g. "bash  cargo build".
pub fn summarize(name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    let detail = match name {
        "bash" => args["command"].as_str().unwrap_or("").lines().next().unwrap_or("").to_string(),
        _ => args["path"].as_str().unwrap_or("").to_string(),
    };
    format!("{name}  {detail}")
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args[key].as_str().ok_or_else(|| format!("missing string argument '{key}'"))
}

/// Parse a tool call and work out what it will do.
pub fn prepare(name: &str, arguments: &str, ctx: &ToolContext) -> Prepared {
    match prepare_inner(name, arguments, ctx) {
        Ok(p) => p,
        Err(msg) => Prepared::Failed(msg),
    }
}

fn prepare_inner(name: &str, arguments: &str, ctx: &ToolContext) -> Result<Prepared, String> {
    let args: Value = serde_json::from_str(arguments)
        .map_err(|e| format!("arguments were not valid JSON ({e}). Raw arguments: {arguments}"))?;
    match name {
        "read" => {
            let path = paths::resolve(&ctx.cwd, str_arg(&args, "path")?);
            let offset = args["offset"].as_u64().map(|n| n as usize);
            let limit = args["limit"].as_u64().map(|n| n as usize);
            Ok(Prepared::Ready(Action::Read { path, offset, limit }))
        }
        "bash" => {
            let command = str_arg(&args, "command")?.to_string();
            guard::check_command(&command)?;
            let timeout =
                args["timeout"].as_u64().unwrap_or(bash::DEFAULT_TIMEOUT).clamp(1, bash::MAX_TIMEOUT);
            let approval = ApprovalRequest {
                title: "Approve bash?".into(),
                subject: String::new(),
                body: command.clone(),
                outside_project: false,
                overwrite: false,
            };
            Ok(Prepared::NeedsApproval(approval, Action::Bash { command, timeout }))
        }
        "edit" => {
            let raw = str_arg(&args, "path")?;
            let path = paths::resolve(&ctx.cwd, raw);
            let old = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {raw}: {e}"))?;
            let new = edit::plan_edit(raw, &old, str_arg(&args, "old_text")?, str_arg(&args, "new_text")?)?;
            let approval = ApprovalRequest {
                title: "Approve edit?".into(),
                subject: path.display().to_string(),
                body: write::unified_diff(raw, &old, &new),
                outside_project: paths::is_outside(&ctx.project_root, &path),
                overwrite: false,
            };
            Ok(Prepared::NeedsApproval(
                approval,
                Action::WriteFile { path, expected_old: Some(old), new_content: new, edit: true },
            ))
        }
        "write" => {
            let raw = str_arg(&args, "path")?;
            let path = paths::resolve(&ctx.cwd, raw);
            let content = str_arg(&args, "content")?.to_string();
            if path.is_dir() {
                return Err(format!("{raw} is a directory."));
            }
            let old = std::fs::read_to_string(&path).ok();
            let body = match &old {
                Some(old) => write::unified_diff(raw, old, &content),
                None => content.clone(),
            };
            let approval = ApprovalRequest {
                title: "Approve write?".into(),
                subject: path.display().to_string(),
                body,
                outside_project: paths::is_outside(&ctx.project_root, &path),
                overwrite: old.is_some(),
            };
            Ok(Prepared::NeedsApproval(
                approval,
                Action::WriteFile { path, expected_old: old, new_content: content, edit: false },
            ))
        }
        other => Err(format!("unknown tool '{other}'. Available tools: read, bash, edit, write.")),
    }
}

/// Run a prepared action. Returns the status and the text for the model.
pub fn execute(
    action: &Action,
    ctx: &ToolContext,
    on_line: &mut dyn FnMut(&str),
    cancelled: &mut dyn FnMut() -> bool,
) -> (ToolStatus, String) {
    let result = match action {
        Action::Read { path, offset, limit } => read::read_file(path, *offset, *limit),
        Action::Bash { command, timeout } => {
            match bash::run(command, &ctx.cwd, &ctx.shim_dir, *timeout, on_line, cancelled) {
                Ok(r) if r.exit == bash::Exit::Interrupted => {
                    return (ToolStatus::Cancelled, r.for_model());
                }
                Ok(r) => Ok(r.for_model()),
                Err(e) => Err(format!("could not run bash: {e}")),
            }
        }
        Action::WriteFile { path, expected_old, new_content, edit } => {
            write_file(path, expected_old.as_deref(), new_content, *edit)
        }
    };
    match result {
        Ok(text) => (ToolStatus::Ok, text),
        Err(msg) => (ToolStatus::Error, format!("Error: {msg}")),
    }
}

fn write_file(path: &Path, expected_old: Option<&str>, new: &str, edit: bool) -> Result<String, String> {
    write::apply_write(path, expected_old, new)?;
    let shown = path.display();
    if edit {
        let (removed, added) = write::diff_stats(expected_old.unwrap_or(""), new);
        Ok(format!("Edited {shown} (−{removed} +{added} lines)"))
    } else {
        Ok(format!("Wrote {} bytes ({} lines) to {shown}", new.len(), new.lines().count()))
    }
}
