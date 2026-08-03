use std::collections::HashSet;
use std::sync::Arc;

use bifrost_types::{AccountFactory, AccountId, Calendar, CalendarId, EventRange, EventTime};
use rusqlite::{Row, params};
use service_state::WriteDbState;
use tokio_util::sync::CancellationToken;

use db::db::queries_extra::calendar_contacts_writes::{
    reap_expired_unlisted_calendars, stamp_unlisted_calendars, sync_caldav_attendees,
    sync_caldav_reminders, upsert_calendar_event_row, upsert_discovered_calendar,
};
use db::db::queries_extra::calendars::recurring_master_intersects_window;
use rtsk::db::ReadDbState;
use rtsk::db::types::DbCalendar;

use super::idmap;

const WINDOW_BACK_MS: i64 = 365 * 24 * 60 * 60 * 1000;
const WINDOW_FORWARD_MS: i64 = 730 * 24 * 60 * 60 * 1000;
const HISTORY_SLICE_MS: i64 = 365 * 24 * 60 * 60 * 1000;
const REAP_THRESHOLD_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Outcome of a calendar sync run.
///
/// `mutated` tracks whether the run wrote anything to the calendar tables
/// before terminating. `CalendarRuntime` reads this independently of
/// `result`: a partial-commit failure (provider error mid-loop after the
/// discovered-calendars upsert succeeded) must still drive a UI reload.
/// Conditioning emission on `result.is_ok()` would leave the UI stale
/// after exactly that case.
pub struct CalendarSyncOutcome {
    pub mutated: bool,
    pub result: Result<(), String>,
}

pub async fn calendar_sync_account_impl(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    factory: Arc<dyn AccountFactory>,
    now_ms: i64,
    cancellation_token: &CancellationToken,
) -> CalendarSyncOutcome {
    if cancellation_token.is_cancelled() {
        return CalendarSyncOutcome {
            mutated: false,
            result: Err("calendar sync cancelled".to_string()),
        };
    }
    let mut mutated = false;
    let result = sync_bifrost_calendar_account(
        account_id,
        write_db,
        read_db,
        factory,
        now_ms,
        cancellation_token,
        &mut mutated,
    )
    .await;

    CalendarSyncOutcome { mutated, result }
}

