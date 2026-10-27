# bridle

A minimal terminal coding harness for local models served by llama.cpp's
`llama-server`. Four tools (read, bash, edit, write), one agent loop, and
nothing hidden: every request and response is logged.

Runs on macOS and Linux.

## Running

Start llama-server with tool-calling templates enabled:

    llama-server -m <model.gguf> --jinja --port 9931

Then, in the project you want to work on:

    bridle [--url http://127.0.0.1:9931] [--model ID] [--wire-raw]

- `--url` — server base URL (default `http://127.0.0.1:9931`)
- `--model` — model id to request (default: the server's first model)
- `--wire-raw` — also log every raw SSE line

Install with `cargo install --path .` from this repository.

## AGENTS.md

If the directory you launch bridle in has an `AGENTS.md`, it is appended to
the system prompt as project instructions (read at start and on `/new`,
capped at 32 KB). It can shape the model's behavior however you like, but it
is only text for the model: approval and the privilege-escalation guard are
enforced in code and cannot be turned off by it.

## Tools and approval

| Tool  | Approval | Notes |
|-------|----------|-------|
| read  | no       | numbered lines, 2000 per call, `offset`/`limit` to page |
| bash  | yes      | fresh `bash -c` per call in the working directory; no tty, no stdin; 120 s default timeout (max 600); output over 30,000 chars is cut in the middle |
| edit  | yes      | `old_text` must match exactly once; you approve the exact diff that will be written |
| write | yes      | creates parent dirs; overwrites are shown as a diff |

The approval box shows the exact command or diff, wrapped so nothing is
hidden, with a "rows A–B of N" position. `y` approves, `n` denies, `r` denies
with a reason that is passed to the model. Keys pressed in the first half
second after the box appears are ignored, so typing ahead can't answer it.

`a` approves the call **and every later one this session** (auto-approve).
The status line shows `AUTO-APPROVE` while it is on, each such call is marked
`(auto)`, and the transcript records it as auto-approved. Ctrl-X a turns it
off; `/new` and every new session start with it off. It can only be turned
on from an approval box, never by the model or AGENTS.md, and the
privilege-escalation guard still applies. The "Continue?" prompt after 50
model requests still asks. Changes outside the
launch directory are marked **OUTSIDE PROJECT**.

### Privilege escalation

Commands mentioning `sudo`, `su`, `doas`, `pkexec`, `run0` or "with administrator
privileges" are refused before you are asked. Commands also run without a
controlling terminal, with a failing `sudo` first on `PATH`. This is a
guard, not a sandbox: approve commands with care.

## `!` commands

`!cmd` runs `cmd` yourself (no approval, same guard) and adds the command and
its output to the conversation; the model sees it with your next message.
It is not an interactive shell: no tty, no stdin, no pagers.

A bare `!cd DIR` changes bridle's working directory for all later commands
and tells the model. `cd` inside a longer command (`!cd x && make`) does
not persist.

## Keys

| Key | Action |
|---|---|
| Enter | send (when idle) |
| Alt-Enter, Ctrl-J | new line |
| Ctrl-V / Alt-V | scroll half a screen forward / back |
| PgDn / PgUp | scroll a screen forward / back |
| Ctrl-X o | show/hide tool output |
| Ctrl-X t | show/hide thinking (hidden by default) |
| Ctrl-X w | wire inspector (raw JSON of each exchange) |
| Ctrl-X a | turn auto-approve off |
| Ctrl-C | interrupt (again to force quit if the server doesn't respond); clear input; twice on empty input to quit |

Input editing: Ctrl-F/B (char), Alt-F/B (word), Ctrl-A/E (line start/end),
Ctrl-P/N (line up/down), Ctrl-D/H (delete char), Alt-D / Alt-Backspace
(kill word), Ctrl-K / Ctrl-U (kill to end/start of line), Ctrl-Y (yank).
On macOS the terminal must send Option as Meta.

Commands: `/new` starts a fresh conversation, `/quit` exits.

Inside tmux, note that tmux's default prefix (Ctrl-B) and the screen-style
Ctrl-A both shadow editing keys; press the prefix twice to send it through,
or pick another prefix.

## Logs

Each session writes to `.bridle/sessions/` in the launch directory:

- `<time>.md` — readable transcript, including every tool call, approval
  decision and result
- `<time>.jsonl` — one line per model request: the full request body and
  the assembled response (plus raw SSE lines with `--wire-raw`)

`.bridle/` contains a `.gitignore` so logs are never committed.

## Testing

    cargo test                              # unit and loop tests, no server needed
    cargo test --test live -- --ignored     # talks to llama-server on :9931
    scripts/smoke.sh                        # end-to-end in tmux against a live server

## How it works

- `src/agent.rs` — the loop: call the model, run tool calls (asking for
  approval), append results, repeat until the model stops calling tools
- `src/llm/stream.rs` — reassembles the streamed response, including tool
  calls whose arguments arrive in fragments
- `src/tools/` — the four tools and the guard
- `src/ui/` — the terminal interface; it talks to the agent only through
  `src/events.rs`
