#!/usr/bin/env sh
# Bridge: promote saehrimnir (mock-server) work staged in research/saehrimnir
# to the installed mock binary the sync-harness runs against.
#
# The spec-loop edits saehrimnir ONLY in ratatoskr/research/saehrimnir - inside
# the harness root, so Opus agents (and the orchestrator) can write there.
# research/saehrimnir and ../sæhrimnir are two clones of the same repo
# (github.com/folknor/saehrimnir). This script pushes the staged commit from
# research/saehrimnir, pulls it into ../sæhrimnir, then reinstalls the
# saehrimnir binary so brokkr's sync-harness picks up the change.
#
# Orchestrator-only. It round-trips through GitHub (network), so it can NEVER
# run inside a codex step (codex's sandbox is network-isolated). Committing in
# research/saehrimnir is also the orchestrator's job; this script assumes the
# work is already committed there.
#
# The cargo install below installs an EXTERNAL tool (saehrimnir), not a
# ratatoskr build - that is why it is raw cargo and not brokkr. Do not "fix" it.
set -e

cd /home/folk/Programs/ratatoskr/research/saehrimnir
git push

cd /home/folk/Programs/sæhrimnir
git pull

cargo install --path /home/folk/Programs/sæhrimnir

echo "saehrimnir reinstalled at:"
git rev-parse HEAD
