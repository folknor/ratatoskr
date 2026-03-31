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

// ── Text alignment ──────────────────────────────────────

/// Text alignment for block-level elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextAlignment {
    /// Left-aligned (the default for LTR text).
    #[default]
    Left,
    /// Center-aligned.
    Center,
    /// Right-aligned.
    Right,
}

// ── Block attributes ───────────────────────────────────

/// Block-level attributes that can be changed independently of the block type.
///
/// Used by `SetBlockAttrs` to modify alignment and list indent without
/// changing the block variant (Paragraph, Heading, ListItem, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockAttrs {
    /// Text alignment for the block content.
    pub alignment: TextAlignment,
    /// Indent level (meaningful for ListItem; stored for all blocks but only
    /// affects rendering of list items).
    pub indent_level: u8,
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

    /// A single list item (bullet or numbered).
    ///
    /// Each list item is a top-level, cursor-addressable block. Consecutive
    /// `ListItem` blocks with matching `ordered` flags are grouped into a
    /// single `<ul>`/`<ol>` during HTML serialization.
    ListItem {
        ordered: bool,
        indent_level: u8,
        runs: Vec<StyledRun>,
    },

    /// A block quote containing nested blocks.
    BlockQuote { blocks: Vec<Arc<Block>> },

    /// A horizontal rule (thematic break).
    HorizontalRule,

    /// An inline image (block embed).
    ///
    /// Images are atomic blocks with no inline text content. The editor
    /// stores the `src` reference; the host application provides image data
    /// at render time.
    Image {
        /// Image source: URL, data-URI, `cid:`, or `inline-image:<hash>`.
        src: String,
        /// Alt text for accessibility and placeholder display.
        alt: String,
        /// Optional explicit width in pixels.
        width: Option<u32>,
        /// Optional explicit height in pixels.
        height: Option<u32>,
    },
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

    /// Create a list item from plain text.
    pub fn list_item(text: impl Into<String>, ordered: bool) -> Self {
        Self::ListItem {
            ordered,
            indent_level: 0,
            runs: vec![StyledRun::plain(text)],
        }
    }

    /// Create a list item with a specific indent level.
    pub fn list_item_with_indent(text: impl Into<String>, ordered: bool, indent_level: u8) -> Self {
        Self::ListItem {
            ordered,
            indent_level,
            runs: vec![StyledRun::plain(text)],
        }
    }

    /// Get the inline runs for this block, if it has any.
    ///
    /// Returns `Some` for `Paragraph`, `Heading`, and `ListItem`;
    /// `None` for all others.
    pub fn runs(&self) -> Option<&[StyledRun]> {
        match self {
            Self::Paragraph { runs } | Self::Heading { runs, .. } | Self::ListItem { runs, .. } => {
                Some(runs)
            }
            _ => None,
        }
    }

    /// Get a mutable reference to the inline runs, if this block has any.
    pub fn runs_mut(&mut self) -> Option<&mut Vec<StyledRun>> {
        match self {
            Self::Paragraph { runs } | Self::Heading { runs, .. } | Self::ListItem { runs, .. } => {
                Some(runs)
            }
            _ => None,
        }
    }

    /// Concatenate all run text into a single string.
    ///
    /// For blocks without inline runs (BlockQuote, HorizontalRule),
    /// returns an empty string. For Image blocks, returns the alt text.
    pub fn flattened_text(&self) -> String {
        match self {
            Self::Image { alt, .. } => alt.clone(),
            _ => match self.runs() {
                Some(runs) => {
                    let total_len: usize = runs.iter().map(|r| r.text.len()).sum();
                    let mut buf = String::with_capacity(total_len);
                    for run in runs {
                        buf.push_str(&run.text);
                    }
                    buf
                }
                None => String::new(),
            },
        }
    }

    /// Total character count of the block's flattened inline text.
    ///
    /// Images are atomic and return 0 (not character-addressable).
    pub fn char_len(&self) -> usize {
        match self.runs() {
            Some(runs) => runs.iter().map(StyledRun::char_len).sum(),
            None => 0,
        }
    }

    /// Whether this block is a "leaf" block (has inline content, not children).
    pub fn is_inline_block(&self) -> bool {
        matches!(
            self,
            Self::Paragraph { .. } | Self::Heading { .. } | Self::ListItem { .. }
        )
    }

    /// Whether this block is a structural container (BlockQuote).
    pub fn is_container(&self) -> bool {
        matches!(self, Self::BlockQuote { .. })
    }

    /// Extract the block-level attributes from this block.
    ///
    /// Indent level is only meaningful for `ListItem` (returns 0 for others).
    /// Alignment defaults to `Left` (stored alignment is a future extension).
    pub fn attrs(&self) -> BlockAttrs {
        match self {
            Self::ListItem { indent_level, .. } => BlockAttrs {
                alignment: TextAlignment::Left,
                indent_level: *indent_level,
            },
            _ => BlockAttrs::default(),
        }
    }

    /// Return a clone of this block with the given attributes applied.
    ///
    /// Only attributes that are meaningful for the block type are applied:
    /// - `indent_level` is applied to `ListItem` blocks.
    /// - `alignment` is stored for future use (not yet rendered).
    ///
    /// Returns `None` if the block type does not support any attributes
    /// (e.g., `HorizontalRule`, `Image`).
    pub fn with_attrs(&self, attrs: BlockAttrs) -> Option<Self> {
        match self {
            Self::ListItem { ordered, runs, .. } => Some(Self::ListItem {
                ordered: *ordered,
                indent_level: attrs.indent_level,
                runs: runs.clone(),
            }),
            Self::Paragraph { .. } | Self::Heading { .. } | Self::BlockQuote { .. } => {
                // These block types don't currently have mutable attrs fields,
                // but we accept the call to allow uniform handling.
                // Indent level changes are silently ignored for non-list blocks.
                Some(self.clone())
            }
            Self::HorizontalRule | Self::Image { .. } => None,
        }
    }

    /// The `BlockKind` discriminant (type without data). Used by `SetBlockType`.
    pub fn kind(&self) -> BlockKind {
        match self {
            Self::Paragraph { .. } => BlockKind::Paragraph,
            Self::Heading { level, .. } => BlockKind::Heading(*level),
            Self::ListItem { ordered, .. } => BlockKind::ListItem { ordered: *ordered },
            Self::BlockQuote { .. } => BlockKind::BlockQuote,
            Self::HorizontalRule => BlockKind::HorizontalRule,
            Self::Image { .. } => BlockKind::Image,
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
    ListItem { ordered: bool },
    BlockQuote,
    HorizontalRule,
    Image,
}

// ── Document position ───────────────────────────────────

/// A position within the document: block index + char offset within that
/// block's flattened inline text.
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

/// A selection within the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DocSelection {
    pub anchor: DocPosition,
    pub focus: DocPosition,
}

