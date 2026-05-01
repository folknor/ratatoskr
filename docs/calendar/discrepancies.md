# Calendar: Spec vs. Code Discrepancies

Audit date: 2026-03-30

---

## Remaining Discrepancies

### High

7. **No runtime reminder/notification system.** Reminders are synced and stored in `calendar_reminders`, displayed in event detail views. But no timer, scheduler, or notification fires reminders at the specified time. The app never alerts users about upcoming events.

### Medium - Interactions requiring custom iced widgets

8. **No drag-to-select time range.** Requires custom widget with mouse tracking. Spec acknowledges this as "the hardest to implement well in iced."

9. **No event drag-to-move.** Same - requires custom widget with hit testing and visual feedback.

10. **No event edge resize.** Same - requires custom drag handlers with edge detection.

11. **No scroll-to-now / working-hours snap.** Blocked on iced fork lacking `scrollable::scroll_to()` API.

12. **Multi-day events not spanning as horizontal bars.** Still render as per-day chips. Continuous spanning requires a fundamentally different layout pass.

### Medium - Event detail / editor gaps

13. **Event detail "popover" not anchored to clicked event.** It's a generic right-aligned overlay, not positioned relative to the event. No context-sensitive RSVP actions, no "Add to my calendar," always offers Edit/Delete regardless of permissions.

14. **No end date field in event editor.** `CalendarEventData` has `start_date` but no `end_date`. Multi-day timed events (flights, conferences) cannot be created.

15. **No time picker popover.** Spec describes a dedicated popover with date pickers, time pickers, timezone button, all-day checkbox. Editor uses plain text input fields for hour/minute.

16. **No reminder editor.** Reminders displayed read-only in detail views. Cannot be created or modified. `ReminderEntry` type imported but unused in editor.

17. **No recurrence editor beyond basic toggle.** On/off toggle defaults to WEEKLY. No day-of-week toggles, month/year options, weekend avoidance, or end conditions.

18. **No attendee input field.** Event editor has no attendee input with autocomplete. Depends on contacts autocomplete infrastructure.

19. **No recurring event edit/delete prompts.** "This / this and following / all" UI not implemented. Requires recurrence instance identity tracking and provider API support.

20. **Double-click to create not wired from UI.** `DoubleClickSlot` message variant exists and is handled but never emitted - iced doesn't expose double-click events on buttons.

### Medium - Provider integration gaps

21. **RSVP action buttons not wired.** RSVP status is displayed. Action buttons (Accept/Decline/Tentative) require provider API calls to send responses.

22. **No "Email organizer" checkbox.** Depends on RSVP actions being wired.

23. **Meeting invite detection: backend done, UI pending.** `messages.has_meeting_invite` and `meeting_invite_method` are now populated at message-insert time across all four providers (Gmail/Graph/JMAP/IMAP) by inspecting the attachment list for `text/calendar` / `application/ics` MIME parts. `meeting_invite_uid` is still `NULL` (requires reading + parsing the iCal payload, which means a follow-up that fetches the attachment bytes during sync). UI affordances - calendar pill on thread cards, RSVP buttons in the reading pane, inline meeting summary - are not wired.

24. **No inline RSVP in reading pane.** Depends on meeting invite detection + RSVP action wiring.

25. **No calendar indicator on thread cards.** Depends on meeting invite detection.

### Medium - Other gaps

26. **No permission-aware UI.** `canEdit` is now persisted on the `calendars` row (Graph populates it from `canEdit`; Google reads `accessRole`; CalDAV/JMAP default to editable until provider-specific permission probes land). UI gating on action buttons is still pending - actions are dispatched regardless of `can_edit`.

27. **IMAP accounts: no auto-discovery for co-hosted CalDAV.** Routing now treats any account with `calendar_provider = "caldav"` and a non-empty `caldav_url` as a CalDAV calendar account, regardless of mail provider. Auto-discovery (probing `/.well-known/caldav` on the mail server's domain) and the settings UI to associate CalDAV with an IMAP account are still missing.

28. **Attendees not pre-filled from email participants.** "Create event from email" sets title and description but not attendees, despite a code comment saying "Pre-fill attendees from To/Cc addresses."

29. **Create-event-from-email switches to calendar mode.** Spec says it should open inline, not switch modes.

30. **No "+N more" overflow in month view.** Time grid's all-day bar has this. Month view does not.

31. **Location URL not clickable.** Full modal applies `text::primary` style to URLs but actual hyperlink opening not implemented.

32. **Pop-out bring-to-foreground not implemented.** One-pop-out limit enforced. No badge on mode toggle, no `window::focus()` / `window::raise()` API in iced.

33. **Mail scroll position not restorable after calendar switch.** Calendar state preserved. Mail selected-thread preserved. Scroll position cannot be restored - iced fork lacks `scroll_to()`.

### Previously marked resolved but not fully correct

34. **Calendar list not grouped by account.** Previous audit said grouped. Current UI is a flat list.

35. **ISO week number click does not switch to week view.** Only selects that row's first date. Spec says it should navigate to week view.

37. **Unsaved-change detection incomplete.** Only checks title, description, location. Changes to time, all-day, timezone, recurrence, availability, visibility, or calendar assignment discarded without prompt.

38. **Recurrence expansion still incomplete.** FREQ, INTERVAL, COUNT, UNTIL, BYDAY (DAILY/WEEKLY), BYMONTHDAY, BYMONTH supported. Still missing: EXDATE (exception list - lives on a separate iCal property, not the RRULE string), BYSETPOS, BYWEEKNO, ordinal BYDAY (e.g. "the 3rd Wednesday" via `BYDAY=3WE`), WKST, RDATE.

39. **Month-view "+X more" just selects the date.** Does not open a day-event popover/list as the spec describes.