async fn sync_bifrost_calendar_account(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    factory: Arc<dyn AccountFactory>,
    now_ms: i64,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    let opened = factory
        .open(AccountId(account_id.to_string()))
        .await
        .map_err(|error| error.to_string())?;
    // `AccountFactory::open` now returns an `OpenedAccount` envelope rather than
    // a bare handle. `skipped_scopes` names surface the open discovered but could
    // not bring up (a shared calendar whose probe failed); it is not consulted
    // here because calendar sync targets are re-derived from `calendars_list`
    // below, which already omits anything that did not open.
    let account = opened.account;
    if !account.capabilities().pim_methods.calendars_list
        || !account.capabilities().pim_methods.events_in_range
    {
        return Ok(());
    }
    let calendars = account
        .calendars_list()
        .await
        .map_err(|error| error.to_string())?;
    let account_owned = account_id.to_string();
    let discovered = calendars.clone();
    // O20: gate `mutated` on rows ACTUALLY changed, not on "an upsert ran".
    // Under the full-window pull the same calendars are re-discovered every
    // hourly kick, so an unconditional flag would fire `CalendarChanged` every
    // hour and churn the app's calendar view. `upsert_discovered_calendar`
    // reports whether the metadata genuinely changed.
    let calendars_changed = write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|error| error.to_string())?;
            let mut changed = false;
            for calendar in &discovered {
                let (_, row_changed) = upsert_discovered_calendar(
                    &tx,
                    &idmap::to_discovered_calendar(&account_owned, calendar),
                )?;
                changed |= row_changed;
            }
            let listed: Vec<String> = discovered
                .iter()
                .map(|calendar| idmap::calendar_remote_id(calendar).to_string())
                .collect();
            // A successful empty list is not strong enough evidence that every
            // cached calendar disappeared, so it neither hides nor reaps rows.
            if !listed.is_empty() {
                let stamped = stamp_unlisted_calendars(&tx, &account_owned, &listed, now_ms)?;
                let reaped = reap_expired_unlisted_calendars(
                    &tx,
                    &account_owned,
                    now_ms,
                    REAP_THRESHOLD_MS,
                )?;
                changed |= stamped > 0 || reaped > 0;
            }
            tx.commit().map_err(|error| error.to_string())?;
            Ok(changed)
        })
        .await?;
    *mutated |= calendars_changed;

    // Resolve the listed-and-visible intersection to (local id, remote id)
    // pairs up front so the active-window pass and the history backfill pass
    // can each iterate the full set (§ 4.2: every step-6 active-window sync
    // precedes any step-7 backfill - freshness before archaeology).
    let visible = load_visible_calendars(read_db, account_id).await?;
    let targets = resolve_sync_targets(&calendars, visible);

    // Step 6: active-window pull + windowed reconcile for every calendar. A
    // page-fetch error aborts the run before any reconcile (§ 4.5: never
    // delete on a transient failure), so this pass stays fail-fast.
    for (calendar_id, remote_calendar_id) in &targets {
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let pulled = pull_range(
            account.as_ref(),
            PullRange {
                account_id,
                write_db,
                calendar_id,
                remote_calendar_id: remote_calendar_id.clone(),
                start: now_ms - WINDOW_BACK_MS,
                end: now_ms + WINDOW_FORWARD_MS,
                cancellation_token,
            },
            mutated,
        )
        .await?;
        let deleted = reconcile_window(
            write_db,
            calendar_id,
            (now_ms - WINDOW_BACK_MS).div_euclid(1000),
            (now_ms + WINDOW_FORWARD_MS).div_euclid(1000),
            &pulled.seen,
            &pulled.failed,
        )
        .await?;
        *mutated |= deleted > 0;
    }

    // Step 7: one-time per-calendar history backfill, AFTER every calendar's
    // active-window work has committed. A backfill failure for one calendar is
    // recorded but must NOT abort the remaining calendars' backfill nor undo
    // the committed active-window work (§ 4.2); the first such error surfaces
    // as the run's `result` after the loop, leaving `history_backfilled_at`
    // NULL for an idempotent retry next kick.
    let mut backfill_error: Option<String> = None;
    for (calendar_id, remote_calendar_id) in &targets {
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        match backfill_history(
            Backfill {
                account: account.as_ref(),
                account_id,
                write_db,
                read_db,
                calendar_id,
                remote_calendar_id: remote_calendar_id.clone(),
                now_ms,
                cancellation_token,
            },
            mutated,
        )
        .await
        {
            Ok(()) => {}
            Err(error) if error == "calendar sync cancelled" => return Err(error),
            Err(error) => {
                log::warn!("calendar history backfill failed for {calendar_id}: {error}");
                if backfill_error.is_none() {
                    backfill_error = Some(error);
                }
            }
        }
    }
    if let Some(error) = backfill_error {
        return Err(error);
    }
    Ok(())
}

/// Resolve the `(local calendar id, remote calendar id)` pairs to sync this
/// run: the intersection of the user-visible calendars with the calendars the
/// current backend actually returned from `calendars_list()`.
///
/// This is the O17 stale-calendar guard. `load_visible_calendars` is
/// provider-BLIND (it filters on `account_id` + `is_visible` only), so after
/// the O10 precedence fix reroutes an account's calendar backend the cache can
/// still hold visible rows minted by the OLD backend (e.g. a Google
/// `calendarId` on an account now routed to CalDAV). Feeding such a stale
/// remote id to the new backend's `events_in_range` would error and abort the
/// run. B7a's policy (spec § 4.2 / O17) is RETAIN-AND-SKIP: a calendar the
/// current backend no longer lists is RETAINED (its cached events keep
/// rendering per `calendars.is_visible`) but excluded from the fetch here - it
/// produces no target, so no `events_in_range` is issued against the wrong
/// backend. The reap-vs-hide lifecycle is deferred to B7c.
fn resolve_sync_targets(
    calendars: &[Calendar],
    visible: Vec<DbCalendar>,
) -> Vec<(String, CalendarId)> {
    let listed: HashSet<&str> = calendars.iter().map(idmap::calendar_remote_id).collect();
    visible
        .into_iter()
        .filter(|calendar| listed.contains(calendar.remote_id.as_str()))
        .filter_map(|calendar| {
            calendars
                .iter()
                .find(|source| idmap::calendar_remote_id(source) == calendar.remote_id)
                .map(|source| (calendar.id, source.id.clone()))
        })
        .collect()
}

