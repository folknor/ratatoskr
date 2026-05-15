# Glossary Discrepancies

Audit date: 2026-05-15
Resolution pass: 2026-05-15

Findings were consolidated from five independent audits of the codebase against
`docs/glossary/folders-labels.md`. Theme numbers are stable identifiers and must
not be renumbered.

## Current Status

All themes from the audit are resolved in the codebase as of the resolution pass.
This document is kept as the stable issue ledger rather than as an active bug
list.

## Theme 1: Message State Never Extracted (resolved)

Messages now persist replied and forwarded booleans. Gmail, Graph, IMAP, and
JMAP parsers extract provider-native reply and forward signals, and the thread
list plus reading pane expose inline reply and forward glyphs.

## Theme 2: Read / Starred Mis-Stored As Labels (resolved)

Read and starred are no longer projected into user label rows. Legacy state rows
are defensively filtered and removed during sync write paths.

## Theme 3: Reserved RFC 5788 System Keywords Surfaced As User Labels (resolved)

Reserved IMAP and JMAP system keywords are filtered before label persistence.
Forwarded state is routed into message state instead of the label surface.

## Theme 4: Graph Importance + Outlook-Specific Synthesis Missing (resolved)

Graph importance is synthesized as mutually exclusive high and low importance
labels, write operations patch Graph importance, Focused Inbox no longer leaks
as a label, and Graph reply / forward state is extracted from the MAPI extended
property.

## Theme 5: Command Palette Mixes Folders And Labels (resolved)

Palette folder queries now return container rows only. Palette label queries now
return tag rows only, and cross-account option IDs preserve the distinction.

## Theme 6: IMAP Flagged Classified As A Folder (resolved)

IMAP flagged state is treated as starred message state. Flagged virtual folders
are excluded from the syncable folder list.

## Theme 7: Typed ID Names Used Provider Tag Terminology (resolved)

Shared typed IDs, sidebar selection, command arguments, service API wire IDs,
and provider operations now use label terminology for additive annotations.

## Theme 8: Folder Vs Label Naming Drift In Provider Mappers (resolved)

JMAP, Graph, and IMAP folder mapper APIs now name folders as folders. Mixed
storage outputs remain named as label rows only at the `labels` table boundary.

## Theme 9: Folder Operations Carried Label Naming (resolved)

Move source fields now use folder naming, pending-operation replay reads the new
field with a legacy fallback, folder action entry points use `FolderId`, and the
test database harness exposes parent folder IDs.

## Theme 10: Upsert Label Defaulted Silently To Container (resolved)

The legacy upsert helper writes and updates `label_kind` explicitly.

## Theme 11: Get Labels Returned Folders, Tags, And Reserved Keywords (resolved)

Shared label readers defensively filter message-state and reserved keyword rows;
UI label pills only render tag-kind labels.

## Theme 12: UI Gaps And Naming Drift (resolved)

Thread rows render label color dots and reply / forward indicators. Non-search
empty state copy no longer says folder, the thread-list mode is scope-based, and
typeahead display text no longer overloads label terminology.

## Theme 13: Schema Dead Column + Glossary Path Drift (resolved)

The glossary path now points at the actual `SYSTEM_FOLDER_ROLES` location. System
folder unread-count helpers use system-folder naming. The legacy `labels.type`
column remains a storage compatibility field for system/user metadata and is not
used as the folder-vs-label discriminator; `label_kind` is the only glossary
discriminator.

## Theme 14: Doc-Comment Hygiene (resolved)

Comments in provider mappers, Graph parsing, folder actions, and label actions
now reflect the folder / label / message-state model.
