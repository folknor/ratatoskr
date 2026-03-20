//! Document model for the rich text editor.
//!
//! Block tree with inline runs. Maps naturally to both HTML (DOM) and iced
//! rendering (column of `Paragraph` widgets).
//!
//! Blocks are wrapped in `Arc` for structural sharing: after an edit, only the
//! affected block gets a new allocation. Unchanged blocks are `Arc::clone`.

use std::ops::Range;
use std::sync::Arc;

use bitflags::bitflags;

// ── Inline style ────────────────────────────────────────

bitflags! {
    /// Inline formatting flags. Composable via bitwise OR.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct InlineStyle: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const UNDERLINE     = 0b0000_0100;
        const STRIKETHROUGH = 0b0000_1000;
    }
}

// ── Styled run ──────────────────────────────────────────

/// A contiguous run of text sharing the same inline style.
///
/// The normalization invariant guarantees that adjacent runs within the same
/// block always have different `(style, link)` pairs. After every edit,
/// adjacent runs with identical formatting merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyledRun {
    pub text: String,
    pub style: InlineStyle,
    /// Optional hyperlink href. `None` means plain text.
    pub link: Option<String>,
}

impl StyledRun {
    /// Create a plain (unstyled, unlinked) run.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: InlineStyle::empty(),
            link: None,
        }
    }

    /// Create a styled run with no link.
    pub fn styled(text: impl Into<String>, style: InlineStyle) -> Self {
        Self {
            text: text.into(),
            style,
            link: None,
        }
    }

    /// Create a linked run.
    pub fn linked(text: impl Into<String>, style: InlineStyle, href: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style,
            link: Some(href.into()),
        }
    }

    /// The number of characters in this run.
    pub fn char_len(&self) -> usize {
        self.text.chars().count()
    }

    /// Whether this run has the same formatting (style + link) as `other`.
    pub fn same_formatting(&self, other: &Self) -> bool {
        self.style == other.style && self.link == other.link
    }

    /// Whether this run is empty (zero characters).
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Split this run at a character offset, returning `(left, right)`.
    ///
    /// Panics if `char_offset` is out of bounds (but 0 and char_len are valid
    /// — they produce an empty left or right respectively).
    pub fn split_at(&self, char_offset: usize) -> (Self, Self) {
        let byte_offset = self.char_to_byte_offset(char_offset);
        let (left_text, right_text) = self.text.split_at(byte_offset);
        (
            Self {
                text: left_text.to_owned(),
                style: self.style,
                link: self.link.clone(),
            },
            Self {
                text: right_text.to_owned(),
                style: self.style,
                link: self.link.clone(),
            },
        )
    }

    /// Convert a char offset to a byte offset within this run's text.
    pub(crate) fn char_to_byte_offset(&self, char_offset: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_offset)
            .map_or(self.text.len(), |(byte_idx, _)| byte_idx)
    }
}

// ── Heading level ───────────────────────────────────────

/// Heading levels supported in email composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingLevel {
    H1,
    H2,
    H3,
}

impl HeadingLevel {
    /// Return the numeric level (1, 2, or 3).
    pub fn as_u8(self) -> u8 {
        match self {
            Self::H1 => 1,
            Self::H2 => 2,
            Self::H3 => 3,
        }
    }

    /// Create from a numeric level. Returns `None` for invalid levels.
    pub fn from_u8(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::H1),
            2 => Some(Self::H2),
            3 => Some(Self::H3),
            _ => None,
        }
    }
}

// ── Block ───────────────────────────────────────────────

/// A block-level element in the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A paragraph of styled text runs.
    Paragraph { runs: Vec<StyledRun> },

    /// A heading (H1–H3) with styled text runs.
    Heading {
        level: HeadingLevel,
        runs: Vec<StyledRun>,
    },

    /// An ordered or unordered list.
    List {
        ordered: bool,
        items: Vec<ListItem>,
    },

    /// A block quote containing nested blocks.
    BlockQuote { blocks: Vec<Arc<Block>> },

    /// A horizontal rule (thematic break).
    HorizontalRule,
}

impl Block {
    /// Create an empty paragraph (the default block type).
    pub fn empty_paragraph() -> Self {
        Self::Paragraph {
            runs: vec![StyledRun::plain(String::new())],
        }
    }

