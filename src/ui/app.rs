//! UI state, updated from agent events.

use crate::events::{AgentEvent, ApprovalRequest, ToolStatus};
use crate::ui::editor::Editor;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// One visual element of the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    User(String),
    /// A `!command`; `output` is None while it runs.
    Shell {
        command: String,
        live: Vec<String>,
        output: Option<String>,
    },
    /// Reasoning. `elapsed` is None while the model is still thinking.
    Thinking {
        text: String,
        started: Instant,
        elapsed: Option<Duration>,
    },
    Assistant(String),
    /// `status` is None while running.
    Tool {
        id: String,
        summary: String,
        status: Option<ToolStatus>,
        live: Vec<String>,
        result: String,
        /// Ran without asking because auto-approve was on.
        auto: bool,
    },
    Info(String),
    Error(String),
}

/// An approval the user is looking at.
#[derive(Debug, Clone)]
pub struct Pending {
    pub request: ApprovalRequest,
    pub scroll: u16,
    /// Some while typing a deny reason.
    pub reason: Option<Editor>,
    /// When the overlay appeared; answers are ignored for `APPROVAL_GRACE`.
    pub opened: Instant,
}

/// Keys pressed this soon after an approval appears are typed-ahead input,
/// not a decision about a command the user hasn't seen yet.
pub const APPROVAL_GRACE: Duration = Duration::from_millis(500);

/// Live output lines kept per running command.
const LIVE_LINES: usize = 200;

#[derive(Debug)]
pub struct App {
    pub blocks: Vec<Block>,
    pub input: Editor,
    /// The agent is working on something; Enter won't submit.
    pub busy: bool,
    pub pending: Option<Pending>,
    pub show_thinking: bool,
    pub show_tools: bool,
    /// Rows scrolled up from the bottom; 0 means following new output.
    pub scroll_back: usize,
    /// Total conversation rows at the last draw, to hold a scrolled-up view still.
    pub last_total: usize,
    /// Height of the conversation pane at the last draw.
    pub view_height: u16,
    /// Pretty JSON of each model exchange, for the inspector.
    pub wire: Vec<String>,
    /// Some(scroll) while the wire inspector is open.
    pub wire_view: Option<u16>,
    /// Ctrl-X was pressed; waiting for the second key.
    pub ctrl_x: bool,
    /// Ctrl-C was pressed on an empty input; another quits.
    pub quit_armed: bool,
    pub quit: bool,
    /// An interrupt was sent and the agent hasn't gone idle yet.
    pub interrupting: bool,
    /// Tool calls run without asking (shown in the status line).
    pub auto_approve: bool,
    pub model: String,
    pub online: bool,
    pub n_ctx: u64,
    pub ctx_used: u64,
    pub tokens_per_sec: Option<f64>,
    pub cwd: PathBuf,
    /// What the agent is doing, for the status line.
    pub activity: String,
}

