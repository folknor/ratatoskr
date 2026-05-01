# Calendar Code Review Findings

## Open

Findings are grouped by code area.

### `crates/db/src/db/queries_extra/calendars.rs`

#### `expand_recurrence` / load path

9. **Note** [arch/claude notes] - **`expand_recurrence` `wall_duration`
   collapses to 0 when the master event lives entirely inside a DST
   gap.** `DTSTART;TZID=America/New_York:20260308T023000` +
   `DTEND;TZID=...:20260308T033000` - both 02:30 and 03:30 resolve to
   03:30 EDT post-shift, raw `end-start=0`, every recurring instance
   inherits zero-length end. Pre-existing parse-time issue
   (`extract_datetime` resolves both endpoints through the gap), not
   introduced in Round 2; flagged for follow-up.

16. **Note (perf)** [perf/codex Medium implicit, perf/claude reference] -
    **`YEARLY_MAX_STEPS=80_000` is large but no longer a render-path
    concern.** Sparse YEARLY rules
    (`BYMONTH=2;BYMONTHDAY=29;COUNT=10000`) walk up to 40k years before
    the count cap fires; expansion now runs off the connection mutex via
    `expand_view_events`, so this only burns a CPU thread for a single
    pathological row rather than blocking sync workers and IPC.

### `crates/db/src/db/time.rs`

18. **Note (perf)** [perf/claude L17] - **`resolve_through_gap` walks
    1-minute steps, up to ~2880 per resolve.** `:60-107`. For
    Pacific/Apia 2011-12-30 the input lands inside a 1440-minute gap;
    forward walk may probe up to 1440 minutes with a
    `from_local_datetime` each step. ~1441 transition lookups for
    that one resolve. In recurrence loops only fires when an instance
    lands inside a gap (rare). Not a bug; flagged as a known cost.
    Decided: keep the linear walk - simplicity is the right tradeoff
    against the rarity of the gap path.

### `crates/core/src/caldav/client.rs`

38. **Note** [arch/claude notes] - **Weak ETag dropped from `If-Match`
    operational impact.** `:1050-1053`. Correct per RFC 7232 § 2.3.2,
    but on Apache `mod_dav` with `FileETag MTime Size` *every* PUT
    becomes last-write-wins. Worth a CalDAV setup-doc note rather than
    a code change.

39. **Note** [bugs/claude notes] - **`prepare_if_match_etag` converts
    iCloud writes to last-write-wins.** iCloud uses weak ETags
    pervasively. Trade-off acknowledged in doc comment; flag for the
    racing-edits user complaint when it lands.

### Feature work (not yet implemented)

These are deferred items that need real lift, not just polish.

45. **Medium** - **Inverted error signaling in `expand_recurrence`.**
    Malformed rules and rules using unsupported BY-rules now log a
    WARN and return `vec![event.clone()]` (single instance);
    well-formed-but-zero-instance rules (UNTIL in past, BYDAY excludes
    everything) still return `vec![]`. The malformed-vs-zero-instance
    signaling is no longer silent on the malformed side, but the
    caller still can't tell the two apart at the data level.
    Surfacing a "broken rule" indicator to the UI would close this
    fully.

46. **Medium** - **YEARLY+ordinal BYDAY without explicit BYMONTH** now
    WARN-logs and falls back to the master instance instead of
    silently emitting zero instances. Implementing the actual
    year-scope ordinal walk (n-th weekday of the year, walking across
    all 12 months) is the real fix and a follow-up.

47. **Medium (upstream)** - **Empty SUMMARY / DESCRIPTION / LOCATION**
    are still indistinguishable from absent. Local code is ready for
    when calcard surfaces empty values; user-cleared-title support
    requires an upstream calcard change before it can land.

50. **Medium** - **Multi-href delegation home-sets** are collected by
    `extract_hrefs_property` (returns `Vec<String>`), but only the
    first is consumed by the discovery flow. Reaching the rest
    requires plumbing `Vec<String>` through `calendar_home_url`
    (single `Option<String>` today), the persisted DB column, and
    `list_calendars`. Multi-href encounters now log a WARN.

### Test gaps flagged

The Round 3 review flagged four behaviors as untested. Three have
pinning tests in `crates/db/src/db/queries_extra/calendars.rs`
(`count_zero_drops_master_emits_empty`,
`negative_master_duration_does_not_panic`,
`yearly_interval_overflow_terminates_safely`); the fourth (#55,
discover_principal against a redirecting base URL) is closed by the
#30 fix in this round - the base-URL try uses
`propfind_with_final_url` and resolves the principal against the
post-redirect URL. The Round 3 #48 LF-only + folded-line gap is now
covered by `parse_lf_only_line_endings`,
`parse_folded_long_description`, and
`parse_lf_only_with_folded_description` in
`crates/core/src/caldav/parse.rs`.

---

## Verified non-issues

Each of these was checked and found correct in Round 3. The notes are now
inline at the call site as `// reviewed (R3 verified non-issue): ...` so
future passes can attribute them without consulting this doc. Search the
tree for `reviewed (R3` to enumerate.

- `time.rs::resolve_local_to_timestamp` - fixed-offset zones never hit gap/ambiguous arms.
- `calendars.rs::collect_monthly_days` - negative BYMONTHDAY and Feb 29 in non-leap years.
- `calendars.rs::parse_rrule` BYDAY split - empty / leading-trailing commas tolerantly dropped.
- `calendars.rs::collect_monthly_days` - mixed bare + ordinal BYDAY resolves per-entry, caller dedups.
- `calendars.rs::expand_weekly` - complex WEEKLY and MONTHLY-with-ordinal traces verified against dateutil.
- `calendars.rs::CalendarViewEvent` - second-precision i64 timestamps; sub-second DTSTART truncated upstream.
- `caldav/parse.rs::parse_propfind_calendars` - multi-`<propstat>` works via per-prop overwrite + 2xx commit gate.
- `caldav/parse.rs::parse_propfind_events` - ETag opaque-tag content (colons, slashes) round-trips verbatim.
- `caldav/parse.rs` text decoding - `unescape()` handles full XML entity set; CDATA goes through the parallel arm.
