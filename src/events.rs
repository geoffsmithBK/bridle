//! The contract between the agent core and the UI.

use crate::llm::types::Usage;
use std::path::PathBuf;

/// UI → core.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A user message for the model.
    Submit(String),
    /// A `!command` typed by the user (without the `!`).
    UserShell(String),
    Approve,
    /// Deny the pending approval, optionally with a reason for the model.
    Deny(Option<String>),
    Interrupt,
    /// Clear the conversation and start new log files.
    NewSession,
    Quit,
}

/// What the user is being asked to approve.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRequest {
    /// e.g. "Approve bash?"
    pub title: String,
    /// Path being changed, or empty.
    pub subject: String,
    /// The exact command, a unified diff, or full new-file content.
    pub body: String,
    pub outside_project: bool,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Ok,
    Error,
    Denied,
    Cancelled,
}

/// Core → UI.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    ServerStatus {
        online: bool,
        model: String,
        n_ctx: u64,
    },
    ReasoningDelta(String),
    TextDelta(String),
    /// A complete tool call was received from the model.
    ToolCallParsed {
        id: String,
        name: String,
        summary: String,
    },
    ApprovalNeeded(ApprovalRequest),
    /// One line of live output from a running command.
    ToolOutput {
        id: String,
        line: String,
    },
    ToolResult {
        id: String,
        status: ToolStatus,
        content: String,
    },
    /// Result of a `!command`.
    ShellResult {
        command: String,
        output: String,
    },
    CwdChanged(PathBuf),
    TurnDone {
        usage: Option<Usage>,
        tokens_per_sec: Option<f64>,
        interrupted: bool,
    },
    /// A failed request; the text should go back into the input box.
    RestoreInput(String),
    /// Pretty-printed JSON of one request/response exchange, for the inspector.
    Wire(String),
    Error(String),
    /// Something worth a dim line in the conversation (e.g. AGENTS.md loaded).
    Info(String),
    /// The core finished handling a command and is waiting for the next one.
    Idle,
}
