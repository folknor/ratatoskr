use std::collections::{HashMap, HashSet};

use rusqlite::params;

use crate::db::ReadConn;

mod rrule;

use rrule::expand_recurrence_with_overrides;

/// A calendar event with resolved calendar color, suitable for view rendering.
//
// reviewed (R3 verified non-issue): `start_time`/`end_time` are i64 seconds
// and the entire pipeline is second-precision. Sub-second iCal DTSTART
// (`19700101T000000.500Z`) is truncated by the parse path that fills these
// in, not by the expander -- RFC 5545 itself uses second precision, so we
// don't owe sub-second support here.
#[derive(Debug, Clone)]
pub struct CalendarViewEvent {
    pub id: String,
    pub title: String,
    pub start_time: i64,
    pub end_time: i64,
    pub all_day: bool,
    pub color: String,
    pub calendar_name: Option<String>,
    pub location: Option<String>,
    pub recurrence_rule: Option<String>,
    pub calendar_id: Option<String>,
    pub account_id: String,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub rsvp_status: Option<String>,
    pub description: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
    pub timezone: Option<String>,
    /// VEVENT UID. The load path uses `(account_id, uid)` to subtract
    /// override slots from the master's RRULE expansion - master and
    /// overrides share UID by construction (RFC 5545 § 3.8.4.4).
    pub uid: Option<String>,
    /// Canonical RECURRENCE-ID for override rows. `None` for master rows.
    /// Carries the wall-clock string from the iCal source (see schema
    /// comment) so the dedup decision is independent of the host's local
    /// zone. Format-equality with the same canonicalization the master
    /// expansion produces is what makes phantom dedup possible.
    pub recurrence_id_canonical: Option<String>,
}

