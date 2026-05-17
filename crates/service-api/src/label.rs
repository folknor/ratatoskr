//! Wire types for label_group IPC.

use serde::{Deserialize, Serialize};

/// `label_group.reorder` params. Each pair is `(label_groups.id, sort_order)`.
/// The Service writes all rows in one transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupReorderParams {
    pub orders: Vec<(i64, i64)>,
}

/// `label_group.reorder` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelGroupReorderAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_group_reorder_params_round_trip_through_serde() {
        let original = LabelGroupReorderParams {
            orders: vec![(1, 0), (2, 1), (3, 2)],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: LabelGroupReorderParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
