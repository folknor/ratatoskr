# Signatures: Implementation Spec

Detailed implementation spec for email signature management, editing, and
compose insertion. Covers four phases: data model + CRUD, management UI in
Settings, compose insertion behavior, and account-switching replacement.

**Depends on:** The rich text editor subsystem (see `docs/editor/architecture.md`; currently targeting `crates/rte/`, though the final crate location may differ) - the signature editor IS the rich text editor. The editor must be at Phase 3 (HTML round-trip) before Phase 2 of this spec can start. This spec references editor types by logical name (`Document`, `Block`, `EditorAction`) rather than hard-coding import paths, so the crate location is not a blocking decision.

**References:**
- `docs/editor/architecture.md` - Document model, Block tree, StyledRun, HTML
  serialization
- `docs/pop-out-windows/problem-statement.md` § Signature - insertion behavior
- `docs/roadmap/signatures.md` - roaming signature research, provider sync
- `docs/accounts/problem-statement.md` - per-account settings, account card
  editor
- `docs/implementation-plan.md` - Tier 3, depends on editor

---

## Existing Infrastructure

Before designing, here is what already exists in the codebase.

### Database schema (migration v1 + v41)

The `signatures` table was created in migration v1 and extended in v41:

```sql
CREATE TABLE signatures (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    body_html TEXT NOT NULL,
    is_default INTEGER DEFAULT 0,
    sort_order INTEGER DEFAULT 0,
    created_at INTEGER DEFAULT (unixepoch())
);

-- v41 additions:
ALTER TABLE signatures ADD COLUMN server_id TEXT;
ALTER TABLE signatures ADD COLUMN body_text TEXT;
ALTER TABLE signatures ADD COLUMN is_reply_default INTEGER NOT NULL DEFAULT 0;
ALTER TABLE signatures ADD COLUMN source TEXT NOT NULL DEFAULT 'local';
ALTER TABLE signatures ADD COLUMN last_synced_at INTEGER;
ALTER TABLE signatures ADD COLUMN server_html_hash TEXT;
CREATE UNIQUE INDEX idx_signatures_server ON signatures(account_id, server_id);
```

### Rust types (`crates/db/src/db/types.rs`)

```rust
pub struct DbSignature {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub is_default: i64,
    pub sort_order: i64,
}
```

**Note:** `DbSignature` does not include the v41 sync columns (`server_id`,
`body_text`, `is_reply_default`, `source`, `last_synced_at`,
`server_html_hash`). These must be added to the struct and its `FromRow` impl.

### CRUD queries (`crates/core/src/db/queries_extra/compose.rs`)

All basic CRUD exists:

- `db_get_signatures_for_account(db, account_id)` - list all for an account
- `db_get_default_signature(db, account_id)` - get the default (is_default = 1)
- `db_insert_signature(db, account_id, name, body_html, is_default)` - insert,
  clearing old default if needed (transactional)
- `db_update_signature(db, id, name, body_html, is_default)` - update with
  optional fields, clearing old default if needed (transactional)
- `db_delete_signature(db, id)` - delete

### Provider sync

- **Gmail:** `crates/gmail/src/sync/labels.rs` pulls signatures from
  `sendAs.signature`, pushes local edits via `update_send_as_signature`
- **JMAP:** `crates/jmap/src/signatures.rs` has `sync_jmap_identity_signatures`
  (pull) and `push_signature_to_jmap` (push)
- **Inline images:** `crates/common/src/signature_images.rs` extracts
  base64 data-URI images from signature HTML, deduplicates via xxh3, stores in
  the inline image store

### Related tables

- `send_as_aliases` has a `signature_id TEXT` FK - per-alias signature
  assignment
- `local_drafts` has a `signature_id TEXT` - tracks which signature was used in
  a draft
- `scheduled_emails` has a `signature_id TEXT` - tracks which signature was used
  in a scheduled send

### Settings UI

The Composing tab in Settings (`crates/app/src/ui/settings.rs`) already has a
placeholder section:

```rust
col = col.push(section("Signatures", vec![
    coming_soon_row("Signature management"),
]));
```

---

## Phase 1: Data Model Completion

