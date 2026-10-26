# bridle — design

Date: 2026-10-26
Status: draft, awaiting review

## 1. Purpose

bridle is a minimal terminal coding harness for local models served by
llama.cpp's OpenAI-compatible API. Goals, in priority order:

1. **Education.** Every behavior of the harness should be readable in its
   source and observable at runtime (transcript, wire log, inspector). No
   hidden SDKs, no framework magic.
2. **Backup environment.** A usable offline coding agent when hosted tools
   are unavailable.

The v1 feature set follows early versions of pi: four tools (`read`, `bash`,
`edit`, `write`) and a short system prompt. Growth comes later, possibly
authored by bridle itself.

### Non-goals for v1

- Multiple providers or endpoints in one session; session persistence/resume.
- Context compaction (262k context window makes this a later problem).
- Auto-approval mode (planned; see end of §6.4).
- Sandboxing beyond the sudo guard (planned: `sandbox-exec`).
- Markdown rendering beyond styled code fences; mouse capture.
- Async runtime; concurrent tool execution; message queueing while busy.

## 2. Environment

- Server: llama.cpp `llama-server` (build b11912), `--jinja` template active,
  at `http://127.0.0.1:9931`. Model: `Qwen3.8-27B-UD-Q6_K_M.gguf`,
  `n_ctx` 262144, 1 slot. Template supports tools, parallel tool calls and
  reasoning (`reasoning_content` returned separately).
- Platform: macOS (darwin), Rust stable (cargo 1.99).

## 3. Architecture

Single binary crate. Concurrency is plain OS threads plus `std::sync::mpsc`
channels: an **agent thread** (core) and the **UI thread** (main).

```
src/
  main.rs        flags, terminal setup, panic hook, spawn agent thread, run UI
  config.rs      --url (default http://127.0.0.1:9931), --model (default: first
                 id from GET /v1/models), --wire-raw
  events.rs      ★ AgentEvent (core → UI) and Command (UI → core)
  agent.rs       ★ the loop; owns message history and state machine
  prompt.rs      system prompt
  llm/
    types.rs     serde types for the OpenAI chat format
    client.rs    POST /v1/chat/completions stream:true; SSE line reader;
                 `Llm` trait + `LlamaClient` impl
    stream.rs    ★ accumulate deltas into one complete assistant message
  tools/
    mod.rs       tool schemas, dispatch by name, path resolution
    read.rs bash.rs edit.rs write.rs
    guard.rs     privilege-escalation blocklist; bash process environment
  ui/
    app.rs       UI state: blocks, input buffer, scroll, pending approval
    render.rs    ratatui drawing
    input.rs     key handling → Command
  log.rs         transcript (.md) and wire log (.jsonl)
```

Boundary rules:
- `agent`, `llm`, `tools`, `log` never import `ratatui`/`crossterm`.
- `ui` never does HTTP or runs tools.
- Core and UI communicate only via the `events.rs` types.

Dependencies: `ratatui`, `crossterm`, `ureq`, `serde`, `serde_json`,
`similar`, `anyhow`, `lexopt`. Nothing else without a reason.

## 4. Protocol (llm)

- Request: `model`, `messages`, `tools` (4 function definitions), `stream:
  true`, `stream_options: {include_usage: true}`. No sampling overrides in v1
  (server defaults apply).
- SSE parsing is hand-written: read lines, handle `data: {...}` and
  `data: [DONE]`, ignore blank lines / comments.
- `stream.rs` accumulates `delta.content`, `delta.reasoning_content`, and
  `delta.tool_calls[]` — tool calls are merged by `index`; `id` and
  `function.name` arrive once, `function.arguments` arrives as string
  fragments that are concatenated. Final usage and `finish_reason` captured.
- Tool-call arguments are parsed as JSON only after the stream completes.
- The `Llm` trait (`fn stream(&self, req, on_delta) -> Result<Assistant>` or
  equivalent) is the seam for `FakeLlm` in tests.

## 5. Agent loop

States: `Idle → Streaming → (tool calls?) → [AwaitingApproval → Running]* →
Streaming … → Idle`.

1. `Command::Submit(text)` appends a user message and enters `Streaming`.
2. Stream ends. Reasoning is kept for UI/log but **not** sent back in later
   requests. If no tool calls: emit `TurnDone{usage}` → `Idle`.
3. Tool calls are executed **sequentially in order**:
   - `read`: run immediately.
   - `bash`/`edit`/`write`: if guard rejects → error result, no prompt.
     Otherwise emit `ApprovalNeeded{preview}` and block on the command
     channel: `Approve` → run; `Deny(Option<reason>)` → result
     `"Denied by user."` or `"Denied by user: <reason>"`.
   - Append `{role:"tool", tool_call_id, content}` per call.
4. After all results, return to step 2 with a new request.
5. Step cap: 50 model requests per user message; at the cap, ask the user
   (approval-style prompt) whether to continue.

**Invariant:** every assistant `tool_call` in history has exactly one tool
result. On interrupt mid-batch, unrun calls get `"Cancelled by user."`. On
interrupt mid-stream, partial text is kept (marked interrupted) and
incomplete tool calls are discarded.

