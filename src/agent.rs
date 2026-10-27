//! The agent loop: the heart of the harness.
//!
//! The agent owns the conversation history. For each user message it calls
//! the model, runs any tool calls it asks for (after approval where needed),
//! appends the results, and calls the model again, until the model answers
//! without tool calls.
//!
//! Invariant: every tool call in the history gets exactly one tool result,
//! even when the user denies or interrupts.

use crate::events::{AgentEvent, ApprovalRequest, Command, ToolStatus};
use crate::llm::client::{Llm, LlmError, Streamed, chat_request};
use crate::llm::stream::Delta;
use crate::llm::types::{Completion, Message, ToolCall};
use crate::log::SessionLog;
use crate::tools::{self, Action, Prepared, ToolContext, bash, guard, paths};
use serde_json::{Value, json};
use std::cell::Cell;
use std::sync::mpsc::{Receiver, Sender};

pub const DEFAULT_MAX_STEPS: usize = 50;

pub struct AgentOptions {
    /// Use this model id instead of the one the server reports.
    pub model: Option<String>,
    /// Write session logs under `<project_root>/.bridle/sessions/`.
    pub log: bool,
    /// Model requests allowed per user message before asking to continue.
    pub max_steps: usize,
    /// Resume this saved session (a stamp from `session::resolve`).
    pub resume: Option<String>,
}

enum Decision {
    Approve,
    Deny(Option<String>),
    Interrupt,
}

pub struct Agent<L: Llm> {
    llm: L,
    options: AgentOptions,
    model: String,
    messages: Vec<Message>,
    ctx: ToolContext,
    log: SessionLog,
    events: Sender<AgentEvent>,
    commands: Receiver<Command>,
    /// Last known server state, so recovery can be reported.
    online: bool,
    /// Set when a Quit arrives while busy.
    quit: Cell<bool>,
    /// Run approval-needing calls without asking. Only the user's ApproveAll
    /// turns this on; it is never saved and is off in every new session.
    auto_approve: Cell<bool>,
}

impl<L: Llm> Agent<L> {
    pub fn new(
        llm: L,
        options: AgentOptions,
        ctx: ToolContext,
        events: Sender<AgentEvent>,
        commands: Receiver<Command>,
    ) -> Self {
        let mut agent = Self {
            llm,
            model: options.model.clone().unwrap_or_default(),
            options,
            messages: vec![],
            ctx,
            log: SessionLog::disabled(),
            events,
            commands,
            online: false,
            quit: Cell::new(false),
            auto_approve: Cell::new(false),
        };
        match agent.options.resume.clone() {
            None => agent.reset(),
            Some(stamp) => {
                if let Err(e) = agent.resume(&stamp) {
                    agent.reset();
                    agent.emit(AgentEvent::Error(format!(
                        "could not resume {stamp}: {e}; started a new session"
                    )));
                }
            }
        }
        agent
    }

    /// Load a saved session: its history (repaired if a crash left calls
    /// unanswered) under a freshly built system prompt, and its working
    /// directory. The UI is shown the history by replaying it as events.
    fn resume(&mut self, stamp: &str) -> Result<(), String> {
        let saved = crate::session::load(&crate::session::session_path(&self.ctx.project_root, stamp))?;
        let mut history = saved.messages;
        let repaired = crate::session::repair(&mut history);
        if saved.cwd.is_dir() {
            self.ctx.cwd = saved.cwd;
        }
        let agents_md = crate::prompt::load_agents_md(&self.ctx.project_root);
        let prompt = crate::prompt::system_prompt(&self.ctx.cwd, &crate::time::today(), agents_md.as_deref());
        self.messages = vec![Message::System { content: prompt }];
        self.messages.extend(history.iter().cloned());
        if self.options.log {
            self.log = SessionLog::resume(&self.ctx.project_root, stamp).map_err(|e| e.to_string())?;
        }
        self.log.md(&format!("## Resumed {}\n", crate::time::file_stamp()));
        if agents_md.is_some() {
            self.emit(AgentEvent::Info("loaded AGENTS.md".into()));
        }
        self.replay(&history);
        // After the replay, so it is on screen however long the history is.
        self.emit(AgentEvent::Info(format!("resumed {stamp} ({} messages)", history.len())));
        if repaired > 0 {
            let s = if repaired == 1 { "" } else { "s" };
            self.emit(AgentEvent::Info(format!(
                "{repaired} unanswered tool call{s} marked cancelled (bridle had exited)"
            )));
        }
        if self.ctx.cwd != self.ctx.project_root {
            self.emit(AgentEvent::CwdChanged(self.ctx.cwd.clone()));
        }
        self.save();
        self.emit(AgentEvent::Idle);
        Ok(())
    }

