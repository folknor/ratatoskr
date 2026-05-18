use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::input::{InputSchema, ParamDef};

use super::builders::{desc_kw, parameterized, with_docs, with_keywords};

pub(super) fn register_smart_folders(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        with_keywords(
            parameterized(
                CommandId::SmartFolderSave,
                "Save Search",
                "Search",
                None,
                |ctx| ctx.active_pinned_search.is_some(),
                InputSchema::Single {
                    param: ParamDef::Text {
                        label: "Name",
                        placeholder: "Smart folder name...",
                    },
                },
            ),
            &["smart folder", "save search", "pin"],
        ),
        "Save as Smart Folder",
        "Save the current search query as a smart folder in the sidebar for quick access.",
    ));

    out.push(with_docs(
        desc_kw(
            CommandId::PinnedSearchesClearAll,
            "Clear All Pinned Searches",
            "Search",
            None,
            |ctx| ctx.has_pinned_searches,
            &["pinned", "clear", "remove", "all"],
        ),
        "Clear All Pinned Searches",
        "Remove every pinned search from the sidebar. Smart folders are unaffected.",
    ));
}