/// Inputs for one calendar's history backfill. Bundled to keep
/// `backfill_history` under the `too_many_arguments` lint cap (mirrors
/// `PullRange`).
struct Backfill<'a> {
    account: &'a dyn bifrost_types::Account,
    account_id: &'a str,
    write_db: &'a WriteDbState,
    read_db: &'a ReadDbState,
    calendar_id: &'a str,
    remote_calendar_id: bifrost_types::CalendarId,
    now_ms: i64,
    cancellation_token: &'a CancellationToken,
}

/// One-time per-calendar history backfill (§ 2.6): pull `[now - 1825d,
/// now - 365d)` as four consecutive 365-day upsert-only slices (oldest
/// first), then stamp `history_backfilled_at`. No deletion reconcile runs
/// over backfill ranges (O22): out-of-window rows are never deletion
/// candidates. `mutated` accumulates only genuine row changes (O20).
async fn backfill_history(args: Backfill<'_>, mutated: &mut bool) -> Result<(), String> {
    if calendar_history_backfilled_at(args.read_db, args.calendar_id)
        .await?
        .is_some()
    {
        return Ok(());
    }
    for slice in 0..4 {
        if args.cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let start = args.now_ms - (5 - slice) * HISTORY_SLICE_MS;
        let end = start + HISTORY_SLICE_MS;
        pull_range(
            args.account,
            PullRange {
                account_id: args.account_id,
                write_db: args.write_db,
                calendar_id: args.calendar_id,
                remote_calendar_id: args.remote_calendar_id.clone(),
                start,
                end,
                cancellation_token: args.cancellation_token,
            },
            mutated,
        )
        .await?;
    }
    mark_history_backfilled(args.write_db, args.calendar_id, args.now_ms).await?;
    Ok(())
}

struct PullRange<'a> {
    account_id: &'a str,
    write_db: &'a WriteDbState,
    calendar_id: &'a str,
    remote_calendar_id: bifrost_types::CalendarId,
    start: i64,
    end: i64,
    cancellation_token: &'a CancellationToken,
}

/// Result of pulling one calendar's window: the dedup keys observed across all
/// pages, and the provider-native ids of resources the provider fetched but
/// could not materialize (`Page::failed_ids`, populated by CalDAV per-resource
/// parse/fetch failures). The reconcile subtracts `failed` from the absence
/// computation so a transiently-failing resource is preserved rather than
/// mistaken for a remote deletion.
struct PullResult {
    seen: HashSet<String>,
    failed: HashSet<String>,
}