    /// Show restored history in the UI using the ordinary events.
    fn replay(&self, history: &[Message]) {
        for m in history {
            match m {
                Message::System { .. } => {}
                Message::User { content } => self.emit(AgentEvent::UserMessage(content.clone())),
                Message::Assistant { content, tool_calls } => {
                    if !content.is_empty() {
                        self.emit(AgentEvent::TextDelta(content.clone()));
                    }
                    for c in tool_calls {
                        let summary = tools::summarize(&c.function.name, &c.function.arguments);
                        self.emit(AgentEvent::ToolCallParsed {
                            id: c.id.clone(),
                            name: c.function.name.clone(),
                            summary,
                        });
                    }
                }
                Message::Tool { tool_call_id, content } => {
                    let status = if content.starts_with("Error:") {
                        ToolStatus::Error
                    } else if content.starts_with("Denied by user") {
                        ToolStatus::Denied
                    } else if content.starts_with("Cancelled") || content.ends_with("[killed: interrupted]") {
                        ToolStatus::Cancelled
                    } else {
                        ToolStatus::Ok
                    };
                    self.emit(AgentEvent::ToolResult {
                        id: tool_call_id.clone(),
                        status,
                        content: content.clone(),
                    });
                }
            }
        }
    }

    /// Save the history so the session can be resumed.
    fn save(&mut self) {
        self.log.save_session(&self.ctx.cwd, &self.messages[1..]);
    }

    /// Handle commands until Quit. Returns the final history.
    pub fn run(mut self) -> Vec<Message> {
        self.connect();
        while let Ok(command) = self.commands.recv() {
            match command {
                Command::Submit(text) => self.user_turn(text),
                Command::UserShell(command) => self.user_shell(command),
                Command::NewSession => self.reset(),
                Command::Quit => break,
                Command::AutoApproveOff => {
                    self.set_auto_approve(false);
                    continue;
                }
                // Only meaningful while busy; stale ones are dropped.
                Command::Approve | Command::ApproveAll | Command::Deny(_) | Command::Interrupt => continue,
            }
            self.save();
            if self.quit.get() {
                break;
            }
            self.emit(AgentEvent::Idle);
        }
        self.messages
    }

    fn emit(&self, event: AgentEvent) {
        // If the UI is gone there is nobody to tell.
        let _ = self.events.send(event);
    }

    /// Fresh history (re-reading AGENTS.md) and fresh log files.
    fn reset(&mut self) {
        self.set_auto_approve(false);
        let agents_md = crate::prompt::load_agents_md(&self.ctx.project_root);
        let prompt = crate::prompt::system_prompt(&self.ctx.cwd, &crate::time::today(), agents_md.as_deref());
        self.messages = vec![Message::System { content: prompt }];
        self.log = if self.options.log {
            SessionLog::create(&self.ctx.project_root, &crate::time::file_stamp())
                .unwrap_or_else(|_| SessionLog::disabled())
        } else {
            SessionLog::disabled()
        };
        if let Some(agents) = agents_md {
            self.log.md(&format!("## AGENTS.md\n\n{agents}\n"));
            self.emit(AgentEvent::Info(format!("loaded AGENTS.md ({} lines)", agents.lines().count())));
        }
    }

    /// Ask the server for its model and context size; report the result.
    fn connect(&mut self) -> bool {
        let probe = self.llm.probe();
        self.online = probe.is_ok();
        let n_ctx = match probe {
            Ok((model, n_ctx)) => {
                if self.options.model.is_none() {
                    self.model = model;
                }
                n_ctx
            }
            Err(_) => 0,
        };
        self.emit(AgentEvent::ServerStatus { online: self.online, model: self.model.clone(), n_ctx });
        self.online
    }

