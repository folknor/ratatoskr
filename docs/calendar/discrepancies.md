# Calendar: Spec vs. Code Discrepancies

Audit date: 2026-03-30

---

## Remaining Discrepancies

### High

1. ~~**New event creation appears broken.**~~ ✅ Fixed — editor now has a `pick_list` calendar selector dropdown populated from `state.calendars`. `EventField::CalendarId` updates the draft. Phase C of contract #11 will enforce blocking save when no calendar is selected.

2. ~~**Calendar visibility toggles are mostly cosmetic.**~~ ✅ Fixed — event-loading query now filters by `is_visible = 1`. Side effect of `EventSaved` reuse on toggle still exists but is cosmetic.

3. ~~**Calendar sync never triggered from the app.**~~ ✅ Fixed — `sync_calendars()` wired to SyncTick alongside email sync, pending ops, and GAL refresh. 60s timeout per account. The sync backend exists (`calendar_sync_account_impl()`, provider-specific sync in Graph/Gmail/CalDAV) but the iced app never calls it. Calendar data only appears if seeded externally. The read path works, the sync path works, but they are not connected.

4. **Graph API timezone handling silently treats everything as UTC.** `parse_graph_datetime()` has "Best-effort: treat as UTC" for all non-UTC timezone names. Microsoft Graph returns Windows timezone names ("Pacific Standard Time") which are silently misinterpreted. Events will be off by hours for non-UTC users.

5. **CalDAV iCalendar parser ignores VTIMEZONE / TZID parameters.** `extract_datetime()` calls `to_timestamp()` with no timezone handling. `DTSTART;TZID=America/New_York:20240315T100000` is treated as UTC.

6. **Two competing CalDAV implementations, neither properly wired.** `crates/core/src/caldav/` has a full client with ctag/etag incremental sync and OAuth2 support. `crates/calendar/src/caldav/` is what actually runs but uses raw reqwest with basic auth only, always does full fetch (never incremental), and returns all events as "created." The core version's features (batched multiget, ctag/etag diffing, OAuth2) are unused.

7. **No runtime reminder/notification system.** Reminders are synced and stored in `calendar_reminders`, displayed in event detail views. But no timer, scheduler, or notification fires reminders at the specified time. The app never alerts users about upcoming events.

### Medium — Interactions requiring custom iced widgets

8. **No drag-to-select time range.** Requires custom widget with mouse tracking. Spec acknowledges this as "the hardest to implement well in iced."

9. **No event drag-to-move.** Same — requires custom widget with hit testing and visual feedback.

10. **No event edge resize.** Same — requires custom drag handlers with edge detection.

11. **No scroll-to-now / working-hours snap.** Blocked on iced fork lacking `scrollable::scroll_to()` API.

12. **Multi-day events not spanning as horizontal bars.** Still render as per-day chips. Continuous spanning requires a fundamentally different layout pass.

### Medium — Event detail / editor gaps

13. **Event detail "popover" not anchored to clicked event.** It's a generic right-aligned overlay, not positioned relative to the event. No context-sensitive RSVP actions, no "Add to my calendar," always offers Edit/Delete regardless of permissions.

14. **No end date field in event editor.** `CalendarEventData` has `start_date` but no `end_date`. Multi-day timed events (flights, conferences) cannot be created.

15. **No time picker popover.** Spec describes a dedicated popover with date pickers, time pickers, timezone button, all-day checkbox. Editor uses plain text input fields for hour/minute.

16. **No reminder editor.** Reminders displayed read-only in detail views. Cannot be created or modified. `ReminderEntry` type imported but unused in editor.

17. **No recurrence editor beyond basic toggle.** On/off toggle defaults to WEEKLY. No day-of-week toggles, month/year options, weekend avoidance, or end conditions.

18. **No attendee input field.** Event editor has no attendee input with autocomplete. Depends on contacts autocomplete infrastructure.

19. **No recurring event edit/delete prompts.** "This / this and following / all" UI not implemented. Requires recurrence instance identity tracking and provider API support.

20. **Double-click to create not wired from UI.** `DoubleClickSlot` message variant exists and is handled but never emitted — iced doesn't expose double-click events on buttons.

### Medium — Provider integration gaps

21. **RSVP action buttons not wired.** RSVP status is displayed. Action buttons (Accept/Decline/Tentative) require provider API calls to send responses.

22. **No "Email organizer" checkbox.** Depends on RSVP actions being wired.

23. **No meeting invite detection.** Requires `text/calendar` MIME part parsing (RFC 5545) in the email rendering pipeline.

24. **No inline RSVP in reading pane.** Depends on meeting invite detection + RSVP action wiring.

25. **No calendar indicator on thread cards.** Depends on meeting invite detection.

### Medium — Other gaps

26. **No shared calendar detection or permission-aware UI.** Graph API fetches `canEdit` but never uses it. No read-only mode for calendars where user lacks edit permission.

27. **IMAP accounts have no calendar provider path.** `calendar_provider_kind()` handles "caldav", "gmail_api", and "graph" but not "imap". Many IMAP servers co-host CalDAV. No auto-discovery or UI to associate CalDAV with IMAP accounts.

28. **Attendees not pre-filled from email participants.** "Create event from email" sets title and description but not attendees, despite a code comment saying "Pre-fill attendees from To/Cc addresses."

29. **Create-event-from-email switches to calendar mode.** Spec says it should open inline, not switch modes.

30. **No "+N more" overflow in month view.** Time grid's all-day bar has this. Month view does not.

31. **Location URL not clickable.** Full modal applies `text::primary` style to URLs but actual hyperlink opening not implemented.

32. **Pop-out bring-to-foreground not implemented.** One-pop-out limit enforced. No badge on mode toggle, no `window::focus()` / `window::raise()` API in iced.

33. **Mail scroll position not restorable after calendar switch.** Calendar state preserved. Mail selected-thread preserved. Scroll position cannot be restored — iced fork lacks `scroll_to()`.

### Previously marked resolved but not fully correct

34. **Calendar list not grouped by account.** Previous audit said grouped. Current UI is a flat list.

35. **ISO week number click does not switch to week view.** Only selects that row's first date. Spec says it should navigate to week view.

36. ~~**`SELECT *` still exists in calendar code.**~~ ✅ Fixed — replaced with explicit 15-column list.

37. **Unsaved-change detection incomplete.** Only checks title, description, location. Changes to time, all-day, timezone, recurrence, availability, visibility, or calendar assignment discarded without prompt.

38. **Recurrence expansion only handles basic RRULE subset.** FREQ, INTERVAL, COUNT, UNTIL supported. Richer semantics (BYDAY, BYMONTH, EXDATE, etc.) not handled.

39. **Month-view "+X more" just selects the date.** Does not open a day-event popover/list as the spec describes.
