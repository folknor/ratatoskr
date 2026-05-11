#!/usr/bin/env python3
"""Hunt for evidence that any of the remaining dispatch_in_process.rs
tests was ever flaky, hung, or failed unexpectedly across this project's
Claude Code session history.

A naive "near a failure word" filter drowns in false positives, because
cargo test output for a passing run still contains the word "failed"
("0 failed"), the panic-handler test legitimately prints a backtrace,
and `running 16 tests` blocks group every test name together so any
unrelated trouble word lights up every name.

This script narrows to a few high-signal patterns:

1. `test <NAME> ... FAILED`   — cargo says the test failed
2. `failures:` block followed by `<NAME>` within ~200 chars
3. Free-form user prose: `<NAME>` within 200 chars of one of
   { flaky, hangs, hangs sometimes, intermittent, stuck, deadlock,
     hung, timed out, never returns, race } where the local context
   does *not* look like a cargo summary (no `test result:`).

We also count plain `... ok` mentions so we know each test ran.
"""

import json
import re
import sys
from pathlib import Path

PROJECT_DIR = Path.home() / ".claude/projects/-home-folk-Programs-ratatoskr"

TESTS = [
    "malformed_json_returns_error_and_loop_continues",
    "oversized_frame_returns_error_and_loop_continues",
    "eof_on_stdin_exits_cleanly",
    "invalid_utf8_returns_parse_error_and_loop_continues",
    "invalid_request_correlates_error_to_extracted_id",
    "panicking_handler_returns_service_error_panic_and_loop_continues",
    "in_flight_semaphore_caps_concurrent_handlers_and_heartbeat_bypasses",
    "boot_sequence_returns_key_load_failure_when_key_file_is_missing",
    "boot_sequence_returns_migration_failure_when_db_is_corrupt",
]

FAILED_LINE = re.compile(r"test\s+(\w+)\s+\.\.\.\s+FAILED")
OK_LINE     = re.compile(r"test\s+(\w+)\s+\.\.\.\s+ok\b")
IGNORED_LINE = re.compile(r"test\s+(\w+)\s+\.\.\.\s+ignored")

PROSE_TROUBLE = re.compile(
    r"\b(flaky|flake|flakiness|hangs|hung|hanging|stuck|deadlock"
    r"|intermittent|times? out|timed out|never returns|never returned"
    r"|race condition|races with|spurious|nondeterministic"
    r"|broken|breaks|broke)\b",
    re.IGNORECASE,
)

CARGO_SUMMARY = re.compile(r"test result: (ok|FAILED)\.")
RUNNING_BLOCK = re.compile(r"running \d+ tests")


def walk_strings(obj):
    if isinstance(obj, str):
        yield obj
    elif isinstance(obj, dict):
        for v in obj.values():
            yield from walk_strings(v)
    elif isinstance(obj, list):
        for v in obj:
            yield from walk_strings(v)


def session_blobs(path: Path):
    try:
        for raw in path.open("r", encoding="utf-8", errors="replace"):
            raw = raw.strip()
            if not raw:
                continue
            try:
                rec = json.loads(raw)
            except json.JSONDecodeError:
                yield raw
                continue
            for blob in walk_strings(rec):
                yield blob
    except OSError as e:
        print(f"warn: could not read {path}: {e}", file=sys.stderr)


def classify_window(text: str, test: str):
    """Return list of (verdict, snippet) for trouble found in `text`
    that is genuinely about `test`."""
    findings = []

    # 1. FAILED line referring to this test.
    for m in FAILED_LINE.finditer(text):
        if m.group(1) == test:
            lo = max(0, m.start() - 200)
            hi = min(len(text), m.end() + 200)
            findings.append(("FAILED-line", text[lo:hi]))

    # 2. cargo "failures:" block listing this test.
    for m in re.finditer(r"failures:\s*\n(.*?)(?:\n\n|\Z)", text, re.DOTALL):
        if test in m.group(1):
            lo = max(0, m.start() - 200)
            hi = min(len(text), m.end() + 200)
            findings.append(("failures-block", text[lo:hi]))

    # 3. Prose trouble near the test name, NOT inside a cargo
    #    "running N tests" / "test result:" summary.
    for m in re.finditer(re.escape(test), text):
        lo = max(0, m.start() - 250)
        hi = min(len(text), m.end() + 250)
        window = text[lo:hi]
        if not PROSE_TROUBLE.search(window):
            continue
        # Skip cargo summary noise.
        if CARGO_SUMMARY.search(window) or RUNNING_BLOCK.search(window):
            continue
        # The test name itself contains "_continues" which never hits
        # PROSE_TROUBLE — good. We deliberately keep this filter narrow.
        findings.append(("prose", window))

    return findings