Goal: complete the Rust types and add missing queries so the full signature
lifecycle is representable before any UI work begins.

### 1.1 Extend `DbSignature` with sync columns

**File:** `crates/db/src/db/types.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbSignature {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub body_text: Option<String>,
    pub is_default: i64,
    pub is_reply_default: i64,
    pub sort_order: i64,
    pub source: String,           // "local" | "gmail_sync" | "jmap_sync" | "exchange_parsed"
    pub server_id: Option<String>,
    pub server_html_hash: Option<String>,
    pub last_synced_at: Option<i64>,
    pub created_at: i64,
}
```

Update the `FromRow` impl in `crates/db/src/db/from_row_impls.rs` to read all
columns. Update all `SELECT` statements in `compose.rs` to include the new
columns.

### 1.2 Add `body_text` auto-generation

When saving a signature (insert or update), auto-generate `body_text` from
`body_html` by stripping tags. This plain-text fallback is used for:

- The `text/plain` multipart alternative in outgoing email
- Provider sync (JMAP `textSignature`)

**Function signature** (in `crates/core/src/`):

```rust
/// Strip HTML tags and decode entities to produce a plain-text signature.
/// Preserves line breaks from block elements (<p>, <br>, <div>).
pub fn html_to_plain_text(html: &str) -> String
```

This is a simple tag-stripping function, not a full HTML renderer. Block
elements insert newlines; inline elements are dropped; `&amp;` / `&lt;` /
`&gt;` / `&nbsp;` are decoded. The existing `lol_html` dependency can handle
this efficiently.

### 1.3 Add missing queries

**File:** `crates/core/src/db/queries_extra/compose.rs`

```rust
/// Get all signatures across all accounts. Used by the signature list in
/// Settings when no account filter is applied.
pub async fn db_get_all_signatures(
    db: &DbState,
) -> Result<Vec<DbSignature>, String>

/// Get the reply-default signature for an account. Falls back to is_default
/// if no is_reply_default is set.
pub async fn db_get_reply_signature(
    db: &DbState,
    account_id: String,
) -> Result<Option<DbSignature>, String>

/// Reorder signatures for an account.
pub async fn db_reorder_signatures(
    db: &DbState,
    account_id: String,
    ordered_ids: Vec<String>,
) -> Result<(), String>

/// Set the reply-default signature for an account. Clears the old
/// reply-default in a transaction.
pub async fn db_set_reply_default_signature(
    db: &DbState,
    account_id: String,
    signature_id: String,
) -> Result<(), String>
```

`db_get_reply_signature` logic:

```sql
SELECT * FROM signatures
WHERE account_id = ?1
  AND (is_reply_default = 1 OR is_default = 1)
ORDER BY is_reply_default DESC
LIMIT 1
```

This returns the reply-default if one exists, otherwise falls back to the
account's default signature.

### 1.4 Signature resolution for compose

A new function encapsulates the logic for "which signature should I insert?"
given the compose context.

**File:** `crates/core/src/db/queries_extra/compose.rs`

```rust
/// Determines the signature to insert for a given compose scenario.
///
/// Resolution order:
/// 1. If the send-as alias has a `signature_id`, use that.
/// 2. For reply/forward: use `is_reply_default` signature, falling back to
///    `is_default`.
/// 3. For new compose: use `is_default` signature.
/// 4. If no default is set: return None (no signature inserted).
pub async fn db_resolve_signature_for_compose(
    db: &DbState,
    account_id: String,
    from_email: Option<String>,
    is_reply: bool,
) -> Result<Option<DbSignature>, String>
```

Implementation:

1. If `from_email` is provided, look up `send_as_aliases` for that email. If
   the alias has a `signature_id`, fetch and return that signature.
2. If `is_reply`, call `db_get_reply_signature`.
3. Otherwise, call `db_get_default_signature`.

---

## Phase 2: Signature Management UI in Settings

Goal: replace the "coming soon" placeholder with a full signature management
surface. Users can create, edit, reorder, and delete signatures, and assign
per-account defaults.

### 2.1 Signatures section in the Composing tab

