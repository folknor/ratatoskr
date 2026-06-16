@AGENTS.md

## More rules

### The spec-loop

- When the user asks to orchestrate, to run the loop, or to work a goal down to landed commits, read `reference/orchestrate.md` FIRST and follow it exactly - it is the standing procedure (roles, the seven steps, the waiting discipline, codex invocation).
- Note its Input section: confirm the goal with the user before launching anything.
- The orchestrate.md workflow, once invoked, overrides the foreground-subagent rule below (its launches are background by design, per the user's standing instruction in that document).

### General rules

- Never suggest or use the `Workflow` tool (multi-agent orchestration / "ultracode"). Ratatoskr orchestration goes through the spec-loop in `reference/orchestrate.md`.
- Always get permission from the user before launching subagents outside the orchestration loop.
- Subagents must always be launched in the foreground, (never use `run_in_background: true`) so the user can approve tool requests.

### Memory rules

Do not use your Memory functionality. Do not read, write, or update memories. Do not suggest saving things to memory. Durable context belongs in CLAUDE.md or the relevant docs.

### Bash rules

- Never use `sed`, `find`, `awk`, `head`, `tail`, or complex bash commands.
- Never `find /`.
- Never run `git` with `-C <path>`
- One Bash() invocation === one command

## Multi-Agent Orchestration

Do NOT use worktree isolation for parallel agents. Instead, launch agents in the same tree with strict file ownership - zero overlap.

Agent coordination rules:
- Each agent gets exclusive ownership of specific files. No two agents touch the same file.
- Agents must NOT run `cargo` or `brokkr`. The orchestrator validates between agents.

## Subagent prompt rules

- Every Claude subagent prompt MUST explicitly forbid the agent from launching its own sub-agents. A latent Claude-harness bug mis-parents nested agents and can deep-freeze the whole session. Codex agents (separate harness) are exempt. This is the one deliberate exception to "don't restate inherited rules" below - state it every time.
- Scope the investigation, not the report. Caps like "under 1500 chars" or "max 15 findings" throw away signal you asked them to surface.
- Invite lateral findings up front. If they notice a bug, optimization, smell, or anything surprising while doing the scoped work, they should flag it, even when it's outside the immediate task.
- Name the question, not the method. Don't prescribe tools ("use `git diff`", "use `Read`"), don't prescribe steps ("read in full, not just hunks"), don't enumerate files when the scope already implies them (the `sync` crate only, plus the agent's own `ls` / `git diff --name-only`, is enough). Prescribing the method wastes tokens and signals distrust.
- Don't restate rules the agent already inherits. Subagents load the same CLAUDE.md / AGENTS.md as the main session, so the bash rules, no-cargo, no-worktrees, gremlins, etc. are already in scope. Re-listing them is noise.