    /// Add a user message, merging with a preceding user message (e.g. from
    /// a `!command`) so roles keep alternating.
    fn push_user(&mut self, text: String) {
        if let Some(Message::User { content }) = self.messages.last_mut() {
            content.push_str("\n\n");
            content.push_str(&text);
        } else {
            self.messages.push(Message::User { content: text });
        }
    }

    fn user_turn(&mut self, text: String) {
        if self.model.is_empty() && !self.connect() {
            self.emit(AgentEvent::Error("server offline".into()));
            self.emit(AgentEvent::RestoreInput(text));
            return;
        }
        let before = self.messages.clone();
        self.log.md(&format!("## User\n\n{text}\n"));
        self.push_user(text.clone());
        self.save();

        let mut steps = 0;
        let mut first_request = true;
        loop {
            if steps == self.options.max_steps {
                if !self.ask_continue(steps) {
                    self.emit(AgentEvent::TurnDone { usage: None, tokens_per_sec: None, interrupted: false });
                    return;
                }
                steps = 0;
            }
            steps += 1;

            let request = chat_request(&self.model, &self.messages, &tools::definitions());
            let (result, interrupted) = self.stream(&request);
            let (completion, interrupted) = match result {
                Err(e) => {
                    if matches!(e, LlmError::Offline(_)) {
                        self.online = false;
                        self.emit(AgentEvent::ServerStatus {
                            online: false,
                            model: self.model.clone(),
                            n_ctx: 0,
                        });
                    }
                    self.log.md(&format!("**Error:** {e}\n"));
                    self.emit(AgentEvent::Error(e.to_string()));
                    if first_request {
                        // Nothing happened; put the history and the input back.
                        self.messages = before;
                        self.emit(AgentEvent::RestoreInput(text));
                    }
                    return;
                }
                Ok(Streamed::Aborted(c)) => (c, true),
                Ok(Streamed::Complete(c)) => (c, interrupted),
            };
            first_request = false;
            if !self.online {
                self.connect(); // the server is back; refresh the status line
            }
            self.record_wire(&request, &completion);
            self.log_completion(&completion);

            if interrupted {
                // Keep what the model said; drop half-formed tool calls.
                if !completion.content.is_empty() {
                    self.messages
                        .push(Message::Assistant { content: completion.content, tool_calls: vec![] });
                }
                self.log.md("_[interrupted]_\n");
                self.emit(AgentEvent::TurnDone {
                    usage: completion.usage,
                    tokens_per_sec: completion.tokens_per_sec,
                    interrupted: true,
                });
                return;
            }

            let calls = completion.tool_calls.clone();
            self.messages.push(Message::Assistant { content: completion.content, tool_calls: calls.clone() });
            self.save();
            if calls.is_empty() {
                self.emit(AgentEvent::TurnDone {
                    usage: completion.usage,
                    tokens_per_sec: completion.tokens_per_sec,
                    interrupted: false,
                });
                return;
            }

            for (i, call) in calls.iter().enumerate() {
                let (status, content) = self.run_call(call);
                self.messages.push(Message::Tool { tool_call_id: call.id.clone(), content });
                self.save();
                if status == ToolStatus::Cancelled {
                    for rest in &calls[i + 1..] {
                        self.cancel_call(rest);
                    }
                    self.emit(AgentEvent::TurnDone {
                        usage: completion.usage,
                        tokens_per_sec: completion.tokens_per_sec,
                        interrupted: true,
                    });
                    return;
                }
            }
        }
    }

    /// True if an Interrupt (or Quit) is waiting. Non-blocking. An
    /// AutoApproveOff that arrives while busy is applied here, so it isn't lost.
    fn interrupt_pending(&self) -> bool {
        match self.commands.try_recv() {
            Ok(Command::Interrupt) => true,
            Ok(Command::Quit) => {
                self.quit.set(true);
                true
            }
            Ok(Command::AutoApproveOff) => {
                self.set_auto_approve(false);
                false
            }
            _ => false,
        }
    }

