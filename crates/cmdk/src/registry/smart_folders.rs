use crate::descriptor::CommandDescriptor;
use crate::id::CommandId;
use crate::input::{InputSchema, ParamDef};

use super::builders::{parameterized, with_docs, with_keywords};

pub(super) fn register_smart_folders(out: &mut Vec<CommandDescriptor>) {
    out.push(with_docs(
        with_keywords(
            parameterized(
                CommandId::SmartFolderSave,
                "Save Search",
                "Search",
                None,
                |ctx| ctx.search_query.as_ref().is_some_and(|q| !q.is_empty()),
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
}
