use iced::Point;

/// A single token displayed inline in the input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Unique ID for this token instance.
    pub id: TokenId,
    /// The email address this token represents.
    pub email: String,
    /// Display label shown on the token chip.
    pub label: String,
    /// Whether this token represents a contact group.
    pub is_group: bool,
    /// Group ID if this is a group token (for expand operations).
    pub group_id: Option<String>,
    /// Member count for group tokens (displayed as suffix).
    pub member_count: Option<i64>,
}

/// Opaque token identifier. Wraps a u64 counter, monotonically increasing
/// per widget instance to guarantee uniqueness across add/remove cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenId(pub u64);

/// Which recipient field this widget instance represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // re-exported alias of compose::RecipientField; consumers haven't switched yet
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// Persistent state owned by the caller (lives in the compose model).
/// Passed as data to the widget constructor - the widget does not own this.
pub struct TokenInputValue {
    /// Current tokens in this field.
    pub tokens: Vec<Token>,
    /// Current text being typed (after the last token).
    pub text: String,
    /// Next token ID counter.
    pub next_id: u64,
}

impl TokenInputValue {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            text: String::new(),
            next_id: 0,
        }
    }

    pub fn next_token_id(&mut self) -> TokenId {
        let id = TokenId(self.next_id);
        self.next_id += 1;
        id
    }
}

impl Default for TokenInputValue {
    fn default() -> Self {
        Self::new()
    }
}

/// Messages emitted by the token input widget upward to the caller.
#[derive(Debug, Clone)]
pub enum TokenInputMessage {
    /// The text input content changed.
    TextChanged(String),
    /// A token should be removed.
    RemoveToken(TokenId),
    /// Raw text should be tokenized (triggered by delimiter keys).
    TokenizeText(String),
    /// A token was clicked (selected).
    SelectToken(TokenId),
    /// Click in empty area - deselect any token.
    DeselectTokens,
    /// Focus was gained by this field.
    Focused,
    /// Focus was lost.
    Blurred,
    /// A paste event with raw text. Caller parses and tokenizes.
    Paste(String),
    /// Backspace was pressed at the start of the text input.
    BackspaceAtStart,
    /// Right-click on a token - emit position for context menu.
    TokenContextMenu(TokenId, Point),
    /// Right-click in the field's empty area (no token under the cursor) -
    /// caller shows a minimal Paste-only context menu.
    FieldContextMenu(Point),
    /// Ctrl+C / Cmd+C with a selected token - copy it to the clipboard.
    /// The actual clipboard write happens at the compose layer because group
    /// tokens require a DB lookup to expand into their member list.
    CopyToken(TokenId),
    /// Ctrl+X / Cmd+X with a selected token - copy then delete.
    CutToken(TokenId),
    /// Arrow key navigated to select a token by index.
    ArrowSelectToken(TokenId),
    /// Arrow right from last token - deselect and focus text.
    ArrowToText,
    /// A drag was initiated on a token (exceeded 4px threshold).
    DragStarted(TokenId),
    // ── Autocomplete keyboard events (emitted when autocomplete_open) ──
    /// Arrow down when autocomplete dropdown is visible.
    AutocompleteDown,
    /// Arrow up when autocomplete dropdown is visible.
    AutocompleteUp,
    /// Enter/Tab when autocomplete dropdown is visible - accept selection.
    AutocompleteAccept,
    /// Escape when autocomplete dropdown is visible - dismiss dropdown.
    AutocompleteDismissKey,
}
