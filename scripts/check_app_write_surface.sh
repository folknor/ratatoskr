#!/usr/bin/env bash
#
# check_app_write_surface.sh
#
# Phase 6a/6c/6d lockdown gate. Fails if `crates/app/src/` references
# any writable-connection symbol. After Phase 6d-A there is no
# allow-listed escape hatch: every UI write surface routes through a
# Service IPC. The previous `phase_6c_pending_write_state` accessor
# (the contacts pipeline's last UI-side write site) was deleted
# alongside the `app.action_ctx` field.
#
# What is forbidden in `crates/app/src/`:
#   - `Db::with_write_conn`            (deleted in 6a-part-2)
#   - `Db::with_write_conn_sync`       (deleted in 6a-part-2)
#   - `Db::write_db_state`             (deleted in 6a-part-2)
#   - `Db::phase_6c_pending_write_state` (deleted in 6d-A)
#   - `service_state::WriteDbState`
#   - `WriteDbState::from_arc`
#
# Exit codes:
#   0  No forbidden references in crates/app/src/.
#   1  At least one forbidden reference; the offending line(s) are
#      printed to stderr.
#
# Usage: invoked from CI / brokkr; can be run by hand at the repo
# root: `bash scripts/check_app_write_surface.sh`.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
APP_SRC="$REPO_ROOT/crates/app/src"

if [ ! -d "$APP_SRC" ]; then
    echo "error: $APP_SRC does not exist; run from a Ratatoskr checkout" >&2
    exit 2
fi

# Forbidden symbol patterns. Each is matched as an extended regex
# against every line of every .rs file under crates/app/src/.
FORBIDDEN_PATTERNS=(
    'Db::with_write_conn\b'
    'Db::with_write_conn_sync\b'
    'Db::write_db_state\b'
    'Db::phase_6c_pending_write_state\b'
    '\.with_write_conn\('
    '\.with_write_conn_sync\('
    '\.write_db_state\('
    '\.phase_6c_pending_write_state\('
    'service_state::WriteDbState\b'
    'WriteDbState::from_arc\b'
)

failed=0
for pattern in "${FORBIDDEN_PATTERNS[@]}"; do
    # rg --no-messages keeps the script silent on permission /
    # encoding errors that aren't relevant here. -t rust scopes to
    # .rs files. -n prints line numbers for the report.
    #
    # Matches are filtered through grep -v '^[^:]*:[0-9]+:[[:space:]]*//'
    # to drop lines whose first non-whitespace character is a
    # comment marker. The forbidden symbols are referenced by name
    # in doc comments inside connection.rs (the lockdown notice
    # itself documents what was removed); the call-site lockdown
    # only applies to active code.
    if raw=$(rg --no-messages -t rust -n "$pattern" "$APP_SRC" 2>/dev/null); then
        matches=$(printf '%s\n' "$raw" \
            | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
            || true)
        if [ -n "$matches" ]; then
            echo "[check_app_write_surface] forbidden pattern: $pattern" >&2
            echo "$matches" >&2
            failed=1
        fi
    fi
done

if [ "$failed" -ne 0 ]; then
    echo "" >&2
    echo "Phase 6d-A lockdown: writable-connection access from" >&2
    echo "crates/app/src/ is forbidden. Every UI write surface must" >&2
    echo "route through a Service IPC; see" >&2
    echo "docs/service/phase-6d-plan.md and docs/service/phase-6a-plan.md." >&2
    exit 1
fi

exit 0