    /// Create a paragraph from plain text.
    pub fn paragraph(text: impl Into<String>) -> Self {
        Self::Paragraph {
            runs: vec![StyledRun::plain(text)],
        }
    }

    /// Get the inline runs for this block, if it has any.
    ///
    /// Returns `Some` for `Paragraph` and `Heading`, `None` for all others.
    pub fn runs(&self) -> Option<&[StyledRun]> {
        match self {
            Self::Paragraph { runs } | Self::Heading { runs, .. } => Some(runs),
            _ => None,
        }
    }

    /// Get a mutable reference to the inline runs, if this block has any.
    pub fn runs_mut(&mut self) -> Option<&mut Vec<StyledRun>> {
        match self {
            Self::Paragraph { runs } | Self::Heading { runs, .. } => Some(runs),
            _ => None,
        }
    }

    /// Concatenate all run text into a single string.
    ///
    /// For blocks without inline runs (List, BlockQuote, HorizontalRule),
    /// returns an empty string.
    pub fn flattened_text(&self) -> String {
        match self.runs() {
            Some(runs) => {
                let total_len: usize = runs.iter().map(|r| r.text.len()).sum();
                let mut buf = String::with_capacity(total_len);
                for run in runs {
                    buf.push_str(&run.text);
                }
                buf
            }
            None => String::new(),
        }
    }

    /// Total character count of the block's flattened inline text.
    pub fn char_len(&self) -> usize {
        match self.runs() {
            Some(runs) => runs.iter().map(StyledRun::char_len).sum(),
            None => 0,
        }
    }

    /// Whether this block is a "leaf" block (has inline content, not children).
    pub fn is_inline_block(&self) -> bool {
        matches!(self, Self::Paragraph { .. } | Self::Heading { .. })
    }

    /// Whether this block is a structural container (List, BlockQuote).
    pub fn is_container(&self) -> bool {
        matches!(self, Self::List { .. } | Self::BlockQuote { .. })
    }

    /// The `BlockKind` discriminant (type without data). Used by `SetBlockType`.
    pub fn kind(&self) -> BlockKind {
        match self {
            Self::Paragraph { .. } => BlockKind::Paragraph,
            Self::Heading { level, .. } => BlockKind::Heading(*level),
            Self::List { ordered, .. } => BlockKind::List(*ordered),
            Self::BlockQuote { .. } => BlockKind::BlockQuote,
            Self::HorizontalRule => BlockKind::HorizontalRule,
        }
    }

    /// Resolve a flattened char offset to `(run_index, char_offset_within_run)`.
    ///
    /// Returns `None` if this block has no inline runs or the offset is out of
    /// bounds. An offset equal to the block's `char_len()` resolves to the end
    /// of the last run.
    pub fn resolve_offset(&self, offset: usize) -> Option<(usize, usize)> {
        let runs = self.runs()?;
        let mut remaining = offset;
        for (i, run) in runs.iter().enumerate() {
            let len = run.char_len();
            if remaining < len || (remaining == len && i == runs.len() - 1) {
                return Some((i, remaining));
            }
            remaining -= len;
        }
        // offset is beyond the end — this shouldn't normally happen after
        // validation, but return None rather than panicking.
        None
    }
}

// ── Block kind (discriminant without data) ──────────────

/// The type of a block without its content. Used in `SetBlockType` operations
/// to record what a block changed from/to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Paragraph,
    Heading(HeadingLevel),
    List(bool),
    BlockQuote,
    HorizontalRule,
}

// ── List item ───────────────────────────────────────────

/// A single item in a list. Usually contains one paragraph, but can nest
/// further blocks (including sub-lists).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub blocks: Vec<Arc<Block>>,
}

impl ListItem {
    /// Create a list item from a single paragraph block.
    pub fn from_paragraph(block: Block) -> Self {
        Self {
            blocks: vec![Arc::new(block)],
        }
    }

    /// Create a list item from plain text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self::from_paragraph(Block::paragraph(text))
    }
}

// ── Document position ───────────────────────────────────

/// A position within the document: block index + char offset within that
/// block's flattened inline text.
///
/// The char offset is computed by concatenating all `StyledRun` texts within
/// the block. This remains stable across run restructuring (splitting/merging
/// runs doesn't change the flattened text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocPosition {
    pub block_index: usize,
    pub offset: usize,
}

impl DocPosition {
    pub fn new(block_index: usize, offset: usize) -> Self {
        Self {
            block_index,
            offset,
        }
    }