    fn set_auto_approve(&self, on: bool) {
        if self.auto_approve.get() != on {
            self.auto_approve.set(on);
            self.emit(AgentEvent::AutoApprove(on));
        }
    }

    /// Stream one completion, forwarding deltas to the UI.
    /// Returns the result and whether the user interrupted.
    fn stream(&self, request: &Value) -> (Result<Streamed, LlmError>, bool) {
        let events = &self.events;
        let mut interrupted = false;
        let result = self.llm.stream(request, &mut |delta| {
            if self.interrupt_pending() {
                interrupted = true;
                return false;
            }
            let event = match delta {
                Delta::Reasoning(r) => AgentEvent::ReasoningDelta(r),
                Delta::Text(t) => AgentEvent::TextDelta(t),
            };
            let _ = events.send(event);
            true
        });
        (result, interrupted)
    }

    fn run_call(&mut self, call: &ToolCall) -> (ToolStatus, String) {
        let id = call.id.clone();
        let name = call.function.name.clone();
        let args = call.function.arguments.clone();
        let summary = tools::summarize(&name, &args);
        self.emit(AgentEvent::ToolCallParsed { id: id.clone(), name, summary: summary.clone() });
        self.log.md(&format!("### Tool call: {summary}\n\n```json\n{args}\n```\n"));

        let (status, content) = match tools::prepare(&call.function.name, &args, &self.ctx) {
            Prepared::Failed(msg) => (ToolStatus::Error, format!("Error: {msg}")),
            Prepared::Ready(action) => self.execute(&id, &action),
            Prepared::NeedsApproval(request, action) if self.auto_approve.get() => {
                self.log.md(&format!("**Auto-approved:**\n\n```\n{}\n```\n", request.body));
                self.emit(AgentEvent::AutoApproved { id: id.clone() });
                self.execute(&id, &action)
            }
            Prepared::NeedsApproval(request, action) => {
                self.log.md(&format!("Approval requested:\n\n```\n{}\n```\n", request.body));
                match self.ask(request) {
                    Decision::Approve => {
                        self.log.md("**Approved.**\n");
                        self.execute(&id, &action)
                    }
                    Decision::Deny(None) => (ToolStatus::Denied, "Denied by user.".to_string()),
                    Decision::Deny(Some(reason)) => (ToolStatus::Denied, format!("Denied by user: {reason}")),
                    Decision::Interrupt => (ToolStatus::Cancelled, "Cancelled by user.".to_string()),
                }
            }
        };
        self.log.md(&format!("Result ({status:?}):\n\n```\n{content}\n```\n"));
        self.emit(AgentEvent::ToolResult { id, status, content: content.clone() });
        (status, content)
    }

    /// Give an unrun call its "Cancelled" result.
    fn cancel_call(&mut self, call: &ToolCall) {
        let content = "Cancelled by user.".to_string();
        let summary = tools::summarize(&call.function.name, &call.function.arguments);
        self.emit(AgentEvent::ToolCallParsed {
            id: call.id.clone(),
            name: call.function.name.clone(),
            summary,
        });
        self.emit(AgentEvent::ToolResult {
            id: call.id.clone(),
            status: ToolStatus::Cancelled,
            content: content.clone(),
        });
        self.messages.push(Message::Tool { tool_call_id: call.id.clone(), content });
    }

    fn execute(&self, id: &str, action: &Action) -> (ToolStatus, String) {
        let events = &self.events;
        tools::execute(
            action,
            &self.ctx,
            &mut |line| {
                let _ = events.send(AgentEvent::ToolOutput { id: id.to_string(), line: line.to_string() });
            },
            &mut || self.interrupt_pending(),
        )
    }

