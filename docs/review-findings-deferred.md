# Deferred Review Findings

Items identified during code review (2026-03-20) that were not fixed
in the bug-fix pass. Grouped by commit.

---

## 1ba6249 — Pinned searches

### Finding 4: No PreSearchView navigation-target restoration
The spec recommends against the `pre_search_threads` clone approach
(calling it a "V1 shortcut") and proposes `PreSearchView` for
navigation-target-based restoration. The implementation uses
`pre_search_threads` for save and `restore_folder_view()` for dismiss.
Both search and pinned searches should converge on `PreSearchView`.

### Finding 5: PinnedSearch struct omits `thread_ids` field from spec
The spec defines `thread_ids: Vec<(String, String)>` on the struct
(loaded lazily) so re-clicking the same pinned search doesn't re-query
the DB. The implementation always re-queries. Minor — the DB query
is fast.

### Finding 6: Missing Phase 2 features
No staleness label, no `SearchBarState` type, no periodic expiry
subscription. Phase 2/4 items.

---

## c9d6a42 — Pop-out message view

### Finding 5: Spec scaffolding fields omitted
`cc_addresses`, `rendering_mode`, `raw_source`, `scroll_offset`,
window position tracking. Acceptable for V1.

---

## d650308 — Pop-out compose window

*(All findings in this commit were fixed.)*

---

## 033650c — Contacts management UI

### Finding 3: Save pattern contradicts spec
TODO.md says "contacts save immediately with no Save/Cancel — shadow
pattern does NOT apply." The spec distinguishes local (immediate save)
vs synced (explicit Save). Implementation uses explicit Save for all
contacts. Needs decision: immediate-save for local contacts, or keep
explicit Save everywhere.

### Finding 4: `account_id` hardcoded to None on save
No account selector dropdown — every contact is implicitly "Local."
Spec calls for account association.

### Finding 5: No delete confirmation
Spec says "Deletion prompts for confirmation." Both contact and group
delete are immediate and irreversible.

### Finding 8: N+1 query for group memberships
`load_contacts_filtered()` calls `load_contact_groups()` per contact.
200 contacts = 201 queries. Minor at current scale, but should be
a single JOIN query eventually.

---

## b15cd89 — Emoji picker widget

### Finding 2: Missing features from TODO spec
TODO.md says the picker needs "recent/frequent section, skin tone
selection." Neither is implemented.

### Finding 5: Missing Flags category
Most emoji pickers include country/flag emoji. Not included in the
static table.