    /// Position at the start of the document.
    pub fn zero() -> Self {
        Self {
            block_index: 0,
            offset: 0,
        }
    }
}

impl PartialOrd for DocPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DocPosition {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.block_index
            .cmp(&other.block_index)
            .then(self.offset.cmp(&other.offset))
    }
}

// ── Document selection ──────────────────────────────────

/// A selection within the document, defined by an anchor (where the selection
/// started) and a focus (where the caret visually is).
///
/// When `anchor == focus`, this represents a collapsed caret (no selection).
/// The focus can be before or after the anchor — the selection direction
/// matters for extending selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocSelection {
    /// Where the selection started (fixed end).
    pub anchor: DocPosition,
    /// Where the caret is (moving end, can be before or after anchor).
    pub focus: DocPosition,
}

impl DocSelection {
    /// A collapsed caret at the given position.
    pub fn caret(pos: DocPosition) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    /// A selection from anchor to focus.
    pub fn range(anchor: DocPosition, focus: DocPosition) -> Self {
        Self { anchor, focus }
    }

    /// Whether this selection is collapsed (caret, no range selected).
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The earlier of anchor and focus (the "start" of the selected range).
    pub fn start(&self) -> DocPosition {
        std::cmp::min(self.anchor, self.focus)
    }

    /// The later of anchor and focus (the "end" of the selected range).
    pub fn end(&self) -> DocPosition {
        std::cmp::max(self.anchor, self.focus)
    }

    /// The range of block indices that this selection spans (inclusive).
    pub fn block_range(&self) -> Range<usize> {
        let s = self.start();
        let e = self.end();
        s.block_index..e.block_index + 1
    }
}

// ── Document slice (clipboard) ──────────────────────────

/// A fragment of a document, used for clipboard copy/paste.
///
/// `open_start` / `open_end` indicate whether the first/last block is a
/// fragment (partial paragraph) rather than a complete block. This is simpler
/// than ProseMirror's arbitrary-depth open counts because our tree is only
/// two levels deep (document → blocks → runs).
///
/// When pasting:
/// - If `open_start`: merge the first slice block's runs into the block at the
///   cursor position (after splitting it at the cursor offset)
/// - Middle blocks insert as-is
/// - If `open_end`: merge the last slice block's runs into the block after the
///   split
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSlice {
    pub blocks: Vec<Block>,
    pub open_start: bool,
    pub open_end: bool,
}

impl DocSlice {
    /// A slice representing a single complete block.
    pub fn single(block: Block) -> Self {
        Self {
            blocks: vec![block],
            open_start: false,
            open_end: false,
        }
    }

    /// A slice representing inline content from within a single block
    /// (both ends open — will merge into the target block).
    pub fn inline_fragment(runs: Vec<StyledRun>) -> Self {
        Self {
            blocks: vec![Block::Paragraph { runs }],
            open_start: true,
            open_end: true,
        }
    }
}

// ── Document ────────────────────────────────────────────

/// The root document. A sequence of blocks with `Arc` structural sharing.
///
/// Invariants (enforced by normalization):
/// - Always contains at least one block.
/// - Every inline block has at least one run (may be empty-string run).
/// - Adjacent runs within a block have different `(style, link)` pairs.
/// - `List` items each contain at least one block.
/// - `BlockQuote` contains at least one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Arc<Block>>,
}

impl Document {
    /// Create a new empty document (single empty paragraph).
    pub fn new() -> Self {
        Self {
            blocks: vec![Arc::new(Block::empty_paragraph())],
        }
    }

