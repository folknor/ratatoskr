@AGENTS.md

## More rules

### General rules

- Subagents must always be launched in the foreground, (never use `run_in_background: true`) so the user can approve tool requests.

### Memory rules

Do not use your Memory functionality. Do not read, write, or update memories. Do not suggest saving things to memory. Durable context belongs in CLAUDE.md or the relevant docs, not in per-session memory files - this project is developed across several hosts and users, and memory does not transfer between them; CLAUDE.md does.

### Bash rules

- Never use `sed`, `find`, `awk`, `head`, `tail`, or complex bash commands.
- Never run `git` with `-C <path>`. Run `git` from the current working directory.

## Multi-Agent Orchestration

Do NOT use worktree isolation for parallel agents. Instead, launch agents in the same tree with strict file ownership - zero overlap.

Agent coordination rules:
- Each agent gets exclusive ownership of specific files. No two agents touch the same file.
- Agents must NOT run `cargo` or `brokkr`. The orchestrator validates between agents.
