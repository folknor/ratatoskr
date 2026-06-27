#!/usr/bin/env python3
"""Shared launcher for the spec-loop codex roles (reference/orchestrate.md).

`run_codex()` invokes `codex exec` with the canonical flags for one role,
captures the NDJSON stream, and prints a clean digest to stdout: the final
agent message, token usage, and any plain-text log lines the codex harness
interleaved into the stream. The final message is captured via
`--output-last-message` as a backstop, so even a frozen/halted stream (a
known codex harness behaviour when a mid-run internal-tool call errors)
still yields the report once the process exits.

Two deliberate simplifications, matching the methodology:

- We never resume a codex thread. A run that ends with its goal unmet is
  replaced by a fresh run, so the thread id is not surfaced.
- The caller passes exactly one argument: the prompt, one line, in ''. The
  role scripts (codex-review.py / codex-implement.py) own the model, the
  reasoning effort, and whether the prompt is /goal-driven.

The raw NDJSON is captured inside this process and never reaches the
caller's stdout, so the launching shell sees only the digest - no stream to
flood a context or freeze a peek.
"""
import json
import os
import subprocess
import sys

MODEL = "gpt-5.5"


# Future improvements, from a study of how cmux tracks a codex agent's
# running/done state. cmux deliberately does NOT trust the codex stdout
# / --json stream for liveness or completion; it derives state from off-stream
# signals. We do not need most of this for a blocking wait-for-exit wrapper -
# the wrapping process is the liveness signal and -o is the completion signal -
# but if this ever grows into a non-blocking or live-state orchestrator, these
# are the upgrades, in rough order of value:
#
# 1. Read codex's session JSONL transcript on disk as the source of truth
#    (cmux: findCodexTranscriptPath under $CODEX_HOME). It carries the same
#    events as --json (task_started, task_complete / turn_complete, error,
#    stream_error) plus the assistant message and per-turn usage, and it
#    SURVIVES a frozen stream. Reading it at exit would recover full usage even
#    when --json halts, and would let us classify WHY a run stopped: a
#    `stream_error` event while the process is still alive is the internal-tool
#    -error-froze-the-stream case; a terminal turn with no assistant message is
#    "ended before final response". (We currently get the final message from -o
#    and accept that --json usage may be partial; the action - relaunch fresh -
#    is the same regardless of why, so this is enrichment, not a fix.)
# 2. --output-schema <FILE>: force the final message into a JSON schema with an
#    explicit goal_met / remaining shape, so the fresh-implementer decision is
#    machine-read instead of prose-judged.
# 3. codex lifecycle hooks (SessionStart / UserPromptSubmit / Stop, written to
#    $CODEX_HOME/hooks.json plus codex_hooks=true in config.toml) give a live
#    running / idle / needs-input state without parsing any stream. cmux keeps
#    the synchronous hook handler trivial (a 5ms budget) and offloads all slow
#    work to a detached monitor. Only worth it if we need mid-run state (e.g. a
#    "needs input" signal), which a batch /goal run does not.
# 4. A detached transcript-monitor subprocess that file-watches the transcript
#    (kqueue/inotify on mtime + appended bytes) with a ~30s poll fallback and a
#    hard deadline (cmux uses 4h), decoupled from the hook stream and from
#    process exit. This is cmux's answer to "frozen stream / yields without
#    exiting"; only needed if we leave the blocking-subprocess model.
# 5. A per-turn lease file (created at submit, retired at stop, expired by age)
#    to disambiguate in-flight from done/orphaned without a PID check - again,
#    only for a non-blocking/daemon model.
# 6. Tail-fingerprint stall heuristic (cmux AgentHibernationController): hash
#    the last ~12 lines of output and treat "fingerprint unchanged for N
#    seconds, confirmed across two windows, fused with PID liveness" as stalled.
#    Useful only if we want to auto-detect a stuck-but-alive run instead of
#    leaving that to human escalation.
# 7. Stale-session guard (cmux hasNewerRunningSession): if we ever run
#    overlapping or nested codex turns, suppress a late Stop from a dead turn
#    flipping state back to idle.


def run_codex(prompt, effort, goal):
    if not prompt or not prompt.strip():
        print("usage: pass exactly one argument - the prompt, one line, in ''",
              file=sys.stderr)
        return 2
    if goal:
        prompt = "/goal " + prompt

    os.makedirs(".brokkr/codex", exist_ok=True)
    last_msg_path = os.path.join(".brokkr/codex", f"last-{os.getpid()}.txt")
    cmd = [
        "codex", "exec",
        "--json",
        "--sandbox", "workspace-write",
        "-m", MODEL,
        "-c", f"model_reasoning_effort={effort}",
        "-o", last_msg_path,
        prompt,
    ]
    # Close stdin: the prompt is the positional arg, and an open stdin makes
    # codex try to read an extra `<stdin>` block ("Reading additional input
    # from stdin...") and could block on a TTY.
    # start_new_session=True: setsid() before exec, so codex leads its own
    # session + process group. Tools codex spawns that group-kill (e.g.
    # brokkr's kill(-pgid) deadline / --hard paths) stay contained to this
    # codex subtree instead of escalating up the shared launcher PG and
    # taking out this launcher or a sibling codex. At worst a stray
    # group-kill ends this run, which the relaunch-fresh model handles.
    result = subprocess.run(cmd, stdin=subprocess.DEVNULL,
                            capture_output=True, text=True,
                            start_new_session=True)

    usage = {"input_tokens": 0, "cached_input_tokens": 0,
             "output_tokens": 0, "reasoning_output_tokens": 0}
    turns = 0
    stream_message = None
    log_lines = []
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            # A plain-text log line (codex ERROR/WARN, apply_patch dump). The
            # harness can halt NDJSON emission here, so keep it visible.
            log_lines.append(line)
            continue
        if not isinstance(event, dict):
            continue
        kind = event.get("type")
        if kind == "item.completed":
            item = event.get("item", {})
            if item.get("type") == "agent_message":
                stream_message = item.get("text")
        elif kind == "turn.completed":
            turns += 1
            reported = event.get("usage", {})
            for key in usage:
                usage[key] += reported.get(key) or 0

    # The --output-last-message file is written only when the agent produces a
    # real final message, via a path separate from the NDJSON stream. So a
    # non-empty file is the authoritative completed-vs-interrupted signal: it
    # survives a frozen stream, and its absence at exit means the run ended
    # without a final report (crashed, killed, or yielded out) - i.e. launch a
    # fresh implementer rather than proceed. This mirrors cmux's completion
    # test (a terminal turn carrying an assistant message) in one file.
    final_message = None
    try:
        with open(last_msg_path) as f:
            final_message = f.read().strip() or None
        os.unlink(last_msg_path)
    except OSError:
        pass
    captured = final_message is not None
    if not final_message:
        final_message = stream_message

    print(f"exit_code: {result.returncode}")
    print(f"final_message_captured: {str(captured).lower()}")
    print(f"turns: {turns}")
    print(f"usage: {usage}")
    if result.stderr.strip():
        print("--- codex stderr ---")
        print(result.stderr.strip())
    if log_lines:
        print(f"--- non-JSON / log lines ({len(log_lines)}) ---")
        for line in log_lines:
            print(line)
    print("--- final agent message ---")
    print(final_message or "(none)")
    return result.returncode