    /// Create a document from a list of blocks.
    ///
    /// If `blocks` is empty, creates a document with a single empty paragraph.
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            return Self::new();
        }
        Self {
            blocks: blocks.into_iter().map(Arc::new).collect(),
        }
    }

    /// The number of blocks in the document.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Get a reference to the block at the given index.
    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index).map(AsRef::as_ref)
    }

    /// Replace the block at `index` with a new block. Returns the old Arc.
    ///
    /// This is the fundamental mutation primitive: create a new block, replace
    /// the Arc at the given index. All other Arcs remain shared.
    pub fn replace_block(&mut self, index: usize, block: Block) -> Option<Arc<Block>> {
        self.blocks
            .get_mut(index)
            .map(|slot| std::mem::replace(slot, Arc::new(block)))
    }

    /// Insert a block at the given index, shifting subsequent blocks right.
    pub fn insert_block(&mut self, index: usize, block: Block) {
        self.blocks.insert(index, Arc::new(block));
    }

    /// Remove the block at the given index. Returns the removed block.
    ///
    /// Will not remove the last block (returns `None` if that would happen).
    pub fn remove_block(&mut self, index: usize) -> Option<Arc<Block>> {
        if self.blocks.len() <= 1 {
            return None;
        }
        Some(self.blocks.remove(index))
    }

    /// The total character count across all inline blocks.
    pub fn total_char_len(&self) -> usize {
        self.blocks.iter().map(|b| b.char_len()).sum()
    }

    /// Concatenate all inline text in the document.
    pub fn flattened_text(&self) -> String {
        let mut buf = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 {
                buf.push('\n');
            }
            buf.push_str(&block.flattened_text());
        }
        buf
    }

    /// Extract a `DocSlice` for the given selection range.
    ///
    /// The returned slice has `open_start = true` if the selection starts
    /// mid-block, and `open_end = true` if it ends mid-block.
    pub fn slice(&self, start: DocPosition, end: DocPosition) -> Option<DocSlice> {
        if start >= end {
            return None;
        }

        let mut blocks = Vec::new();
        let start_block = self.block(start.block_index)?;
        let end_block = self.block(end.block_index)?;

        if start.block_index == end.block_index {
            // Single-block selection: extract runs in the range
            if let Some(runs) = start_block.runs() {
                let extracted = extract_runs(runs, start.offset, end.offset);
                return Some(DocSlice {
                    blocks: vec![Block::Paragraph { runs: extracted }],
                    open_start: true,
                    open_end: true,
                });
            }
            return None;
        }

        // Multi-block selection
        let open_start = start.offset > 0;
        let open_end = end.offset < end_block.char_len();

        // First block (possibly partial)
        if let Some(runs) = start_block.runs() {
            let extracted = extract_runs(runs, start.offset, start_block.char_len());
            blocks.push(Block::Paragraph { runs: extracted });
        } else {
            blocks.push(start_block.clone());
        }

        // Middle blocks (complete)
        for i in (start.block_index + 1)..end.block_index {
            if let Some(block) = self.block(i) {
                blocks.push(block.clone());
            }
        }

        // Last block (possibly partial)
        if let Some(runs) = end_block.runs() {
            let extracted = extract_runs(runs, 0, end.offset);
            blocks.push(Block::Paragraph { runs: extracted });
        } else {
            blocks.push(end_block.clone());
        }

        Some(DocSlice {
            blocks,
            open_start,
            open_end,
        })
    }

    /// Validate that a `DocPosition` is within bounds.
    pub fn is_valid_position(&self, pos: DocPosition) -> bool {
        if let Some(block) = self.block(pos.block_index) {
            pos.offset <= block.char_len()
        } else {
            false
        }
    }

    /// Clamp a position to valid bounds.
    pub fn clamp_position(&self, pos: DocPosition) -> DocPosition {
        let block_index = pos.block_index.min(self.blocks.len().saturating_sub(1));
        let max_offset = self
            .block(block_index)
            .map_or(0, Block::char_len);
        DocPosition::new(block_index, pos.offset.min(max_offset))
    }

    /// Position at the very end of the document.
    pub fn end_position(&self) -> DocPosition {
        let last_idx = self.blocks.len().saturating_sub(1);
        let last_len = self.block(last_idx).map_or(0, Block::char_len);
        DocPosition::new(last_idx, last_len)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ─────────────────────────────────────────────

/// Extract runs covering the character range `[start_offset..end_offset)` from
/// a slice of runs. The result preserves styles and links, splitting runs at
/// the boundaries.
fn extract_runs(runs: &[StyledRun], start_offset: usize, end_offset: usize) -> Vec<StyledRun> {
    if start_offset >= end_offset {
        return vec![StyledRun::plain(String::new())];
    }

    let mut result = Vec::new();
    let mut pos = 0;

    for run in runs {
        let run_len = run.char_len();
        let run_start = pos;
        let run_end = pos + run_len;

        // Skip runs entirely before the range
        if run_end <= start_offset {
            pos = run_end;
            continue;
        }
        // Stop at runs entirely after the range
        if run_start >= end_offset {
            break;
        }

        // Compute the overlap
        let overlap_start = start_offset.max(run_start) - run_start;
        let overlap_end = end_offset.min(run_end) - run_start;

        // Extract the overlapping substring
        let byte_start = run.char_to_byte_offset(overlap_start);
        let byte_end = run.char_to_byte_offset(overlap_end);
        let text = run.text[byte_start..byte_end].to_owned();

        result.push(StyledRun {
            text,
            style: run.style,
            link: run.link.clone(),
        });

        pos = run_end;
    }

    if result.is_empty() {
        result.push(StyledRun::plain(String::new()));
    }

    result
}

// ── Run utilities used by operations ────────────────────

/// Isolate the runs covering character range `[start..end)` within a run list.
///
/// Splits runs at the start and end boundaries so that the returned index range
/// covers exactly the characters in `[start..end)`. The runs Vec is mutated
/// in-place (runs may be split into 2 or 3 pieces).
///
/// Returns the `Range<usize>` of run indices covering the isolated region.
pub fn isolate_runs(runs: &mut Vec<StyledRun>, start: usize, end: usize) -> Range<usize> {
    assert!(start <= end, "isolate_runs: start ({start}) > end ({end})");

    if runs.is_empty() || start == end {
        return 0..0;
    }

    // Split at `start` first, then `end`. Track count changes so indices
    // remain consistent.
    let len_before = runs.len();
    let start_idx = split_runs_at(runs, start);
    let inserted_by_start = runs.len() - len_before;

    let len_before = runs.len();
    let end_idx = split_runs_at(runs, end);
    let _ = runs.len() - len_before; // for clarity

    // If splitting at `start` inserted a run, `end_idx` is already correct
    // because `split_runs_at` operates on the updated Vec.
    let _ = inserted_by_start;

    start_idx..end_idx
}

/// Split the run list at a character offset, returning the run index at which
/// the split falls. If the offset falls on an existing run boundary, no split
/// occurs; the existing boundary index is returned.
///
/// After this call, `offset` corresponds to the start of `runs[returned_index]`.
///
/// Public alias: [`split_runs_at_char_offset`].
fn split_runs_at(runs: &mut Vec<StyledRun>, offset: usize) -> usize {
    let mut pos = 0;
    for i in 0..runs.len() {
        let run_len = runs[i].char_len();
        if pos == offset {
            return i;
        }
        if pos + run_len > offset {
            // Split this run
            let local_offset = offset - pos;
            let (left, right) = runs[i].split_at(local_offset);
            runs[i] = left;
            runs.insert(i + 1, right);
            return i + 1;
        }
        pos += run_len;
    }
    // offset == total length → past the end
    runs.len()
}

/// Public wrapper for [`split_runs_at`]. Splits a run list at a character
/// offset, returning the run index where the split falls. Used by the paste
/// path to splice runs into a block.
pub fn split_runs_at_char_offset(runs: &mut Vec<StyledRun>, offset: usize) -> usize {
    split_runs_at(runs, offset)
}

// ── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_has_one_block() {
        let doc = Document::new();
        assert_eq!(doc.block_count(), 1);
        assert_eq!(doc.block(0).map(Block::char_len), Some(0));
    }

    #[test]
    fn styled_run_split_at() {
        let run = StyledRun::styled("hello world", InlineStyle::BOLD);
        let (left, right) = run.split_at(5);
        assert_eq!(left.text, "hello");
        assert_eq!(right.text, " world");
        assert_eq!(left.style, InlineStyle::BOLD);
        assert_eq!(right.style, InlineStyle::BOLD);
    }

    #[test]
    fn styled_run_split_at_unicode() {
        let run = StyledRun::plain("cafe\u{0301}!"); // café! (e + combining accent)
        // chars: 'c' 'a' 'f' 'e' '\u{0301}' '!'
        let (left, right) = run.split_at(4);
        assert_eq!(left.text, "cafe");
        assert_eq!(right.text, "\u{0301}!");
    }

    #[test]
    fn block_resolve_offset() {
        let block = Block::Paragraph {
            runs: vec![
                StyledRun::plain("hello"),  // chars 0..5
                StyledRun::styled(" world", InlineStyle::BOLD), // chars 5..11
            ],
        };
        assert_eq!(block.resolve_offset(0), Some((0, 0)));
        assert_eq!(block.resolve_offset(4), Some((0, 4)));
        assert_eq!(block.resolve_offset(5), Some((1, 0)));
        assert_eq!(block.resolve_offset(10), Some((1, 5)));
        assert_eq!(block.resolve_offset(11), Some((1, 6)));  // end of last run
        assert_eq!(block.resolve_offset(12), None);           // out of bounds
    }

    #[test]
    fn doc_position_ordering() {
        let a = DocPosition::new(0, 5);
        let b = DocPosition::new(1, 0);
        let c = DocPosition::new(1, 3);
        assert!(a < b);
        assert!(b < c);
        assert!(a < c);
    }

    #[test]
    fn selection_start_end() {
        // Forward selection
        let sel = DocSelection::range(DocPosition::new(0, 5), DocPosition::new(1, 3));
        assert_eq!(sel.start(), DocPosition::new(0, 5));
        assert_eq!(sel.end(), DocPosition::new(1, 3));

        // Backward selection
        let sel = DocSelection::range(DocPosition::new(1, 3), DocPosition::new(0, 5));
        assert_eq!(sel.start(), DocPosition::new(0, 5));
        assert_eq!(sel.end(), DocPosition::new(1, 3));
    }

    #[test]
    fn isolate_runs_splits_correctly() {
        let mut runs = vec![
            StyledRun::plain("hello"),      // 0..5
            StyledRun::styled(" world", InlineStyle::BOLD), // 5..11
        ];
        let range = isolate_runs(&mut runs, 3, 8);
        // Should split "hello" at 3 → "hel" + "lo"
        // Should split " world" at 8-5=3 → " wo" + "rld"
        // Isolated range is "lo" + " wo"
        assert_eq!(runs.len(), 4);
        assert_eq!(runs[0].text, "hel");
        assert_eq!(runs[1].text, "lo");
        assert_eq!(runs[2].text, " wo");
        assert_eq!(runs[3].text, "rld");
        assert_eq!(range, 1..3);
    }

    #[test]
    fn isolate_runs_at_boundary() {
        let mut runs = vec![
            StyledRun::plain("hello"),      // 0..5
            StyledRun::styled(" world", InlineStyle::BOLD), // 5..11
        ];
        // Isolate exactly the second run
        let range = isolate_runs(&mut runs, 5, 11);
        assert_eq!(runs.len(), 2); // no splits needed
        assert_eq!(range, 1..2);
    }

    #[test]
    fn document_slice_single_block() {
        let doc = Document::from_blocks(vec![
            Block::Paragraph {
                runs: vec![
                    StyledRun::plain("hello"),
                    StyledRun::styled(" world", InlineStyle::BOLD),
                ],
            },
        ]);
        let slice = doc
            .slice(DocPosition::new(0, 2), DocPosition::new(0, 8))
            .expect("slice should succeed");
        assert!(slice.open_start);
        assert!(slice.open_end);
        assert_eq!(slice.blocks.len(), 1);
        let text: String = slice.blocks[0]
            .runs()
            .into_iter()
            .flatten()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(text, "llo wo");
    }

    #[test]
    fn document_replace_block_structural_sharing() {
        let mut doc = Document::from_blocks(vec![
            Block::paragraph("first"),
            Block::paragraph("second"),
            Block::paragraph("third"),
        ]);
        let original_third = Arc::clone(&doc.blocks[2]);

        // Replace the second block
        doc.replace_block(1, Block::paragraph("replaced"));

        // Third block's Arc should be the same pointer
        assert!(Arc::ptr_eq(&doc.blocks[2], &original_third));
        assert_eq!(doc.block(1).map(Block::flattened_text).as_deref(), Some("replaced"));
    }

    #[test]
    fn extract_runs_preserves_style() {
        let runs = vec![
            StyledRun::plain("aaa"),                               // 0..3
            StyledRun::styled("bbb", InlineStyle::BOLD),           // 3..6
            StyledRun::styled("ccc", InlineStyle::ITALIC),         // 6..9
        ];
        let extracted = extract_runs(&runs, 2, 7);
        assert_eq!(extracted.len(), 3);
        assert_eq!(extracted[0].text, "a");
        assert_eq!(extracted[0].style, InlineStyle::empty());
        assert_eq!(extracted[1].text, "bbb");
        assert_eq!(extracted[1].style, InlineStyle::BOLD);
        assert_eq!(extracted[2].text, "c");
        assert_eq!(extracted[2].style, InlineStyle::ITALIC);
    }
}