impl App {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            blocks: vec![],
            input: Editor::new(),
            busy: false,
            pending: None,
            show_thinking: false,
            show_tools: false,
            scroll_back: 0,
            last_total: 0,
            view_height: 0,
            wire: vec![],
            wire_view: None,
            ctrl_x: false,
            quit_armed: false,
            quit: false,
            interrupting: false,
            auto_approve: false,
            model: String::new(),
            online: false,
            n_ctx: 0,
            ctx_used: 0,
            tokens_per_sec: None,
            cwd,
            activity: String::new(),
        }
    }

    fn tool_mut(&mut self, id: &str) -> Option<&mut Block> {
        self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Tool { id: i, .. } if i == id))
    }

    fn running_shell(&mut self) -> Option<&mut Block> {
        self.blocks.iter_mut().rev().find(|b| matches!(b, Block::Shell { output: None, .. }))
    }

    /// Stop the clock on a thinking block that is still open.
    fn end_thinking(&mut self) {
        if let Some(Block::Thinking { started, elapsed: elapsed @ None, .. }) = self.blocks.last_mut() {
            *elapsed = Some(started.elapsed());
        }
    }

    pub fn on_agent_event(&mut self, event: AgentEvent) {
        // Anything other than more reasoning means the thinking has finished.
        if !matches!(event, AgentEvent::ReasoningDelta(_)) {
            self.end_thinking();
        }
        match event {
            AgentEvent::ServerStatus { online, model, n_ctx } => {
                self.online = online;
                self.model = model;
                if n_ctx > 0 {
                    self.n_ctx = n_ctx;
                }
            }
            AgentEvent::ReasoningDelta(t) => {
                self.activity = "thinking".into();
                match self.blocks.last_mut() {
                    Some(Block::Thinking { text, elapsed: None, .. }) => text.push_str(&t),
                    _ => {
                        self.blocks.push(Block::Thinking { text: t, started: Instant::now(), elapsed: None })
                    }
                }
            }
            AgentEvent::TextDelta(t) => {
                self.activity = "writing".into();
                match self.blocks.last_mut() {
                    Some(Block::Assistant(s)) => s.push_str(&t),
                    _ => self.blocks.push(Block::Assistant(t)),
                }
            }
            AgentEvent::ToolCallParsed { id, summary, .. } => {
                self.activity = format!("running {summary}");
                self.blocks.push(Block::Tool {
                    id,
                    summary,
                    status: None,
                    live: vec![],
                    result: String::new(),
                    auto: false,
                });
            }
            AgentEvent::ApprovalNeeded(request) => {
                self.activity = "waiting for approval".into();
                self.pending = Some(Pending { request, scroll: 0, reason: None, opened: Instant::now() });
            }
            AgentEvent::AutoApprove(on) => {
                self.auto_approve = on;
                self.blocks.push(Block::Info(if on {
                    "auto-approve is on: tool calls run without asking (Ctrl-X a turns it off)".into()
                } else {
                    "auto-approve is off".into()
                }));
            }
            AgentEvent::AutoApproved { id } => {
                if let Some(Block::Tool { auto, .. }) = self.tool_mut(&id) {
                    *auto = true;
                }
            }
            AgentEvent::ToolOutput { id, line } => {
                let block = if id == "!" { self.running_shell() } else { self.tool_mut(&id) };
                match block {
                    Some(Block::Tool { live, .. }) | Some(Block::Shell { live, .. }) => {
                        live.push(line);
                        if live.len() > LIVE_LINES {
                            live.remove(0);
                        }
                    }
                    _ => {}
                }
            }
            AgentEvent::ToolResult { id, status, content } => {
                self.pending = None;
                if let Some(Block::Tool { status: s, result, live, .. }) = self.tool_mut(&id) {
                    *s = Some(status);
                    *result = content;
                    live.clear(); // the result holds the output now
                }
            }
            AgentEvent::ShellResult { command, output } => {
                if let Some(Block::Shell { output: o, live, .. }) = self.running_shell() {
                    *o = Some(output);
                    live.clear();
                } else {
                    self.blocks.push(Block::Shell { command, live: vec![], output: Some(output) });
                }
            }
            AgentEvent::CwdChanged(dir) => {
                if matches!(self.blocks.last(), Some(Block::Shell { output: None, .. })) {
                    self.blocks.pop();
                }
                self.blocks.push(Block::Info(format!("working directory is now {}", dir.display())));
                self.cwd = dir;
            }
            AgentEvent::TurnDone { usage, tokens_per_sec, interrupted } => {
                if let Some(u) = usage {
                    self.ctx_used = u.total_tokens;
                }
                if tokens_per_sec.is_some() {
                    self.tokens_per_sec = tokens_per_sec;
                }
                if interrupted {
                    self.blocks.push(Block::Info("[interrupted]".into()));
                }
            }
            AgentEvent::RestoreInput(text) => {
                // The request never happened: take its message off the screen.
                if let Some(i) = self.blocks.iter().rposition(|b| *b == Block::User(text.clone())) {
                    self.blocks.remove(i);
                }
                self.input.set_text(&text);
            }
            AgentEvent::Wire(json) => self.wire.push(json),
            AgentEvent::Error(e) => self.blocks.push(Block::Error(e)),
            AgentEvent::Info(text) => self.blocks.push(Block::Info(text)),
            AgentEvent::Idle => {
                self.busy = false;
                self.interrupting = false;
                self.pending = None;
                self.activity.clear();
            }
        }
    }
}
