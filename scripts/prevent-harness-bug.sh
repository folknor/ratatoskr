#!/usr/bin/env bash
# prevent-harness-bug.sh - the "flush" for the spec-loop orchestration.
#
# Recent Claude harness versions leave a completed background agent lingering
# for ~20-30s before it fully tears down. If the orchestrator launches the next
# agent during that window, the new agent is mis-parented INSIDE the lingering
# one: it runs to completion there, the lingering agent's lifetime swallows the
# inner agent's entire runtime, and the inner work bubbles back as a confusing
# second return on the WRONG (outer) agent's id.
#
# Fix: after any agent comes to rest, the orchestrator fires this script as a
# background task, waits for it to exit, and only then launches the next agent.
# The wall-clock delay lets the prior agent fully tear down so the next launch
# lands at top level; the script's exit reliably re-invokes the orchestrator.
# Teardown is wall-clock, so idling through the flush is sufficient. 60s is well
# clear of the measured 20-30s window.
#
# Arg 1 = seconds to sleep (default 60).
sleep "${1:-60}"
