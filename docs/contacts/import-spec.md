# Contact Import: Problem Statement

## Overview

Enterprise users manage contacts in spreadsheets. The reality is Excel files with columns that are never consistently named, and CSV exports with unpredictable encoding and delimiters. A contact importer must handle this mess gracefully.

This is a self-contained problem warranting its own crate (`crates/contact-import/`).

## Supported Formats

1. **CSV** — the most common export format from Excel and other tools
   - Encoding detection: UTF-8, UTF-16 (with/without BOM), Windows-1252, Latin-1, and other locale-specific encodings
   - Delimiter detection: comma, semicolon, tab (European Excel uses semicolons)
   - Quoted fields, escaped quotes, mixed line endings

2. **XLSX** — Excel's native format
   - Consistently UTF-8 internally (OOXML)
   - Multi-sheet workbooks (user may need to select a sheet)
   - OOXML parsing infrastructure already exists in the squeeze crate

3. **vCard (.vcf)** — standard contact interchange format
   - RFC 6350 (vCard 4.0) and RFC 2426 (vCard 3.0)
   - CardDAV vCard parsing already exists in `crates/core/src/carddav/parse.rs`
   - A single .vcf file can contain multiple contacts

## The Column Mapping Problem

CSV and XLSX files have no standardized column headers. The importer must:

1. **Auto-detect columns** — heuristic matching on common header names:
   - Email: "email", "e-mail", "email address", "mail", "e-post", etc.
   - Name: "name", "full name", "display name", "contact", "navn", etc.
   - First/Last: "first name", "last name", "given name", "surname", "fornavn", "etternavn", etc.
   - Phone: "phone", "telephone", "mobile", "tel", "telefon", etc.
   - Company: "company", "organization", "org", "firma", etc.
   - Group: "group", "category", "list", "gruppe", etc.

2. **Let the user correct the mapping** — auto-detection will be wrong sometimes. The user must be able to reassign columns before import.

3. **Preview the result** — show what will be imported before committing, so the user can catch mapping errors.

## Import Flow

### Step 1: File Selection

User selects a file (.csv, .xlsx, .vcf). For .xlsx with multiple sheets, a sheet selector appears.

### Step 2: Preview + Column Mapping

The importer displays a table preview of the first ~20 rows. Each column has a dropdown header where the user can assign its role (Email, Name, Phone, Company, Group, or Ignore). Auto-detected assignments are pre-filled.

**No headers?** The importer must detect whether the first row is a header or data. Heuristic: if the first row contains values that look like email addresses, phone numbers, or other data rather than label-like strings, treat it as data (no headers). In that case, column mapping relies entirely on content sniffing (e.g., a column where most values contain `@` is probably Email) and user correction. A checkbox "First row is a header" lets the user override the detection either way.

```
┌─────────────────────────────────────────────────────────────┐
│ Column mapping:                                             │
│                                                             │
│  [Email ▾]          [Name ▾]          [Phone ▾]             │
│  ──────────────────────────────────────────────             │
│  alice@corp.com     Alice Smith       +47 123 456           │
│  bob@corp.com       Bob Jones         +47 789 012           │
│  carol@corp.com     Carol Williams                          │
│  ...                                                        │
│                                                             │
│ Account: [Work Account ▾]                                   │
│                                                             │
│ 247 contacts will be imported.                              │
│ 3 rows skipped (no valid email).                            │
│                                                             │
│ [Import]                                                    │
└─────────────────────────────────────────────────────────────┘
```

- Rows without a valid email in the mapped email column are skipped (with a count shown)
- An account selector determines where imported contacts are created (same options as contact creation: provider accounts + "Local")
- If a "Group" column is mapped, contacts are automatically added to groups matching the group column value (groups are created if they don't exist)

### Step 3: Import

The import runs. A summary shows: N contacts imported, N groups created, N rows skipped, N duplicates (matched by email).

**Duplicate handling:** If an imported email matches an existing contact, the existing contact is not overwritten. The duplicate is skipped and reported in the summary.

### vCard Import

vCard files skip the column mapping step entirely — the format is structured. The flow is: file selection → preview (list of contacts to be imported) → account selector → import.

## Crate Design

`crates/contact-import/` — pure library crate, no UI. Responsibilities:

- File format detection and parsing
- Encoding detection and conversion
- Delimiter detection
- Column header heuristic matching
- Parsed contact rows as structured output
- vCard parsing (reuse or depend on existing CardDAV parser)

The UI for the import wizard lives in the app crate. The import crate provides the data; the app provides the interaction.

## Ecosystem Patterns

How patterns from the [iced ecosystem survey](../iced-ecosystem-survey.md) map to this spec's requirements.

### Requirements to Survey Matches

| Requirement | Primary Source | How It Applies |
|---|---|---|
| Preview data table | shadcn-rs `data_table` | Column renderers, row iteration; adapt for dynamic columns (unknown at compile time) |
| File selection | shadcn-rs/pikeru (`rfd`) | `rfd::AsyncFileDialog` with format filter — solved problem |
| Multi-step wizard | raffi query routing | `ImportStep` enum state machine: FileSelect → SheetSelect → Preview → Importing → Summary |
| Column mapping dropdowns | shadcn-rs/iced-plus props-builder | `ColumnRole` enum with iced `pick_list` per column header |
| Import progress/cancel | bloom generational tracking + pikeru subscriptions | Tag import task with generation; stream row-by-row progress |
| Account selector | bloom config shadow | Entire wizard state is transient editing state; only commits on "Import" |
| Drag-and-drop file import | iced_drop + shadcn-rs file-drop-zone | Optional enhancement using OS-level drag events |

### Gaps

- **Encoding detection** (UTF-8, UTF-16, Windows-1252): Library crate concern (`chardetng`, `encoding_rs`) — no iced involvement
- **vCard parsing**: Internal (existing CardDAV parser in `crates/core/src/carddav/parse.rs`)
- **Duplicate handling**: Library crate concern — matching logic is entirely backend, no UI pattern needed
