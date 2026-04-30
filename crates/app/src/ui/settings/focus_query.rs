use iced::Rectangle;
use iced::advanced::widget::{Id, Operation, operation};

use super::types::FilterId;

/// Map a widget ID we set on a filter input back to its `FilterId`.
fn id_to_filter(id: &Id) -> Option<FilterId> {
    // Compare against the specific IDs we hand the filter inputs in tabs.rs.
    if *id == Id::from("contact-filter") {
        Some(FilterId::Contacts)
    } else if *id == Id::from("group-filter") {
        Some(FilterId::Groups)
    } else if *id == Id::from("group-add-filter") {
        Some(FilterId::GroupAddMembers)
    } else if *id == Id::from("group-members-filter") {
        Some(FilterId::GroupMembers)
    } else {
        None
    }
}

/// Walks the widget tree and returns `Some(FilterId)` if a known filter
/// input is currently focused, `None` otherwise. Always finishes with an
/// `Outcome::Some(...)` so the caller's `Task::widget(...)` always emits a
/// message — that lets us clear `focused_filter` when focus moves to a
/// non-filter widget.
pub fn find_focused_filter() -> impl Operation<Option<FilterId>> {
    struct FindFocusedFilter {
        found: Option<FilterId>,
    }

    impl Operation<Option<FilterId>> for FindFocusedFilter {
        fn focusable(
            &mut self,
            id: Option<&Id>,
            _bounds: Rectangle,
            state: &mut dyn operation::Focusable,
        ) {
            if state.is_focused()
                && let Some(id) = id
                && let Some(filter) = id_to_filter(id)
            {
                self.found = Some(filter);
            }
        }

        fn traverse(
            &mut self,
            operate: &mut dyn FnMut(&mut dyn Operation<Option<FilterId>>),
        ) {
            operate(self);
        }

        fn finish(&self) -> operation::Outcome<Option<FilterId>> {
            operation::Outcome::Some(self.found)
        }
    }

    FindFocusedFilter { found: None }
}