Interrupt detection: `try_recv` between SSE chunks; blocking `recv` while
awaiting approval; 50 ms poll loop (child `try_wait` + `try_recv`) while bash
runs, killing the process group on interrupt.

Events: `ReasoningDelta`, `TextDelta`, `ToolCallParsed`, `ApprovalNeeded`,
`ToolOutput` (streaming bash lines), `ToolResult`, `TurnDone{usage, tok/s}`,
`Error`, `CwdChanged`, `ServerStatus`.

Commands: `Submit`, `Approve`, `Deny(Option<String>)`, `Interrupt`,
`UserShell(String)`, `NewSession`, `Quit`.

## 6. Tools

Paths may be relative (to the session working directory) or absolute. All
failures are returned as tool results prefixed `Error:`; only harness-level
failures (e.g. server unreachable) abort a turn.

### 6.1 read `{path, offset?, limit?}` — no approval
- Output lines formatted `{n:>6}│ {line}`; default/max limit 2000 lines;
  lines over 2000 chars truncated with a marker.
- Footer when more remains: `[lines A–B of N — call read again with
  offset=B+1 to continue]`.
- Errors: not found; is a directory (suggest `ls` via bash); binary (NUL in
  first 8 KB); not UTF-8.
- Reads outside the project are permitted.

### 6.2 bash `{command, timeout?}` — approval + guard
- `bash -c <command>`, cwd = session working directory, new session
  (`setsid`), stdin `/dev/null`, stdout+stderr merged in arrival order,
  streamed to UI.
- Timeout default 120 s, max 600 s; on timeout the process group is killed.
- Fresh shell every call; `cd` does not persist (stated in description).
- Result: output + `\n[exit code: N]` (or `[killed: timeout]` /
  `[killed: interrupted]`). Output over 30,000 chars keeps head and tail with
  `[… N chars omitted …]` between, biased toward the tail.

### 6.3 edit `{path, old_text, new_text}` — approval
- `old_text` must occur exactly once.
  - 0 matches: `Error: old_text not found in {path}. The file may have
    changed — read it again. Check whitespace and indentation exactly.`
  - >1: `Error: old_text matches K times (lines a, b, c). Include more
    surrounding lines to make it unique.`
- New content is computed **before** the approval prompt; preview is a
  unified diff of exactly that content. On approve, the file is re-read and
  must be byte-identical to what the diff was computed from, else
  `Error: {path} changed since the edit was proposed; read it again.`
- Result: `Edited {path} (−a +b lines)`.

### 6.4 write `{path, content}` — approval
- Creates parent directories.
- Preview: existing file → diff vs current, labeled OVERWRITE; new file →
  full content (scrollable). Same stale check as edit for overwrites.
- Result: `Wrote N bytes (M lines) to {path}`.

For bash/edit/write, targets outside the project root are allowed but the
approval overlay shows a red **OUTSIDE PROJECT** marker.

**Future (not v1):** an "approve all from now on" key in the approval overlay,
toggling auto-approval; to be added once the transcript has established that
proposed and executed actions match.

### 6.5 Guard (`guard.rs`)
Layered, honest about limits — it is not a sandbox:
1. Blocklist: commands matching `sudo`, `su`, `doas`, `pkexec`, or
   `with administrator privileges` (as words) are rejected before approval
   with `Error: privilege escalation is not permitted in bridle.`
2. bash runs with no controlling tty (`setsid`) — defeats macOS per-tty sudo
   timestamp reuse and password prompting. `SUDO_ASKPASS=/usr/bin/false`.
3. A `sudo` shim that always fails is placed first on `PATH` (in a temp dir
   created at startup).
4. User approval of every command.

Known gap: with a `NOPASSWD` sudoers rule, a command that evades the
blocklist (e.g. an obfuscated `/usr/bin/sudo`) defeats layers 2–3; only
layer 4 (approval) covers it.
Applies equally to `!` commands.

## 7. System prompt

```
You are an expert coding assistant running inside bridle, a minimal terminal
coding harness. You help users with coding tasks by reading files, executing
commands, editing code, and writing new files.

Available tools:
- read: Read file contents
- bash: Execute bash commands
- edit: Make surgical edits to files
- write: Create or overwrite files

Guidelines:
- Use bash for file operations like ls, grep, find
- Use read to examine files before editing
- Use edit for precise changes (old text must match exactly)
- Use write only for new files or complete rewrites
- Each bash call runs in a fresh shell in the working directory; cd does not persist
- bash, edit and write require the user's approval; if denied, follow the user's reason
- Never use sudo or attempt privilege escalation
- When summarizing your actions, output plain text directly - do NOT use cat or bash to display what you did
- Be concise in your responses
- Show file paths clearly when working with files

Documentation:
- Your own documentation is at: {BRIDLE_README}
- Read it when users ask about bridle's features, configuration, or setup.

Environment:
- Working directory: {cwd}
- Date: {date}
- OS: {os}
```