impl DocSelection {
    pub fn caret(pos: DocPosition) -> Self {
        Self {
            anchor: pos,
            focus: pos,
        }
    }

    pub fn range(anchor: DocPosition, focus: DocPosition) -> Self {
        Self { anchor, focus }
    }

    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    pub fn start(&self) -> DocPosition {
        std::cmp::min(self.anchor, self.focus)
    }

    pub fn end(&self) -> DocPosition {
        std::cmp::max(self.anchor, self.focus)
    }

    pub fn block_range(&self) -> Range<usize> {
        let s = self.start();
        let e = self.end();
        s.block_index..e.block_index + 1
    }
}

// ── Document slice (clipboard) ──────────────────────────

/// A fragment of a document, used for clipboard copy/paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSlice {
    pub blocks: Vec<Block>,
    pub open_start: bool,
    pub open_end: bool,
}

impl DocSlice {
    pub fn single(block: Block) -> Self {
        Self {
            blocks: vec![block],
            open_start: false,
            open_end: false,
        }
    }

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
/// - `BlockQuote` contains at least one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Arc<Block>>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            blocks: vec![Arc::new(Block::empty_paragraph())],
        }
    }

    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        if blocks.is_empty() {
            return Self::new();
        }
        Self {
            blocks: blocks.into_iter().map(Arc::new).collect(),
        }
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn block(&self, index: usize) -> Option<&Block> {
        self.blocks.get(index).map(AsRef::as_ref)
    }

    pub fn replace_block(&mut self, index: usize, block: Block) -> Option<Arc<Block>> {
        self.blocks
            .get_mut(index)
            .map(|slot| std::mem::replace(slot, Arc::new(block)))
    }

    pub fn insert_block(&mut self, index: usize, block: Block) {
        self.blocks.insert(index, Arc::new(block));
    }

    pub fn remove_block(&mut self, index: usize) -> Option<Arc<Block>> {
        if self.blocks.len() <= 1 {
            return None;
        }
        Some(self.blocks.remove(index))
    }

    pub fn total_char_len(&self) -> usize {
        self.blocks.iter().map(|b| b.char_len()).sum()
    }

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

    pub fn slice(&self, start: DocPosition, end: DocPosition) -> Option<DocSlice> {
        if start >= end {
            return None;
        }

        let mut blocks = Vec::new();
        let start_block = self.block(start.block_index)?;
        let end_block = self.block(end.block_index)?;

        if start.block_index == end.block_index {
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

        let open_start = start.offset > 0;
        let open_end = end.offset < end_block.char_len();

        if let Some(runs) = start_block.runs() {
            let extracted = extract_runs(runs, start.offset, start_block.char_len());
            blocks.push(Block::Paragraph { runs: extracted });
        } else {
            blocks.push(start_block.clone());
        }

        for i in (start.block_index + 1)..end.block_index {
            if let Some(block) = self.block(i) {
                blocks.push(block.clone());
            }
        }

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

    pub fn is_valid_position(&self, pos: DocPosition) -> bool {
        if let Some(block) = self.block(pos.block_index) {
            pos.offset <= block.char_len()
        } else {
            false
        }
    }

    pub fn clamp_position(&self, pos: DocPosition) -> DocPosition {
        let block_index = pos.block_index.min(self.blocks.len().saturating_sub(1));
        let max_offset = self.block(block_index).map_or(0, Block::char_len);
        DocPosition::new(block_index, pos.offset.min(max_offset))
    }

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
        if run_end <= start_offset {
            pos = run_end;
            continue;
        }
        if run_start >= end_offset {
            break;
        }
        let overlap_start = start_offset.max(run_start) - run_start;
        let overlap_end = end_offset.min(run_end) - run_start;
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
pub fn isolate_runs(runs: &mut Vec<StyledRun>, start: usize, end: usize) -> Range<usize> {
    assert!(start <= end, "isolate_runs: start ({start}) > end ({end})");
    if runs.is_empty() || start == end {
        return 0..0;
    }
    let start_idx = split_runs_at(runs, start);
    let end_idx = split_runs_at(runs, end);
    start_idx..end_idx
}

fn split_runs_at(runs: &mut Vec<StyledRun>, offset: usize) -> usize {
    let mut pos = 0;
    for i in 0..runs.len() {
        let run_len = runs[i].char_len();
        if pos == offset {
            return i;
        }
        if pos + run_len > offset {
            let local_offset = offset - pos;
            let (left, right) = runs[i].split_at(local_offset);
            runs[i] = left;
            runs.insert(i + 1, right);
            return i + 1;
        }
        pos += run_len;
    }
    runs.len()
}

/// Public wrapper for [`split_runs_at`].
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
    }

    #[test]
    fn block_resolve_offset() {
        let block = Block::Paragraph {
            runs: vec![
                StyledRun::plain("hello"),
                StyledRun::styled(" world", InlineStyle::BOLD),
            ],
        };
        assert_eq!(block.resolve_offset(0), Some((0, 0)));
        assert_eq!(block.resolve_offset(5), Some((1, 0)));
        assert_eq!(block.resolve_offset(11), Some((1, 6)));
        assert_eq!(block.resolve_offset(12), None);
    }

    #[test]
    fn doc_position_ordering() {
        let a = DocPosition::new(0, 5);
        let b = DocPosition::new(1, 0);
        assert!(a < b);
    }

    #[test]
    fn isolate_runs_splits_correctly() {
        let mut runs = vec![
            StyledRun::plain("hello"),
            StyledRun::styled(" world", InlineStyle::BOLD),
        ];
        let range = isolate_runs(&mut runs, 3, 8);
        assert_eq!(runs.len(), 4);
        assert_eq!(range, 1..3);
    }

    #[test]
    fn list_item_is_inline_block() {
        let item = Block::list_item("hello", false);
        assert!(item.is_inline_block());
        assert!(!item.is_container());
        assert!(item.runs().is_some());
        assert_eq!(item.char_len(), 5);
        assert_eq!(item.kind(), BlockKind::ListItem { ordered: false });
    }

    #[test]
    fn image_block_char_len_is_zero() {
        let img = Block::Image {
            src: String::new(),
            alt: "x".into(),
            width: None,
            height: None,
        };
        assert_eq!(img.char_len(), 0);
    }

    // ── BlockAttrs tests ────────────────────────────────

    #[test]
    fn list_item_attrs_returns_indent() {
        let item = Block::list_item_with_indent("hello", false, 3);
        let attrs = item.attrs();
        assert_eq!(attrs.indent_level, 3);
        assert_eq!(attrs.alignment, TextAlignment::Left);
    }

    #[test]
    fn paragraph_attrs_returns_defaults() {
        let para = Block::paragraph("hello");
        let attrs = para.attrs();
        assert_eq!(attrs, BlockAttrs::default());
    }

    #[test]
    fn list_item_with_attrs_sets_indent() {
        let item = Block::list_item("hello", true);
        let new_item = item
            .with_attrs(BlockAttrs {
                indent_level: 2,
                alignment: TextAlignment::Left,
            })
            .expect("with_attrs should succeed for ListItem");
        assert_eq!(new_item.attrs().indent_level, 2);
        assert_eq!(new_item.flattened_text(), "hello");
        assert!(matches!(new_item, Block::ListItem { ordered: true, .. }));
    }

    #[test]
    fn with_attrs_on_hr_returns_none() {
        let hr = Block::HorizontalRule;
        assert!(hr.with_attrs(BlockAttrs::default()).is_none());
    }

    #[test]
    fn with_attrs_on_paragraph_returns_clone() {
        let para = Block::paragraph("hello");
        let result = para.with_attrs(BlockAttrs {
            indent_level: 5,
            ..Default::default()
        });
        assert!(result.is_some());
        // Paragraph ignores indent_level
        assert_eq!(
            result.as_ref().map(Block::flattened_text).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn text_alignment_default_is_left() {
        assert_eq!(TextAlignment::default(), TextAlignment::Left);
    }
}