    /// Show an approval request and block until the user decides.
    fn ask(&self, request: ApprovalRequest) -> Decision {
        self.emit(AgentEvent::ApprovalNeeded(request));
        loop {
            match self.commands.recv() {
                Ok(Command::Approve) => return Decision::Approve,
                Ok(Command::ApproveAll) => {
                    self.set_auto_approve(true);
                    return Decision::Approve;
                }
                Ok(Command::AutoApproveOff) => self.set_auto_approve(false),
                Ok(Command::Deny(reason)) => return Decision::Deny(reason),
                Ok(Command::Interrupt) => return Decision::Interrupt,
                Ok(Command::Quit) | Err(_) => {
                    self.quit.set(true);
                    return Decision::Interrupt;
                }
                Ok(_) => continue,
            }
        }
    }

    fn ask_continue(&self, steps: usize) -> bool {
        let request = ApprovalRequest {
            title: "Continue?".into(),
            subject: String::new(),
            body: format!(
                "The model has made {steps} requests for this message.\n\
                 Approve to allow {} more; deny to stop here.",
                self.options.max_steps
            ),
            outside_project: false,
            overwrite: false,
        };
        matches!(self.ask(request), Decision::Approve)
    }

    /// Handle `!command`: run it, or change directory for a bare `!cd`.
    fn user_shell(&mut self, command: String) {
        if let Some(dir) = parse_cd(&command) {
            let target = paths::resolve(&self.ctx.cwd, dir);
            if !target.is_dir() {
                let output = format!("cd: no such directory: {}", target.display());
                self.emit(AgentEvent::ShellResult { command, output });
                return;
            }
            self.ctx.cwd = target.clone();
            let note = format!("Working directory is now {}", target.display());
            self.log.md(&format!("_{note}_\n"));
            self.push_user(note);
            self.emit(AgentEvent::CwdChanged(target));
            return;
        }

        let output = match guard::check_command(&command) {
            Err(msg) => format!("Error: {msg}"),
            Ok(()) => {
                let events = &self.events;
                let result = bash::run(
                    &command,
                    &self.ctx.cwd,
                    &self.ctx.shim_dir,
                    bash::DEFAULT_TIMEOUT,
                    &mut |line| {
                        let _ =
                            events.send(AgentEvent::ToolOutput { id: "!".into(), line: line.to_string() });
                    },
                    &mut || self.interrupt_pending(),
                );
                match result {
                    Ok(r) => r.for_model(),
                    Err(e) => format!("Error: could not run bash: {e}"),
                }
            }
        };
        let message =
            format!("I ran this command in {}:\n$ {command}\n```\n{output}\n```", self.ctx.cwd.display());
        self.log.md(&format!("## User shell\n\n{message}\n"));
        self.push_user(message);
        self.emit(AgentEvent::ShellResult { command, output });
    }

    fn record_wire(&mut self, request: &Value, c: &Completion) {
        let mut record = json!({
            "request": request,
            "response": {
                "content": c.content,
                "reasoning": c.reasoning,
                "tool_calls": c.tool_calls,
                "finish_reason": c.finish_reason,
                "usage": c.usage,
                "tokens_per_sec": c.tokens_per_sec,
            },
        });
        let raw = self.llm.take_raw();
        if !raw.is_empty() {
            record["raw"] = json!(raw);
        }
        self.log.wire(&record);
        self.emit(AgentEvent::Wire(serde_json::to_string_pretty(&record).unwrap_or_default()));
    }

    fn log_completion(&mut self, c: &Completion) {
        if !c.reasoning.is_empty() {
            self.log.md(&format!("### Thinking\n\n{}\n", c.reasoning));
        }
        if !c.content.is_empty() {
            self.log.md(&format!("## Assistant\n\n{}\n", c.content));
        }
    }
}

/// If `command` is a bare `cd [dir]`, return the directory (`~` if none).
/// Anything more complex (`cd a && make`, quoting, variables) returns None
/// and runs as a normal command.
pub fn parse_cd(command: &str) -> Option<&str> {
    let rest = command.trim().strip_prefix("cd")?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None; // e.g. "cdk"
    }
    let arg = rest.trim();
    if arg.is_empty() {
        return Some("~");
    }
    if arg.contains(|c: char| c.is_whitespace() || ";&|<>$`'\"(){}*?".contains(c)) {
        return None;
    }
    Some(arg)
}
