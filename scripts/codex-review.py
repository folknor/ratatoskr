#!/usr/bin/env python3
"""Spec-loop review role (reference/orchestrate.md): codex gpt-5.5 at xhigh,
no goal. The deepest reasoner in the system, spent critiquing a spec before
any code exists.

Usage: codex-review.py '<one-line prompt, single-quoted>'

Prints a clean digest (final agent message, usage, any log lines). Never
resumes; the raw NDJSON stays inside the process.
"""
import sys

from codex_common import run_codex

if __name__ == "__main__":
    prompt = sys.argv[1] if len(sys.argv) > 1 else ""
    sys.exit(run_codex(prompt, effort="xhigh", goal=False))