async fn pull_range(
    account: &dyn bifrost_types::Account,
    range: PullRange<'_>,
    mutated: &mut bool,
) -> Result<PullResult, String> {
    let mut cursor = None;
    let mut seen = HashSet::new();
    let mut failed = HashSet::new();
    loop {
        if range.cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let page = account
            .events_in_range(EventRange {
                calendar_id: range.remote_calendar_id.clone(),
                start: epoch_time(range.start),
                end: epoch_time(range.end),
                page_cursor: cursor,
                limit: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        failed.extend(page.failed_ids);
        let events = page.items;
        for event in &events {
            seen.insert(idmap::event_dedup_key(event));
        }
        let changed =
            apply_event_page(range.write_db, range.account_id, range.calendar_id, events).await?;
        // O20: flag mutation only when a row was actually inserted or its
        // content changed, not merely because events were seen. The
        // full-window pull re-fetches unchanged rows on every kick.
        *mutated |= changed;
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(PullResult { seen, failed })
}

fn epoch_time(ms: i64) -> EventTime {
    let seconds = ms.div_euclid(1000);
    EventTime {
        value: jiff::Timestamp::from_second(seconds).map_or_else(
            |_| "1970-01-01T00:00:00Z".to_string(),
            |value| value.to_zoned(jiff::tz::TimeZone::UTC).to_string(),
        ),
        timezone: Some("UTC".to_string()),
    }
}

/// Upsert one page of events (plus attendees/reminders) in a single txn.
/// Returns whether any event row was inserted or had its content change
/// (O20): the caller uses this to gate `mutated` so `CalendarChanged` fires
/// only on genuine change under the full-window pull.
async fn apply_event_page(
    write_db: &WriteDbState,
    account_id: &str,
    calendar_id: &str,
    events: Vec<bifrost_types::CalendarEvent>,
) -> Result<bool, String> {
    let account_id = account_id.to_string();
    let calendar_id = calendar_id.to_string();
    write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|error| error.to_string())?;
            let mut changed = false;
            for event in events {
                let Some(row) = idmap::to_event_row(&account_id, &calendar_id, &event) else {
                    continue;
                };
                let key = row.google_event_id.clone();
                let (_, row_changed) = upsert_calendar_event_row(&tx, &row)?;
                changed |= row_changed;
                let attendees = idmap::to_attendees(&event);
                sync_caldav_attendees(
                    &tx,
                    &account_id,
                    &key,
                    &attendees,
                    row.organizer_email.as_deref(),
                    row.organizer_name.as_deref(),
                )?;
                let reminders = idmap::to_reminders(&event);
                sync_caldav_reminders(&tx, &account_id, &key, &reminders)?;
            }
            tx.commit().map_err(|error| error.to_string())?;
            Ok(changed)
        })
        .await
}

/// Windowed deletion reconcile (§ 4.5). Returns the number of events deleted
/// so the caller can gate `mutated` on real deletions (O20).
async fn reconcile_window(
    write_db: &WriteDbState,
    calendar_id: &str,
    start: i64,
    end: i64,
    seen: &HashSet<String>,
    failed: &HashSet<String>,
) -> Result<usize, String> {
    // Empty-pull guard (O16): a successful pull that returned zero events
    // against a non-empty in-window cache is a suspected transient failure -
    // skip the delete step rather than mass-delete. `full_resync` remains the
    // intentional force-clear path.
    if seen.is_empty() {
        return Ok(0);
    }
    let calendar_id = calendar_id.to_string();
    let seen = seen.clone();
    let failed = failed.clone();
    write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|error| error.to_string())?;
            // Overlap + recurring-master candidate set (O21), mirroring the
            // view query (`calendars/view/mod.rs`): a recurring master is
            // loaded regardless of its own DTSTART position because its
            // in-window instances are what the load-path RRULE expansion
            // renders. The occurrence-intersection decision below (FORK 6)
            // then decides whether an absent master is a real delete.
            let mut stmt = tx
                .prepare(
                    "SELECT id, google_event_id, remote_event_id, start_time, end_time, \
                            recurrence_rule, timezone FROM calendar_events \
                     WHERE calendar_id = ?1 \
                       AND (recurrence_rule IS NOT NULL OR (start_time < ?2 AND end_time > ?3))",
                )
                .map_err(|error| error.to_string())?;
            let candidates = stmt
                .query_map(params![calendar_id, end, start], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                })
                .map_err(|error| error.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            drop(stmt);
            let mut deleted = 0usize;
            for (id, key, remote_event_id, start_time, end_time, recurrence_rule, timezone) in
                candidates
            {
                // Preserve a row still present in the pull, and preserve a row
                // whose backing resource the provider reported as FAILED this
                // run (CalDAV per-resource 207 failure, SQ-4) - its absence
                // from `seen` is a transient fetch/parse failure, not a remote
                // delete. Only a row that is absent AND not failed is a true
                // deletion candidate.
                if seen.contains(&key) {
                    continue;
                }
                if let Some(remote_event_id) = &remote_event_id
                    && failed.contains(remote_event_id)
                {
                    continue;
                }
                // FORK 6 (O21): an absent recurring MASTER whose occurrences
                // all fall outside the active window is legitimately unseen by
                // a range pull and must be PRESERVED, not deleted.
                if !should_delete_absent_candidate(
                    recurrence_rule.as_deref(),
                    start_time,
                    end_time,
                    timezone.as_deref(),
                    start,
                    end,
                ) {
                    continue;
                }
                tx.execute(
                    "DELETE FROM calendar_attendees WHERE event_id = ?1",
                    params![id],
                )
                .map_err(|error| error.to_string())?;
                tx.execute(
                    "DELETE FROM calendar_reminders WHERE event_id = ?1",
                    params![id],
                )
                .map_err(|error| error.to_string())?;
                tx.execute("DELETE FROM calendar_events WHERE id = ?1", params![id])
                    .map_err(|error| error.to_string())?;
                deleted += 1;
            }
            tx.commit().map_err(|error| error.to_string())?;
            Ok(deleted)
        })
        .await
}

