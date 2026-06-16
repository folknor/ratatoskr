#!/usr/bin/env python3
"""Spec-loop implement role (reference/orchestrate.md): codex gpt-5.5 at
medium, /goal-driven. The falsifiability test of the spec - a medium-effort
implementer working only from the spec.

Usage: codex-implement.py '<one-line prompt, single-quoted>'

The /goal prefix is added here, not by the caller. Prints a clean digest
(final agent message, usage, any log lines). Never resumes: if a run ends
with the goal unmet, launch a fresh codex-implement.py run instead.
"""
import sys

from codex_common import run_codex

if __name__ == "__main__":
    prompt = sys.argv[1] if len(sys.argv) > 1 else ""
    sys.exit(run_codex(prompt, effort="medium", goal=True))