`{BRIDLE_README}` is `env!("CARGO_MANIFEST_DIR")/README.md` at compile time.

## 8. UI (ratatui, full screen / alternate screen)

Layout: conversation pane (top), input (multi-line, grows to ~6 lines),
status line (model · ctx used/total · tok/s · state · key hint).

Blocks in the conversation:
- user message; `!` command + output;
- thinking — dimmed while streaming, collapses to `▸ thinking (N tokens)`;
- assistant text — plain, code fences styled;
- tool call — `⏳/✓/✗/⊘ tool summary`; running bash shows last 8 output
  lines; collapses on completion.

Approval overlay (modal, centered): title `Approve {tool}?` + path; body is
the exact bash command or the diff (j/k/PgUp/PgDn scroll); footer
`[y] approve  [n] deny  [r] deny with reason` (r opens an inline text field).

Keys — global:

| Key | Action |
|---|---|
| Enter | submit (idle only) |
| Alt-Enter / Ctrl-J | newline in input |
| PgUp/PgDn, Ctrl-V / Meta-V | scroll conversation down/up; auto-follow unless scrolled up |
| Ctrl-X o | toggle all tool output |
| Ctrl-X t | toggle all thinking |
| Ctrl-X w | wire inspector overlay (raw request/response JSON) |
| Ctrl-C | interrupt if busy; else clear input; twice on empty input quits |

Keys — input editing (emacs/readline basics). "Line" means the current
logical line of the multi-line input. A "word" is a run of non-whitespace
characters.

| Key | Action |
|---|---|
| Ctrl-F / → | forward one character |
| Ctrl-B / ← | back one character |
| Meta-F | forward to the end of the current/next word |
| Meta-B | back to the start of the current/previous word |
| Ctrl-A / Home | start of line |
| Ctrl-E / End | end of line |
| Ctrl-P / ↑ | previous line |
| Ctrl-N / ↓ | next line |
| Ctrl-D / Delete | delete character forward |
| Ctrl-H / Backspace | delete character back |
| Meta-D | kill word forward |
| Meta-Backspace | kill word back |
| Ctrl-K | kill to end of line (at end of line: join with next line) |
| Ctrl-U | kill to start of line |
| Ctrl-Y | yank (paste) the last kill |

- Single-slot kill buffer, internal to bridle (not the system clipboard).
- Meta = Alt. On macOS the terminal must send Option as Meta (Terminal.app:
  "Use Option as Meta key"; iTerm2: Option key "Esc+"); otherwise Option-F
  types `ƒ`. crossterm reports these as `ALT`-modified keys.
- Toggles live under a `Ctrl-X` prefix so Ctrl-letter keys stay free for
  editing. The status line shows `C-x-` while the prefix is pending; any
  unbound second key cancels it.

Slash commands: `/new` (reset conversation, new log files), `/quit`.

No mouse capture (preserves terminal text selection).

## 9. `!` commands

- `!<cmd>` runs through the same bash runner and guard as the tool; no
  approval; no model request.
- Appended to history as a user message:
  ````
  I ran this command in {cwd}:
  $ {cmd}
  ```
  {output}
  [exit code: N]
  ```
  ````
  The model sees it with the next submitted message.
- Not an interactive shell: no tty, stdin `/dev/null`; interactive programs
  and pagers don't work.
- A bare `!cd <dir>` (only that, nothing else in the line) changes the
  session working directory (validated, `~` expanded) and appends
  `Working directory is now {dir}` to history. Compound commands containing
  `cd` run normally and do not change it. The model cannot change the
  session working directory.
- Project root (for OUTSIDE PROJECT and log location) stays the launch
  directory.

## 10. Errors

- Server unreachable: status shows offline; input is retained for resubmit.
- HTTP non-2xx: error block with response body; turn ends; history remains
  valid (no dangling assistant message).
- Malformed tool arguments / unknown tool: `Error:` tool result including
  the parse error and raw arguments.
- Panic hook restores the terminal before printing the panic.

## 11. Logging

Directory `./.bridle/sessions/` under the project root (launch directory);
`.bridle/.gitignore` containing `*` is created on first run.

- `{timestamp}.md` — human-readable transcript: user messages, thinking,
  assistant text, every tool call with full arguments, approval decision,
  and full result.
- `{timestamp}.jsonl` — one record per model request: full request body,
  reassembled response, usage, timings; with `--wire-raw`, every SSE chunk.
- Ctrl-W inspector reads the same records from memory.

## 12. Testing

- Unit: SSE parsing/accumulation from fixtures captured from the live server
  (including fragmented tool-call arguments and parallel calls); edit match
  cases and stale check; truncation; guard; `!cd` parsing; read
  paging/binary detection.
- Agent loop: `FakeLlm` with scripted responses driven via the command
  channel; assert the tool-result invariant across approve, deny, deny with
  reason, interrupt mid-stream, interrupt mid-batch, step cap.
- UI: `TestBackend` snapshots of the approval overlay and main layout.
- Manual end-to-end against the live server: create a file, edit it, run it,
  deny a command with a reason, interrupt a long `sleep`.