/// Load calendar event rows in the visible window (synchronous, DB-only).
///
/// Pushes the window into the SQL `WHERE` clause for non-recurring rows so
/// the result set scales with what the user is actually viewing rather
/// than with the entire history (the project targets 5+ years of synced
/// data, which the previous "load everything" shape walked on every
/// render). Recurring masters are kept regardless of their start/end
/// because their stored `start_time`/`end_time` reflect the MASTER, not
/// the instance window an RRULE actually covers - excluding them by
/// the master's own bounds would silently drop recurring events whose
/// instances run far past or before the master itself.
///
/// This function only loads. Expansion is deferred to
/// `expand_view_events` (which has no DB dependency), so callers can
/// drop the connection mutex before the CPU-heavy walk. (Round 3 #3.)
pub fn load_view_event_rows_sync(
    conn: &ReadConn<'_>,
    window_start: i64,
    window_end: i64,
) -> Result<Vec<CalendarViewEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.id, e.summary, e.title, e.start_time, e.end_time,
                    e.is_all_day, COALESCE(c.color, '#3498db') AS color,
                    c.display_name AS calendar_name, e.location,
                    e.recurrence_rule, e.calendar_id, e.account_id,
                    e.organizer_name, e.organizer_email, e.rsvp_status,
                    e.description, e.availability, e.visibility, e.timezone,
                    e.uid, e.recurrence_id
             FROM calendar_events e
             LEFT JOIN calendars c
               ON c.account_id = e.account_id AND c.id = e.calendar_id
             WHERE (c.is_visible = 1 OR e.calendar_id IS NULL)
               AND (
                 e.recurrence_rule IS NOT NULL
                 OR (e.start_time < ?1 AND e.end_time > ?2)
               )
             ORDER BY e.start_time ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![window_end, window_start], |row| {
            // Prefer `title` over `summary` (title is the v63 canonical field).
            let title_v63: Option<String> = row.get("title")?;
            let summary: Option<String> = row.get("summary")?;
            let display_title = title_v63.or(summary).unwrap_or_default();
            Ok(CalendarViewEvent {
                id: row.get::<_, String>("id")?,
                title: display_title,
                start_time: row.get("start_time")?,
                end_time: row.get("end_time")?,
                all_day: row.get::<_, i64>("is_all_day")? != 0,
                color: row
                    .get::<_, Option<String>>("color")?
                    .unwrap_or_else(|| "#3498db".to_string()),
                calendar_name: row.get("calendar_name")?,
                location: row.get("location")?,
                recurrence_rule: row.get("recurrence_rule")?,
                calendar_id: row.get("calendar_id")?,
                account_id: row.get("account_id")?,
                organizer_name: row.get("organizer_name")?,
                organizer_email: row.get("organizer_email")?,
                rsvp_status: row.get("rsvp_status")?,
                description: row.get("description")?,
                availability: row.get("availability")?,
                visibility: row.get("visibility")?,
                timezone: row.get("timezone")?,
                uid: row.get("uid")?,
                recurrence_id_canonical: row.get("recurrence_id")?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Expand recurring rows into concrete instances and clip to the visible
/// window. Pure CPU work - no DB access - so it runs off the connection
/// mutex.
///
/// Override rows (RECURRENCE-ID set) are emitted in their own slot, and
/// each master expansion subtracts the overridden slots so a series with
/// one moved instance shows N events rather than N+1. See
/// `expand_recurrence_with_overrides` for the dedup mechanism.
///
/// `window_start` / `window_end` clip the final result so an unbounded
/// recurring rule (no UNTIL) doesn't pour 800+ default-capped instances
/// into the view when the user is only looking at a single week. The
/// expander still produces up to its cap; the clip drops anything
/// outside [window_start, window_end).
pub fn expand_view_events(
    rows: Vec<CalendarViewEvent>,
    window_start: i64,
    window_end: i64,
) -> Vec<CalendarViewEvent> {
    // Build a (account_id, uid) -> set of override canonical strings index
    // before expansion. The master row carries `recurrence_id_canonical =
    // None` and the RRULE; each override row carries its own
    // `recurrence_id_canonical = Some(...)`. While walking the master we
    // canonicalise each candidate timestamp with the same shape and skip
    // it when it lands in this set.
    let mut overrides_by_series: HashMap<(String, String), HashSet<String>> = HashMap::new();
    for ev in &rows {
        if let (Some(uid), Some(canonical)) =
            (ev.uid.as_ref(), ev.recurrence_id_canonical.as_ref())
        {
            overrides_by_series
                .entry((ev.account_id.clone(), uid.clone()))
                .or_default()
                .insert(canonical.clone());
        }
    }

    let mut expanded = Vec::with_capacity(rows.len());
    for ev in rows {
        if let Some(ref rrule) = ev.recurrence_rule {
            let overrides = ev
                .uid
                .as_ref()
                .and_then(|uid| {
                    overrides_by_series.get(&(ev.account_id.clone(), uid.clone()))
                })
                .cloned()
                .unwrap_or_default();
            for inst in expand_recurrence_with_overrides(&ev, rrule, &overrides) {
                // Clip to window: an unbounded rule's expansion can
                // produce instances years before/after the visible
                // range. Keep instances that overlap [start, end).
                if inst.start_time < window_end && inst.end_time > window_start {
                    expanded.push(inst);
                }
            }
        } else {
            expanded.push(ev);
        }
    }
    expanded.sort_by_key(|e| e.start_time);
    expanded
}

/// Compatibility wrapper: loads + expands in one synchronous call. Holds
/// the connection mutex for the full duration. Prefer the
/// `load_view_event_rows_sync` + `expand_view_events` split for hot
/// paths (the iced app's calendar reload runs once per nav refresh and
/// would otherwise block sync workers, IPC, search, and body store on
/// each render).
pub fn load_calendar_events_for_view_sync(
    conn: &ReadConn<'_>,
    window_start: i64,
    window_end: i64,
) -> Result<Vec<CalendarViewEvent>, String> {
    let rows = load_view_event_rows_sync(conn, window_start, window_end)?;
    Ok(expand_view_events(rows, window_start, window_end))
}
