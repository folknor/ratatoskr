//! Drag-and-drop widget for iced.
//!
//! Vendored from <https://github.com/jhannyj/iced_drop> (v0.2.2) and adapted
//! for the Halloy/folknor iced fork used by Ratatoskr.
//!
//! # Public API
//!
//! - [`droppable()`] — wraps any element to make it draggable
//! - [`zones_on_point()`] — produces a [`Task`] that finds drop zones containing a point
//! - [`find_zones()`] — produces a [`Task`] that finds drop zones matching a custom filter
//! - [`Droppable`] — the drag-and-drop widget wrapper
//! - [`DroppableState`] — internal widget state (useful for custom [`Operation`]s)

pub mod widget;

use iced_core::widget::Id;
use iced_core::{renderer, Element, Point, Rectangle};
use iced_runtime::futures::MaybeSend;
use iced_runtime::task::widget as operate;
use iced_runtime::Task;
use widget::droppable::*;
use widget::operation::drop;

/// Creates a new [`Droppable`] widget wrapping the given content.
pub fn droppable<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Droppable<'a, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    Droppable::new(content)
}

/// Produces a [`Task`] that finds all drop zones whose bounds contain the given `point`.
///
/// - `msg`: closure that receives the matching zones and returns a message.
/// - `options`: if `Some`, only zones with these [`Id`]s are considered.
/// - `depth`: maximum nesting depth to search (`None` = unlimited).
pub fn zones_on_point<T, MF>(
    msg: MF,
    point: Point,
    options: Option<Vec<Id>>,
    depth: Option<usize>,
) -> Task<T>
where
    T: Send + 'static,
    MF: Fn(Vec<(Id, Rectangle)>) -> T + MaybeSend + Sync + Clone + 'static,
{
    operate(drop::find_zones(
        move |bounds| bounds.contains(point),
        options,
        depth,
    ))
    .map(msg)
}

/// Produces a [`Task`] that finds all drop zones whose bounds pass a custom filter.
///
/// - `msg`: closure that receives the matching zones and returns a message.
/// - `filter`: predicate applied to each zone's bounds.
/// - `options`: if `Some`, only zones with these [`Id`]s are considered.
/// - `depth`: maximum nesting depth to search (`None` = unlimited).
pub fn find_zones_task<Message, MF, F>(
    msg: MF,
    filter: F,
    options: Option<Vec<Id>>,
    depth: Option<usize>,
) -> Task<Message>
where
    Message: Send + 'static,
    MF: Fn(Vec<(Id, Rectangle)>) -> Message + MaybeSend + Sync + Clone + 'static,
    F: Fn(&Rectangle) -> bool + Send + 'static,
{
    operate(drop::find_zones(filter, options, depth)).map(msg)
}
