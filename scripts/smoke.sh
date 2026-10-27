#!/usr/bin/env bash
# End-to-end smoke test. Drives bridle in a detached tmux session against a
# running llama-server, the way a person would: type, approve, deny, interrupt.
#
# Usage: scripts/smoke.sh [bridle flags, e.g. --url http://127.0.0.1:9931]
# Needs: bash, tmux, python3 (for the file the model writes), a live server.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --quiet --manifest-path "$root/Cargo.toml"
bin="$root/target/debug/bridle"

work="$(mktemp -d)"
session="bridle-smoke-$$"
cleanup() {
    tmux kill-session -t "$session" 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

screen() { tmux capture-pane -t "$session" -p; }
keys() { tmux send-keys -t "$session" "$@"; }
type_line() { keys -l "$1"; keys Enter; }
transcript() { cat "$work"/.bridle/sessions/*.md 2>/dev/null || true; }
fail() {
    echo "FAIL: $1"
    echo "--- screen ---"; screen
    echo "--- transcript ---"; transcript
    exit 1
}
# wait_for SECONDS DESCRIPTION COMMAND... : poll until COMMAND succeeds.
wait_for() {
    local secs=$1 what=$2; shift 2
    for _ in $(seq 1 $((secs * 2))); do
        if "$@"; then return 0; fi
        sleep 0.5
    done
    fail "timed out after ${secs}s waiting for: $what"
}
on_screen() { screen | grep -qF -- "$1"; }
in_transcript() { transcript | grep -qF -- "$1"; }
idle() { ! on_screen "working…"; }
# Approve every request until the agent goes idle.
approve_until_idle() {
    for _ in $(seq 1 360); do
        if on_screen "[y] approve"; then keys y; sleep 1; continue; fi
        if idle; then return 0; fi
        sleep 0.5
    done
    fail "agent never went idle"
}

echo 'Answer as briefly as possible.' > "$work/AGENTS.md"
tmux new-session -d -s "$session" -x 120 -y 40 -c "$work" "$bin $*"

echo "== starts, connects, loads AGENTS.md"
wait_for 15 "status line" on_screen "· ctx "
wait_for 15 "AGENTS.md notice" on_screen "loaded AGENTS.md (1 lines)"
on_screen "server offline" && fail "server offline"

echo "== !command output goes into the conversation"
type_line '!echo smoke-ok'
wait_for 10 "shell output" on_screen "│ smoke-ok"
wait_for 10 "idle" idle

echo "== model writes a file and runs it (approving each step)"
type_line "Create hello.py that prints 'hello from bridle', then run it with python3."
sleep 1
approve_until_idle
[ -f "$work/hello.py" ] || fail "hello.py was not created"
wait_for 5 "run result in transcript" in_transcript "hello from bridle"

echo "== deny with a reason reaches the model"
type_line "Use bash to run: ls -la"
wait_for 120 "approval prompt" on_screen "[y] approve"
sleep 1 # answers are ignored for the first half second
keys r
type_line "not now"
wait_for 10 "denial in transcript" in_transcript "Denied by user: not now"
approve_until_idle

echo "== interrupting a running command"
type_line "Use bash to run: sleep 60"
wait_for 120 "approval prompt" on_screen "[y] approve"
sleep 1 # answers are ignored for the first half second
keys y
sleep 2
keys C-c
wait_for 10 "interrupted" on_screen "[interrupted]"
wait_for 10 "idle" idle

echo "== approve all, then turn it off"
type_line "Use bash to run: echo first"
wait_for 120 "approval prompt" on_screen "[y] approve"
sleep 1 # answers are ignored for the first half second
keys a
wait_for 10 "auto-approve on" on_screen "AUTO-APPROVE"
wait_for 120 "idle" idle
type_line "Use bash to run: echo second"
wait_for 120 "auto-approved call" on_screen "(auto)"
on_screen "[y] approve" && fail "asked for approval with auto-approve on"
wait_for 120 "idle" idle
keys C-x a
wait_for 10 "auto-approve off" on_screen "auto-approve is off"
in_transcript "**Auto-approved:**" || fail "transcript does not mark auto-approved calls"

echo "== reasoning stays hidden"
on_screen "▸ thought for" || fail "no collapsed thinking line"

echo "== quit"
keys C-c C-c
sleep 1
tmux has-session -t "$session" 2>/dev/null && fail "bridle did not exit"
ls "$work"/.bridle/sessions/*.jsonl >/dev/null || fail "no wire log"

echo "== --continue resumes the session"
tmux new-session -d -s "$session" -x 120 -y 40 -c "$work" "$bin $* --continue"
wait_for 15 "resumed notice" on_screen "resumed "
on_screen "AUTO-APPROVE" && fail "auto-approve survived a restart"
answers() { transcript | grep -c '^## Assistant' || true; }
before=$(answers)
type_line "In one word: what did the echo command in my first !command print?"
new_answer() { [ "$(answers)" -gt "$before" ]; }
wait_for 120 "an answer after resuming" new_answer
wait_for 120 "idle" idle
in_transcript "## Resumed" || fail "transcript has no resume marker"
keys C-c C-c
gone() { ! tmux has-session -t "$session" 2>/dev/null; }
wait_for 5 "bridle to exit after resume" gone

echo "PASS"