The Composing tab's "Signatures" section becomes a list of signatures grouped
by account, with create/edit/delete actions. The implementation follows the
existing `editable_list` pattern used for Labels and Filters.

#### Data flow

On entering the Composing tab, load all signatures from the database:

```rust
// In Settings state
pub signatures: Vec<DbSignature>,
pub signatures_loaded: bool,
```

Load is triggered by `SelectTab(Tab::Composing)` - a `Task::perform` calls
`db_get_all_signatures` and returns them via a new
`SettingsMessage::SignaturesLoaded(Result<Vec<DbSignature>, String>)`.

#### Signature list layout

```
Signatures
┌──────────────────────────────────────────────────────────┐
│ ≡  Work Signature              Default ✕                │
│─────────────────────────────────────────────────────────│
│ ≡  Personal Signature                   ✕                │
│─────────────────────────────────────────────────────────│
│              + Add Signature                             │
└──────────────────────────────────────────────────────────┘
```

Each row shows:

- **Grip handle** (≡) - for drag reordering (same as existing `editable_list`)
- **Name** - the signature name (TEXT_LG, text::base)
- **Default badge** - if `is_default = 1`, show a "Default" chip/badge
  (TEXT_SM, muted). If `is_reply_default = 1`, show "Reply default".
- **Remove button** (✕) - deletes the signature with confirmation
- **Click** - opens the signature editor overlay (slide-in from right)

The "Add Signature" button at the bottom opens the editor overlay with an empty
signature.

**Account grouping:** Signatures are grouped by account. Each group has a
header showing the account email and color indicator. This uses the same visual
pattern as the existing Labels section in Mail Rules.

#### Signature list messages

```rust
// New SettingsMessage variants
SignaturesLoaded(Result<Vec<DbSignature>, String>),
SignatureCreate(String),           // account_id
SignatureEdit(String),             // signature_id
SignatureDelete(String),           // signature_id
SignatureDeleted(Result<(), String>),
SignatureReorder(String, Vec<String>),  // account_id, ordered_ids
SignatureSetDefault(String, String),     // account_id, signature_id
SignatureSetReplyDefault(String, String),
```

### 2.2 Signature editor overlay

Clicking a signature row (or "Add Signature") opens a slide-in overlay - the
same pattern as `SettingsOverlay::CreateFilter`. The overlay contains the
signature editor.