def main():
    files = sorted(PROJECT_DIR.glob("*.jsonl"))
    print(f"Scanning {len(files)} session files under {PROJECT_DIR}\n")

    per_test = {t: {"ok": 0, "failed": 0, "ignored": 0,
                    "sessions_ok": set(), "sessions_failed": set(),
                    "prose_hits": [], "fail_hits": []}
                for t in TESTS}

    for path in files:
        try:
            full = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        if not any(t in full for t in TESTS):
            continue

        # Walk each string blob: keeps per-blob context tight enough that
        # `running 16 tests` blocks don't bleed into unrelated prose.
        for blob in session_blobs(path):
            for t in TESTS:
                if t not in blob:
                    continue
                # Counts via line-shape matches.
                for m in OK_LINE.finditer(blob):
                    if m.group(1) == t:
                        per_test[t]["ok"] += 1
                        per_test[t]["sessions_ok"].add(path.name)
                for m in FAILED_LINE.finditer(blob):
                    if m.group(1) == t:
                        per_test[t]["failed"] += 1
                        per_test[t]["sessions_failed"].add(path.name)
                for m in IGNORED_LINE.finditer(blob):
                    if m.group(1) == t:
                        per_test[t]["ignored"] += 1
                # Free-form trouble.
                for verdict, snippet in classify_window(blob, t):
                    if verdict == "prose":
                        per_test[t]["prose_hits"].append((path.name, snippet))
                    else:
                        per_test[t]["fail_hits"].append((path.name, verdict, snippet))

    print("=" * 78)
    print("Per-test pass/fail counts across all session transcripts")
    print("=" * 78)
    print(f"{'test':<70} {'ok':>5} {'FAIL':>5} {'ign':>5}")
    for t in TESTS:
        s = per_test[t]
        print(f"{t:<70} {s['ok']:>5} {s['failed']:>5} {s['ignored']:>5}")

    print()
    print("=" * 78)
    print("Real failures (test ... FAILED + cargo failures: blocks)")
    print("=" * 78)
    any_fail = False
    for t in TESTS:
        hits = per_test[t]["fail_hits"]
        if not hits:
            continue
        any_fail = True
        print(f"\n{t}: {len(hits)} hit(s)")
        for fname, verdict, snippet in hits[:3]:
            head = snippet.replace("\n", " ⏎ ")
            if len(head) > 500:
                head = head[:500] + "…"
            print(f"  [{verdict} in {fname}] {head}")
        if len(hits) > 3:
            print(f"  … and {len(hits) - 3} more")
    if not any_fail:
        print("\nNone.")

    print()
    print("=" * 78)
    print("Prose trouble (user/assistant text near test name)")
    print("=" * 78)
    any_prose = False
    for t in TESTS:
        hits = per_test[t]["prose_hits"]
        if not hits:
            continue
        any_prose = True
        # Deduplicate by snippet to cut conversation-replay noise.
        seen = set()
        unique = []
        for fname, snippet in hits:
            key = snippet.strip()[:200]
            if key in seen:
                continue
            seen.add(key)
            unique.append((fname, snippet))
        print(f"\n{t}: {len(unique)} unique snippet(s) ({len(hits)} total with replays)")
        for fname, snippet in unique[:5]:
            head = snippet.replace("\n", " ⏎ ")
            if len(head) > 600:
                head = head[:600] + "…"
            print(f"  [{fname}] {head}")
        if len(unique) > 5:
            print(f"  … and {len(unique) - 5} more unique")
    if not any_prose:
        print("\nNone.")


if __name__ == "__main__":
    main()
