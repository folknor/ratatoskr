use serde::{Deserialize, Serialize};

/// How a send relates to an existing message.
///
/// Lives in `types` (a leaf, serde-only crate) so that both the
/// app/service IPC layer (`service-api`) and the provider/sync layer
/// (`common`) can name the same canonical type without dragging in
/// each other's dep graphs. Previously this lived in `common::types`
/// and `service_api::action` as separate enums with identical shape,
/// glued together by a hand-written conversion fn in `service::actions::send` -
/// a maintenance trap that silently misroutes if a variant is added
/// to one side and forgotten on the other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendIntent {
    #[default]
    New,
    Reply,
    Forward,
}