#### Overlay enum extension

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsOverlay {
    CreateFilter,
    EditSignature {
        /// None for new signature, Some for editing existing.
        signature_id: Option<String>,
        account_id: String,
    },
}
```

**Note:** `SettingsOverlay` must change from `Copy` to `Clone` since it now
contains `String` fields. All existing `Copy` derives on types containing
`SettingsOverlay` must be updated.

#### Signature editor state

```rust
/// Editing state for the signature editor overlay.
pub struct SignatureEditorState {
    /// The signature being edited (None = new).
    pub signature_id: Option<String>,
    pub account_id: String,
    pub name: String,
    pub is_default: bool,
    pub is_reply_default: bool,
    /// The rich text editor's document model.
    pub document: Document,
    /// The widget state for the rich text editor.
    pub editor_state: EditorWidgetState,
    /// Whether the signature has unsaved changes.
    pub dirty: bool,
}
```

#### Editor overlay layout

```
┌──────────────────────────────────────────────────────────┐
│  ← Back                                                  │
│                                                          │
│  Edit Signature                                          │
│                                                          │
│  Name                                                    │
│  [Work Signature                                      ]  │
│                                                          │
│  ☐ Default for new messages                              │
│  ☐ Default for replies & forwards                        │
│                                                          │
│  ─── Formatting Toolbar ──────────────────────────────   │
│  B  I  U  S  │ • ─ 1. │ "" │ 🔗                         │
│                                                          │
│  ┌──────────────────────────────────────────────────┐    │
│  │                                                  │    │
│  │  Bob Jones                                       │    │
│  │  Engineering Lead · Corp Inc                     │    │
│  │  https://corp.com                                │    │
│  │                                                  │    │
│  └──────────────────────────────────────────────────┘    │
│                                                          │
│                              [Delete]           [Save]   │
└──────────────────────────────────────────────────────────┘
```

- **Name field:** `text_input` with the signature name. Required - cannot save
  with an empty name.
- **Default checkboxes:** Two toggles. "Default for new messages" maps to
  `is_default`. "Default for replies & forwards" maps to `is_reply_default`.
  Both can be enabled simultaneously. Enabling either clears the respective
  default from other signatures for the same account (handled transactionally
  in the CRUD layer).
- **Formatting toolbar:** A horizontal row of format buttons. This is
  identical to the compose window's toolbar - same buttons, same messages,
  same layout. The toolbar is built in the app crate (not in the editor
  crate), sending messages that the editor interprets.
- **Editor surface:** The `RichTextEditor` widget from
  `crates/rte/`. The signature's `body_html` is parsed into a
  `Document` via `html_parse::parse_html()` when the overlay opens. On save,
  the `Document` is serialized back to HTML via `html_serialize::to_html()`.
- **Delete button:** Only shown when editing an existing signature (not for
  new). Shows a confirmation prompt.
- **Save button:** Validates (name non-empty), serializes the document to
  HTML, auto-generates `body_text`, and calls `db_insert_signature` or
  `db_update_signature`. Closes the overlay on success.

#### Editor messages

The signature editor reuses the editor's message types. The toolbar sends
format-toggle messages; the editor widget sends text-input, selection, and
scroll messages. These are namespaced under a new `SettingsMessage` variant:

```rust
SignatureEditorMsg(SignatureEditorMessage),
```

Where `SignatureEditorMessage` wraps the rich text editor's action enum:

```rust
#[derive(Debug, Clone)]
pub enum SignatureEditorMessage {
    NameChanged(String),
    ToggleDefault(bool),
    ToggleReplyDefault(bool),
    EditorAction(EditorAction),  // from crates/rte
    Save,
    Delete,
    DeleteConfirmed,
    Saved(Result<String, String>),  // Ok(signature_id)
}
```

#### HTML round-trip

This is the critical integration point with the editor. The signature editor
exercises the editor's full HTML round-trip:

1. **Open:** `body_html` (from DB) → `html_parse::parse_html()` → `Document`
2. **Edit:** User modifies the `Document` via the editor widget
3. **Save:** `Document` → `html_serialize::to_html()` → `body_html` (to DB)

The editor's `html_parse` module handles the signature's HTML subset: basic
formatting tags (`<strong>`, `<em>`, `<u>`, `<s>`, `<a>`), block elements
(`<p>`, `<h1>`-`<h3>`, `<ul>`, `<ol>`, `<li>`, `<blockquote>`, `<hr>`), and
inline images (`<img>` with `inline-image:` or `https:` src).

For server-synced signatures (Gmail, JMAP), the imported HTML may contain
provider-specific markup (Gmail's `<div dir="ltr">`, Outlook's `MsoNormal`
classes). The editor's parser handles this gracefully: unknown block elements
become paragraphs, unknown inline elements pass through content. The HTML that
gets saved back is the editor's clean output, not a round-trip of the
provider's original markup. This is acceptable - the signature content is
preserved, only the markup changes.

### 2.3 Per-account default signature in Account Settings

The account editor slide-in (specified in `docs/accounts/problem-statement.md`
§ Account Actions) gets a new "Default signature" dropdown.

This is a `widgets::select` showing all signatures for the account plus "None".
Selecting a signature calls `db_update_signature` to set `is_default = 1` on
the chosen signature (and clear the old default).

This provides a second path to assign defaults - the first is the checkbox in the signature editor itself. Both paths are equivalent surfaces over the same state (`is_default` / `is_reply_default` columns on the `signatures` table) and use the same underlying CRUD operations. They must not diverge.

**Dependency:** This section requires the account settings implementation (`docs/accounts/implementation-spec.md`) to be real enough to supply the account editor slide-in with account grouping and a dropdown insertion point. The signature management UI (Phase 2's list grouped by account) also depends on account metadata being accessible. If account settings ships first, the default-signature dropdown is a small addition. If signatures ship first, the dropdown is deferred until the account editor exists.

---

## Phase 3: Signature Insertion in Compose

Goal: when a compose window opens, the correct signature is automatically
inserted into the editor's document. The signature is part of the document
(editable, serialized with the body on send) but is tracked so it can be
replaced when the user switches From accounts.

### 3.1 Signature as Document blocks

When a signature is resolved for compose (via `db_resolve_signature_for_compose`),
its `body_html` is parsed into a `Document`, and the blocks are appended to the
compose document with a separator.

The compose document structure:

```
Block 0:    Empty paragraph (cursor starts here)
...         (user's message content)
Block N:    SignatureSeparator (HorizontalRule)
Block N+1:  First block of signature
Block N+2:  Second block of signature
...         (remaining signature blocks)
Block M:    (for reply/forward) attribution line paragraph
Block M+1:  BlockQuote containing quoted content
```

#### Separator convention

The signature separator is a `Block::HorizontalRule`. In HTML serialization, it becomes `<hr>`. When the editor serializes the full document for sending, the `<hr>` naturally appears between the user's content and the signature.

**This is a deliberate outgoing markup choice.** An `<hr>` is a stronger visual separator than a simple divider line, and it will be visible in the recipient's mail client. This matches the convention used by Outlook, Thunderbird, and Apple Mail for signature separation. If user feedback indicates the `<hr>` is too heavy, it can be replaced with a styled `<div>` border or a lighter visual treatment - but for V1, `<hr>` is the simplest correct choice because it has universal mail-client support and clear semantic meaning.

For RFC 3676 compliance in the `text/plain` alternative, the plain-text
serializer emits `-- \n` (dash dash space newline) before the signature's
plain text. This is handled in the send path, not in the editor.

#### Wrapper div for signature identification

When serializing to HTML for send, the signature blocks are wrapped in:

```html
<div id="ratatoskr-signature" data-signature-id="{uuid}">
  <!-- signature blocks rendered here -->
</div>
```

This wrapper is NOT part of the editor's document model - it is added during HTML serialization for outgoing email only. Its primary purpose is **interoperability**: other email clients can identify and strip the signature on reply, and web-based clients can style or collapse it. The `data-signature-id` attribute also enables Ratatoskr to identify its own signatures when re-parsing sent messages.

Draft restoration does NOT depend on this wrapper. Drafts persist the editor `Document` plus `ComposeDocumentState` metadata (including `signature_separator_index` and `active_signature_id`), so the signature region is known structurally without reparsing HTML.

#### Tracking signature block range

The compose state tracks where the signature starts in the document:

```rust
/// Compose-specific document metadata.
pub struct ComposeDocumentState {
    pub document: Document,
    pub editor_state: EditorWidgetState,
    /// Block index where the signature separator (HorizontalRule) is.
    /// None if no signature is inserted.
    ///
    /// V1 region model: the signature region is "everything from this index
    /// to the attribution line (or end of document)." This is pragmatic but
    /// fragile - if block insertions/deletions shift indices, this must be
    /// updated. A stronger approach (e.g., region markers or block-level
    /// metadata) can replace this if signatures and quoted content become
    /// more structurally complex.
    pub signature_separator_index: Option<usize>,
    /// The signature_id that was inserted, for change detection.
    pub active_signature_id: Option<String>,
    /// Whether the user has manually edited the signature blocks.
    pub signature_edited: bool,
}
```

`signature_separator_index` points to the `HorizontalRule` block. Everything
from `signature_separator_index` to the end of the document (or to the
attribution line, if present) is the signature region.

**Edit detection:** When the user edits any block within the signature region
(detected by comparing block indices in edit operations against
`signature_separator_index`), `signature_edited` is set to `true`. This flag
controls whether switching From accounts prompts before replacing the
signature.

### 3.2 Insertion behavior per compose scenario

#### New message

1. Resolve signature via `db_resolve_signature_for_compose(account_id, from_email, false)`
2. If a signature is returned:
   a. Create an empty paragraph (block 0 - cursor position)
   b. Add `Block::HorizontalRule` as separator
   c. Parse signature `body_html` → `Document`, append its blocks
3. Set `signature_separator_index = Some(1)` (after the initial paragraph)
4. Place cursor at `DocPosition::new(0, 0)` - the user types above the
   signature

#### Reply / Reply All

1. Resolve signature via `db_resolve_signature_for_compose(account_id, from_email, true)`
2. Build the document:
   a. Empty paragraph (block 0 - cursor position)
   b. If signature resolved:
      - `Block::HorizontalRule` (separator)
      - Signature blocks
   c. Attribution paragraph: `Block::Paragraph` with runs containing
      "On {date}, {sender_name} wrote:" in italic
   d. `Block::BlockQuote` containing the quoted message blocks (parsed from
      the replied message's HTML)
3. Set `signature_separator_index` accordingly
4. Place cursor at `DocPosition::new(0, 0)`

The attribution line and quoted content are below the signature - this is
the standard top-posting layout.

#### Forward

Same as reply. The attribution line reads "---------- Forwarded message ----------"
(or similar convention). The forwarded message's body appears in a BlockQuote
below. The original message's attachments are included in the compose window's
attachment list.

### 3.3 Compose document assembly function

**File:** `crates/core/src/` (or a new module in the app crate's compose state)

```rust
/// Assemble the initial compose document with signature and optional
/// quoted content.
///
/// Returns the assembled document, the signature separator index (if any),
/// and the active signature ID.
pub fn assemble_compose_document(
    signature: Option<&DbSignature>,
    quoted_content: Option<QuotedContent>,
) -> ComposeDocumentAssembly

pub struct QuotedContent {
    /// Attribution line, e.g., "On Mar 19, Alice Smith wrote:"
    pub attribution: String,
    /// The quoted message's HTML body, to be parsed and wrapped in BlockQuote.
    pub body_html: String,
}

pub struct ComposeDocumentAssembly {
    pub document: Document,
    pub signature_separator_index: Option<usize>,
    pub active_signature_id: Option<String>,
}
```

Implementation:

```rust
pub fn assemble_compose_document(
    signature: Option<&DbSignature>,
    quoted_content: Option<QuotedContent>,
) -> ComposeDocumentAssembly {
    let mut blocks: Vec<Block> = Vec::new();
    let mut sig_sep_index = None;
    let mut sig_id = None;

    // 1. Initial empty paragraph for user content
    blocks.push(Block::empty_paragraph());

    // 2. Signature (if any)
    if let Some(sig) = signature {
        sig_sep_index = Some(blocks.len());
        sig_id = Some(sig.id.clone());

        blocks.push(Block::HorizontalRule);

        let sig_doc = html_parse::parse_html(&sig.body_html);
        for block in sig_doc.blocks {
            blocks.push(Arc::unwrap_or_clone(block));
        }
    }

    // 3. Quoted content (if reply/forward)
    if let Some(quoted) = quoted_content {
        // Attribution line
        let attribution_runs = vec![StyledRun::styled(
            quoted.attribution,
            InlineStyle::ITALIC,
        )];
        blocks.push(Block::Paragraph { runs: attribution_runs });

        // Quoted body in a BlockQuote
        let quoted_doc = html_parse::parse_html(&quoted.body_html);
        blocks.push(Block::BlockQuote {
            blocks: quoted_doc.blocks,
        });
    }

    ComposeDocumentAssembly {
        document: Document::from_blocks(blocks),
        signature_separator_index: sig_sep_index,
        active_signature_id: sig_id,
    }
}
```

---

## Phase 4: Account-Switching Signature Replacement

Goal: when the user changes the From account in a compose window, the signature
updates to match the new account's default.

### 4.1 From-account change handler

When the From dropdown changes in the compose window:

```rust
fn handle_from_account_changed(
    &mut self,
    new_account_id: &str,
    new_from_email: &str,
    is_reply: bool,
) -> Task<ComposeMessage>
```

Steps:

1. Resolve the new signature:
   `db_resolve_signature_for_compose(new_account_id, new_from_email, is_reply)`
2. Compare with `self.active_signature_id`:
   - If the resolved signature ID is the same as the current one: do nothing.
   - If different (or one is None and the other isn't): proceed to replacement.
3. Check `self.signature_edited`:
   - If `true`: show a confirmation dialog: "The signature has been edited.
     Replace with the new account's signature?" with [Keep Current] [Replace]
     buttons.
   - If `false`: replace silently.

### 4.2 Signature replacement

When replacing:

1. Remove existing signature blocks: if `signature_separator_index` is `Some(idx)`,
   remove blocks from `idx` up to (but not including) the attribution line
   (if present). The attribution line is identified as the first
   `Block::Paragraph` after the signature that contains italic text matching
   the attribution pattern, OR as the block immediately before a
   `Block::BlockQuote` at the end of the document.
2. If a new signature is resolved:
   a. Insert `Block::HorizontalRule` at `idx`
   b. Parse new signature HTML → blocks, insert after the separator
   c. Update `signature_separator_index` to `idx`
   d. Update `active_signature_id`
3. If no new signature (new account has no default):
   a. Clear `signature_separator_index` to `None`
   b. Clear `active_signature_id` to `None`
4. Reset `signature_edited` to `false`

```rust
fn replace_signature(
    &mut self,
    new_signature: Option<&DbSignature>,
) {
    // Find the range of signature blocks to remove
    let sig_start = match self.signature_separator_index {
        Some(idx) => idx,
        None => {
            // No existing signature - just insert at the end
            // (before attribution/quoted content if present)
            self.insert_signature(new_signature);
            return;
        }
    };

    // Find the end of the signature region (before attribution/quote)
    let sig_end = self.find_signature_end(sig_start);

    // Remove old signature blocks [sig_start..sig_end)
    for _ in sig_start..sig_end {
        self.document.blocks.remove(sig_start);
    }

    // Insert new signature (if any)
    if let Some(sig) = new_signature {
        self.document.insert_block(sig_start, Block::HorizontalRule);
        let sig_doc = html_parse::parse_html(&sig.body_html);
        for (i, block) in sig_doc.blocks.into_iter().enumerate() {
            let block = Arc::unwrap_or_clone(block);
            self.document.insert_block(sig_start + 1 + i, block);
        }
        self.signature_separator_index = Some(sig_start);
        self.active_signature_id = Some(sig.id.clone());
    } else {
        self.signature_separator_index = None;
        self.active_signature_id = None;
    }

    self.signature_edited = false;
}
```

### 4.3 Confirmation dialog

When `signature_edited` is true and the user switches From accounts with a
different default signature, a modal dialog appears:

```
┌──────────────────────────────────────────────┐
│                                              │
│  The signature has been modified.            │
│  Replace with the new account's signature?   │
│                                              │
│                  [Keep Current]    [Replace]  │
│                                              │
└──────────────────────────────────────────────┘
```

- **Keep Current:** Dismisses the dialog. The old signature stays.
  `active_signature_id` is updated to the new signature's ID (to prevent
  re-prompting if the user switches back and forth), but the blocks are NOT
  replaced.
- **Replace:** Calls `replace_signature` with the new signature.

---

## Phase 5: Send Path Integration

Goal: ensure the signature is correctly serialized in outgoing email.

### 5.1 HTML body serialization

The compose document is serialized via `html_serialize::to_html()`. The
signature blocks serialize naturally as HTML. No special handling is needed in
the editor's serializer - the signature IS part of the document.

However, the send path should wrap the signature region in an identifying div.
This happens AFTER the editor serializes the full document to HTML:

```rust
fn finalize_compose_html(
    html: &str,
    signature_html: Option<&str>,
) -> String
```

This post-processing step finds the `<hr>` that marks the signature separator
and wraps everything from there to the end of the signature in:

```html
<div id="ratatoskr-signature">
  ...signature content...
</div>
```

If the document was modified such that the `<hr>` no longer exists (user
deleted it), the signature wrapper is omitted. This is fine - the signature
content is still part of the body; it just won't be identified as a signature
by other clients.

### 5.2 Plain-text alternative

The `text/plain` multipart alternative is generated from the document's
plain-text projection. The send path inserts `-- \n` (RFC 3676 signature
separator) before the signature's plain text:

```rust
fn finalize_compose_plain_text(
    document: &Document,
    signature_separator_index: Option<usize>,
) -> String
```

1. Serialize blocks 0..separator_index as user content (newline between blocks)
2. If separator exists: append `\n-- \n`
3. Serialize signature blocks as plain text
4. If attribution/quoted content exists: append with `> ` quoting prefix

### 5.3 Draft persistence

When auto-saving a draft, the full document HTML is stored in `local_drafts.body_html`.
The `signature_id` is stored in `local_drafts.signature_id`. On draft
restoration:

1. Parse `body_html` → `Document`
2. Look up `signature_id` to determine if the draft had a signature
3. Locate the `Block::HorizontalRule` to reconstruct `signature_separator_index`
4. Restore `active_signature_id` from the draft's `signature_id`

This means drafts fully preserve the compose state including signature position.

---

## Widget Tree Summary

### Signature list (in Composing tab)

```
section("Signatures")
  column (per account group)
    text (account email + color dot)
    editable_list-like rows:
      row
        grip_handle
        text (signature name)
        [optional] badge ("Default" / "Reply default")
        button (✕ remove)
    button ("+ Add Signature")
```

### Signature editor overlay

```
stack [content_area, blocker, overlay_panel]
  overlay_panel
    column
      button (← Back)
      text ("Edit Signature" / "New Signature")
      text_input (name)
      toggle_row ("Default for new messages")
      toggle_row ("Default for replies & forwards")
      container (formatting toolbar)
        row [B, I, U, S, |, •, ─, 1., |, "", |, 🔗]
      container (editor surface)
        RichTextEditor widget
      row
        [conditional] button ("Delete") - danger style
        Space (fill)
        button ("Save") - primary style
```

### Compose window signature region

```
column (compose document blocks)
  ... user content blocks ...
  horizontal_rule (signature separator)
  ... signature blocks (rendered by the editor) ...
  paragraph (attribution line - italic)
  blockquote (quoted content)
```

---

## Migration

No new database migration is needed. The `signatures` table and all necessary
columns already exist (v1 + v41). The only data model changes are in the Rust
struct (`DbSignature`) and its `FromRow` impl.

---

## Testing Strategy

### Unit tests (no GUI)

- `html_to_plain_text`: verify tag stripping, entity decoding, block-element
  newline insertion
- `db_resolve_signature_for_compose`: test alias override, reply-default
  fallback, no-default case
- `assemble_compose_document`: verify block ordering for new/reply/forward
  scenarios, signature separator index correctness
- Signature replacement: verify block removal/insertion, separator index
  tracking, edge cases (no old signature, no new signature, both)
- HTML round-trip: parse a signature's HTML, serialize back, verify content
  preservation (this exercises the editor's html_parse + html_serialize)

### Integration tests (with DB)

- CRUD lifecycle: insert, read, update, delete signature
- Default management: setting default clears previous, per-account isolation
- Reply-default fallback: verify `db_get_reply_signature` falls back correctly
- Draft persistence: save draft with signature, restore, verify signature
  blocks and ID

### Manual testing

- Create signatures with formatting (bold, italic, links, lists)
- Verify the editor preserves formatting on save and re-open (HTML round-trip)
- Switch From accounts in compose, verify signature replacement
- Edit signature in compose body, switch From, verify confirmation dialog
- Reply to a message, verify signature appears between reply area and quoted
  content
- Delete a signature that is set as default, verify the account has no default
- Server-synced signatures (Gmail/JMAP) appear in the list and are editable

---

## Phasing Summary

| Phase | Scope | Blocked by |
|-------|-------|------------|
| 1 | Data model: extend `DbSignature`, add missing queries, resolution logic | Nothing |
| 2 | Settings UI: signature list, editor overlay with rich text editor | Editor Phase 3 (HTML round-trip) |
| 3 | Compose insertion: document assembly, separator tracking, per-scenario placement | Phase 1 + compose window implementation |
| 4 | Account switching: signature replacement, edit detection, confirmation dialog | Phase 3 |
| 5 | Send path: HTML wrapping, plain-text separator, draft persistence | Phase 3 |

Phases 1 and 2 can proceed in parallel once the editor reaches Phase 3.
Phase 3 requires the compose window to exist (from the pop-out windows spec).
Phases 4 and 5 are incremental additions to Phase 3.
