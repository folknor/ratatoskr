#!/usr/bin/env sh
# Bridge: promote bifrost work staged in research/bifrost to the Cargo
# dependency path (../bifrost).
#
# The spec-loop edits bifrost ONLY in ratatoskr/research/bifrost - inside the
# harness root, so Opus agents (and the orchestrator) can write there.
# research/bifrost and ../bifrost are two clones of the same repo
# (github.com/folknor/bifrost); the migration keeps them at one shared commit
# (see docs/bifrost-migration.md section 11). This script is the re-sync: it
# pushes the staged commit from research/bifrost and pulls it into ../bifrost,
# which is what Cargo actually builds against.
#
# Orchestrator-only. It round-trips through GitHub (network), so it can NEVER
# run inside a codex step (codex's sandbox is network-isolated). Committing in
# research/bifrost is also the orchestrator's job; this script assumes the work
# is already committed there.
#
# The ../bifrost HEAD printed at the end is the frozen reference the
# orchestrator records for the in-flight item (section 11).
set -e

cd /home/folk/Programs/ratatoskr/research/bifrost
git push

cd /home/folk/Programs/bifrost
git pull

echo "bifrost now frozen at:"
git rev-parse HEAD
