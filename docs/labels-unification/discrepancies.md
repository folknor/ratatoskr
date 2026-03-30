# Labels Unification: Spec vs. Code Discrepancies

Audit date: 2026-03-30

---

## Functional blockers

1. ~~**Command palette rejects non-Gmail label operations.**~~ ✅ Fixed — `is_folder_based_provider()` gate removed from `command_resolver.rs`. All providers can now use Add Label / Remove Label from the command palette.

2. **Palette queries use legacy type filtering, not label_kind.** `get_user_folders_for_palette()`, `get_user_labels_for_palette()`, `get_thread_labels_for_palette()`, and `get_all_labels_cross_account()` query by provider type and visibility, not by unified container/tag semantics.

## Model gaps

3. **Sidebar single-account scope surfaces account-local tag rows.** The spec says section 4 is always cross-account grouped labels. In single-account scope, `build_account_labels()` injects account-specific tag rows directly, bypassing the cross-account grouping.

4. **Cross-account label grouping missing trim normalization.** SQL groups by `l.name COLLATE NOCASE` but does not trim whitespace. "Work" and "Work " are different sidebar entries. Spec requires case-insensitive + trim.

5. **Label color overrides not implemented beyond migration.** `label_color_overrides` table exists (migration 67) but has zero code reads. The runtime resolver uses synced provider color or hash fallback only. The spec's 3-tier priority (user override > synced > hash) is not enforced.

## Unimplemented features

6. **Cross-account label creation** — No action or UI to create a new label.
7. **User-initiated label deletion** — No UI in the sidebar labels section.
8. **`label:` search operator** — Not verified in smart folder query parser.

## Stale docs

9. **`problem-statement.md` references `apply_category()`/`remove_category()`** — Removed in Phase 6. Lines ~222 and ~284 are stale.
10. **`generate-test-db.py`** still creates a `categories` table and different-shaped `label_color_overrides` table.

## Resolved

- Phase 6 backend cleanup fully implemented (ProviderOps unified, legacy tables dropped, action service tag guard, IMAP keyword capability).
- Phase-6-plan.md can be deleted (all steps verified complete).
