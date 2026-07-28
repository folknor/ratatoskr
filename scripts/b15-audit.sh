#!/usr/bin/env bash
# Re-runnable B15 deletion audit (spec B15f step 2). Exit non-zero when
# legacy provider-transport residue needs an explicit disposition, so this
# stays useful after the NEXT deletion item rather than being a one-shot.
set -euo pipefail

status=0
flag() {
    printf 'B15 audit: %s\n' "$1"
    status=1
}

# (a) Dependencies with a bifrost equivalent.
#
# `bifrost-*` names are the equivalent, not the residue, so they are excluded
# by name rather than by position. `reqwest` is allowed only in the crates
# that own non-provider HTTP plumbing (discovery/CalDAV in core, the shared
# helper in common, AI in ai, OAuth loopback in app, and service itself).
reqwest_allowed='^crates/(app|ai|common|core|service)/Cargo\.toml$'
while IFS= read -r manifest; do
    if rg -n '^[A-Za-z0-9_-]*(async-imap|imap-proto|lettre|smtp|jmap|graph|gmail|imap)[A-Za-z0-9_-]*\s*=' "$manifest" \
        | rg -v '^\s*[0-9]+:bifrost-'; then
        flag "legacy-looking provider dependency in $manifest"
    fi
    if [[ ! $manifest =~ $reqwest_allowed ]] && rg -n '^reqwest\s*=' "$manifest"; then
        flag "reqwest outside the sanctioned HTTP-plumbing crates in $manifest"
    fi
done < <(find crates -mindepth 2 -maxdepth 2 -name Cargo.toml -print | sort)

# (b) Modules whose name or doc claims provider transport duty.
if rg -n -i '(provider transport|legacy provider|imap client|jmap client|graph client|gmail client)' \
    crates --glob '*.rs' --glob '!**/bifrost/**' \
    | rg -v 'legacy provider.*retired|legacy provider-construction'; then
    flag 'provider-transport claim outside the Bifrost implementation'
fi

# (c) `RATATOSKR_TEST_*` consumers whose endpoint no longer exists.
allowed_endpoints='RATATOSKR_TEST_(JMAP|IMAP|SMTP|GRAPH|GMAIL|CALDAV|CARDDAV|PEOPLE|GCAL)_ENDPOINT|RATATOSKR_TEST_DISCOVERY_BASE'
if rg -n -o 'RATATOSKR_TEST_[A-Z_]+' crates --glob '*.rs' \
    | awk -F: '{print $NF}' | sort -u | rg -v "^${allowed_endpoints}$"; then
    flag 'unknown or orphaned test endpoint consumer'
fi

exit "$status"