/// Decide whether an absent-from-seen (and not-failed) cached candidate is a
/// genuine remote deletion to reap, or a recurring MASTER whose occurrences
/// all fall outside the active window and must be PRESERVED.
///
/// FORK 6 (spec § 4.5, O21): the range pull (JMAP `CalendarEvent/query`,
/// CalDAV time-range `REPORT`) matches events by occurrence expansion and
/// omits an ended / out-of-window series, so such a master is legitimately
/// absent from the seen set. Deleting it would discard exactly the multi-year
/// history the § 2.6 backfill exists to build. Google/Graph store expanded
/// instances with `recurrence_rule` NULL (no master rows), so they never take
/// the recurring branch. A non-recurring absent candidate is a real delete
/// (the candidate SQL only loads non-recurring rows that already overlap the
/// window). A recurring master is a real delete ONLY when it still yields an
/// occurrence intersecting the window - so a genuinely cancelled long-running
/// series does not keep rendering in the visible view forever.
fn should_delete_absent_candidate(
    recurrence_rule: Option<&str>,
    start_time: i64,
    end_time: i64,
    timezone: Option<&str>,
    window_start: i64,
    window_end: i64,
) -> bool {
    let Some(rrule) = recurrence_rule else {
        // Non-recurring: the candidate SQL guarantees window overlap already.
        return true;
    };
    // Fast path: the master's own interval overlaps the window (it has, or is,
    // an in-window occurrence) - a real delete without expanding the rule.
    if start_time < window_end && end_time > window_start {
        return true;
    }
    // Otherwise expand the RRULE over the window and delete only if some
    // occurrence lands inside it; else preserve (out-of-window history the
    // range pull cannot see).
    recurring_master_intersects_window(
        rrule,
        start_time,
        end_time,
        timezone,
        window_start,
        window_end,
    )
}

