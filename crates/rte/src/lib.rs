//! Rich text editor for email composition in iced.
//!
//! Built from scratch — no existing rich text editor exists for iced. Design
//! informed by deep study of ProseMirror, Slate.js, Quill, and fleather.
//!
//! # Crate structure
//!
//! **Pure Rust** (no iced dependency): `document`, `operations`, `normalize`,
//! `rules`, `undo`, `html_serialize`, `html_parse`. Unit-testable without a GUI.
//!
//! **Feature-gated** (`widget` feature, default on): `widget` module depends
//! on iced for rendering, input handling, and cursor management.

pub mod compose;
pub mod document;
pub mod html_parse;
pub mod html_serialize;
pub mod normalize;
pub mod operations;
pub mod rules;
pub mod undo;

#[cfg(feature = "widget")]
pub mod widget;

pub use document::{
    Block, BlockAttrs, BlockKind, DocPosition, DocSelection, DocSlice, Document, HeadingLevel,
    InlineStyle, StyledRun, TextAlignment,
};
pub use normalize::normalize;
pub use operations::{EditOp, PosMap};
pub use rules::EditAction;
pub use undo::UndoStack;

#[cfg(feature = "widget")]
pub use widget::{Action, EditorState, RichTextEditor, rich_text_editor};