async fn calendar_history_backfilled_at(
    db: &ReadDbState,
    calendar_id: &str,
) -> Result<Option<i64>, String> {
    let calendar_id = calendar_id.to_string();
    db.with_read(move |conn| {
        conn.query_row(
            "SELECT history_backfilled_at FROM calendars WHERE id = ?1",
            params![calendar_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())
    })
    .await
}

async fn mark_history_backfilled(
    db: &WriteDbState,
    calendar_id: &str,
    now_ms: i64,
) -> Result<(), String> {
    let calendar_id = calendar_id.to_string();
    db.with_write(move |conn| {
        conn.execute(
            "UPDATE calendars SET history_backfilled_at = ?1 WHERE id = ?2",
            params![now_ms, calendar_id],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .await
}

pub async fn load_visible_calendars(
    db: &ReadDbState,
    account_id: &str,
) -> Result<Vec<DbCalendar>, String> {
    let account_id = account_id.to_string();
    db.with_read_mapped(
        move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, provider, remote_id, display_name, color, \
                            is_primary, is_visible, sync_token, ctag, created_at, \
                            updated_at, sort_order, is_default, provider_id, can_edit \
                     FROM calendars WHERE account_id = ?1 AND is_visible = 1 \
                     AND unlisted_since IS NULL \
                     ORDER BY is_primary DESC, display_name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_db_calendar)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        },
        |e| e,
    )
    .await
}

fn row_to_db_calendar(row: &Row<'_>) -> rusqlite::Result<DbCalendar> {
    Ok(DbCalendar {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        provider: row.get("provider")?,
        remote_id: row.get("remote_id")?,
        display_name: row.get("display_name")?,
        color: row.get("color")?,
        is_primary: row.get("is_primary")?,
        is_visible: row.get("is_visible")?,
        sync_token: row.get("sync_token")?,
        ctag: row.get("ctag")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        sort_order: row.get("sort_order")?,
        is_default: row.get("is_default")?,
        provider_id: row.get("provider_id")?,
        can_edit: row.get("can_edit")?,
    })
}

#[cfg(test)]
mod tests {
    use super::{resolve_sync_targets, should_delete_absent_candidate};
    use bifrost_types::{Calendar, CalendarId, CalendarProvenance, ProtocolKind};
    use rtsk::db::types::DbCalendar;

    const DAY: i64 = 86_400;
    const NOW: i64 = 1_700_000_000;

    fn bifrost_calendar(provider: ProtocolKind, native_id: &str) -> Calendar {
        Calendar {
            id: CalendarId(native_id.to_string()),
            native_id: native_id.to_string(),
            name: native_id.to_string(),
            color: None,
            provenance: CalendarProvenance {
                provider,
                native: native_id.to_string(),
                calendar_native: None,
            },
            is_default: false,
            can_create_events: true,
            can_update_events: true,
            can_delete_events: true,
        }
    }

    fn db_calendar(remote_id: &str) -> DbCalendar {
        DbCalendar {
            id: format!("local-{remote_id}"),
            account_id: "acct".to_string(),
            provider: "caldav".to_string(),
            remote_id: remote_id.to_string(),
            display_name: Some(remote_id.to_string()),
            color: None,
            is_primary: 0,
            is_visible: 1,
            sync_token: None,
            ctag: None,
            created_at: 0,
            updated_at: 0,
            sort_order: 0,
            is_default: 0,
            provider_id: None,
            can_edit: 1,
        }
    }

    // O17: a visible calendar the current backend no longer lists (e.g. a
    // stale Google calendarId left over on an account now routed to CalDAV)
    // must NOT become a sync target, so no `events_in_range` is issued against
    // the wrong backend. The listed CalDAV calendar still syncs.
    #[test]
    fn stale_unlisted_calendar_is_not_fetched() {
        let listed = vec![bifrost_calendar(
            ProtocolKind::CalDav,
            "https://dav/home/work/",
        )];
        let visible = vec![
            db_calendar("https://dav/home/work/"), // listed by the CalDAV backend
            db_calendar("google-calendar-id-123"), // stale Google row, unlisted now
        ];
        let targets = resolve_sync_targets(&listed, visible);
        assert_eq!(
            targets,
            vec![(
                "local-https://dav/home/work/".to_string(),
                CalendarId("https://dav/home/work/".to_string()),
            )],
            "only the listed CalDAV calendar is a fetch target; the stale Google row is skipped"
        );
    }

    fn window_start() -> i64 {
        NOW - 365 * DAY
    }
    fn window_end() -> i64 {
        NOW + 730 * DAY
    }

    fn ical_utc(ts: i64) -> String {
        jiff::Timestamp::from_second(ts)
            .expect("valid timestamp")
            .to_zoned(jiff::tz::TimeZone::UTC)
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string()
    }

    // FORK 6 case (a): an ended series (UNTIL two years ago) whose master is
    // absent from the pull's seen set is PRESERVED - its occurrences all fall
    // before the active window, so the range pull legitimately omits it.
    #[test]
    fn ended_series_absent_is_preserved() {
        let dtstart = NOW - 3 * 365 * DAY;
        let until = NOW - 2 * 365 * DAY; // before window_start (one year ago)
        let rrule = format!("FREQ=WEEKLY;UNTIL={}", ical_utc(until));
        assert!(
            !should_delete_absent_candidate(
                Some(&rrule),
                dtstart,
                dtstart + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "ended out-of-window series must be preserved, not deleted"
        );
    }

    // FORK 6 case (b): an unbounded weekly series started two years ago, absent
    // from the seen set, is DELETED - its occurrences continue into the active
    // window, so a still-live series would have been returned by the pull; its
    // absence is a genuine remote delete.
    #[test]
    fn unbounded_series_still_live_absent_is_deleted() {
        let dtstart = NOW - 2 * 365 * DAY;
        assert!(
            should_delete_absent_candidate(
                Some("FREQ=WEEKLY"),
                dtstart,
                dtstart + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "a live in-window recurring series absent from the pull is a real delete"
        );
    }

    // FORK 6 regression (the reconcile's own bug): an unbounded weekly series
    // whose DTSTART sits FAR before the active window (four years back, well
    // beyond the DTSTART-anchored 2-year synthetic expansion horizon the view
    // expander uses) still recurs into the window and, when absent from the
    // seen set, must be DELETED. The earlier implementation expanded a fixed
    // count from DTSTART and never reached the window, wrongly preserving a
    // remotely-deleted series forever. Mirrors the
    // caldav-calendar-recurrence-window-reconcile.lua fixture (DTSTART 2024-01
    // vs a 2027-era window).
    #[test]
    fn far_dtstart_unbounded_series_still_live_absent_is_deleted() {
        let dtstart = NOW - 4 * 365 * DAY;
        assert!(
            should_delete_absent_candidate(
                Some("FREQ=WEEKLY"),
                dtstart,
                dtstart + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "an unbounded series with a far-past DTSTART still lands occurrences in \
             the window; its absence from the pull is a real delete"
        );
    }

    // Companion to the far-DTSTART case: an ended series whose UNTIL predates
    // the window must STAY preserved even with a far-past DTSTART (no occurrence
    // reaches the window). Guards against an over-eager reconcile that would
    // delete on absence regardless of the rule's actual reach.
    #[test]
    fn far_dtstart_ended_series_absent_is_preserved() {
        let dtstart = NOW - 4 * 365 * DAY;
        let until = NOW - 2 * 365 * DAY; // still a year before window_start
        let rrule = format!("FREQ=WEEKLY;UNTIL={}", ical_utc(until));
        assert!(
            !should_delete_absent_candidate(
                Some(&rrule),
                dtstart,
                dtstart + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "a far-past ended series whose occurrences all predate the window is \
             preserved, not deleted"
        );
    }

    // FORK 6 case (c): a series whose DTSTART is beyond window_end, absent from
    // the seen set, is PRESERVED - none of its occurrences intersect the active
    // window yet, so the range pull cannot have returned it.
    #[test]
    fn future_series_beyond_window_absent_is_preserved() {
        let dtstart = window_end() + 30 * DAY;
        assert!(
            !should_delete_absent_candidate(
                Some("FREQ=WEEKLY"),
                dtstart,
                dtstart + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "a wholly-future recurring series must be preserved, not deleted"
        );
    }

    // FORK 6 case (d): a non-recurring in-window row absent from the seen set is
    // still a genuine remote delete.
    #[test]
    fn non_recurring_in_window_absent_is_deleted() {
        assert!(
            should_delete_absent_candidate(
                None,
                NOW,
                NOW + 3600,
                None,
                window_start(),
                window_end(),
            ),
            "a non-recurring in-window row absent from the pull is a real delete"
        );
    }
}
