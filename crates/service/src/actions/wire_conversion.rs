//! Wire <-> domain conversion for action plans (Phase 2 task 9).
//!
//! `WireMailOperation` (in `service-api`) is a serializable 1:1 mirror
//! of `MailOperation` (here, in `service::actions`). The conversion
//! lives in `service` rather than `service-api` because `service-api`
//! is intentionally lightweight (no provider/search/store deps), and
//! mapping `WireFolderId(String) -> FolderId` requires `common`'s
//! typed-ID constructors.
//!
//! The two `match` arms below are exhaustive without `_` wildcards on
//! purpose: adding a new variant to `MailOperation` (or
//! `WireMailOperation`) without a matching arm here is a compile
//! error, which is the regression guard the static-mirror contract
//! requires.

use common::typed_ids::{FolderId, LabelGroupId, LabelId};
use service_api::{WireFolderId, WireLabelGroupId, WireMailOperation, WireLabelId};

use super::operation::MailOperation;

/// Convert a UI-side `WireMailOperation` (e.g. fresh off the wire as
/// part of an `ActionWirePlan`) into the canonical `MailOperation` the
/// action service runs.
pub(crate) fn wire_to_mail(op: WireMailOperation) -> MailOperation {
    match op {
        WireMailOperation::Archive => MailOperation::Archive,
        WireMailOperation::Trash => MailOperation::Trash,
        WireMailOperation::PermanentDelete => MailOperation::PermanentDelete,
        WireMailOperation::SetSpam { to } => MailOperation::SetSpam { to },
        WireMailOperation::SetStarred { to } => MailOperation::SetStarred { to },
        WireMailOperation::SetRead { to } => MailOperation::SetRead { to },
        WireMailOperation::SetPinned { to } => MailOperation::SetPinned { to },
        WireMailOperation::SetMuted { to } => MailOperation::SetMuted { to },
        WireMailOperation::MoveToFolder { dest, source } => MailOperation::MoveToFolder {
            dest: wire_folder_to_folder(dest),
            source: source.map(wire_folder_to_folder),
        },
        WireMailOperation::AddLabel { label_id } => MailOperation::AddLabel {
            label_id: wire_label_to_label(label_id),
        },
        WireMailOperation::RemoveLabel { label_id } => MailOperation::RemoveLabel {
            label_id: wire_label_to_label(label_id),
        },
        WireMailOperation::ApplyLabelGroup { group_id } => MailOperation::ApplyLabelGroup {
            group_id: wire_label_group_to_label_group(group_id),
        },
        WireMailOperation::RemoveLabelGroup { group_id } => MailOperation::RemoveLabelGroup {
            group_id: wire_label_group_to_label_group(group_id),
        },
        WireMailOperation::Snooze { until } => MailOperation::Snooze { until },
        WireMailOperation::Unsnooze => MailOperation::Unsnooze,
    }
}

/// Reverse direction: `MailOperation` -> `WireMailOperation`. Used by
/// the worker when it journals a plan locally for replay (the journal
/// stores the wire form; if a path ever needs to insert a journal row
/// from a `MailOperation` rather than from an `ActionWirePlan`, it
/// goes through here).
///
/// **Do NOT delete even if it appears unused.** This function is
/// the bidirectional regression guard for the `MailOperation` <->
/// `WireMailOperation` mirror: its exhaustive match (no `_`
/// wildcard) means that adding a `MailOperation` variant without
/// adding the corresponding `WireMailOperation` variant fails to
/// compile here. The companion guard for the wire->domain
/// direction is `wire_to_mail`. The `mail_side_mirror_is_exhaustive`
/// test below pins the bidirectional invariant by force-calling
/// this function on every `MailOperation` variant.
#[allow(dead_code)]
pub(crate) fn mail_to_wire(op: MailOperation) -> WireMailOperation {
    match op {
        MailOperation::Archive => WireMailOperation::Archive,
        MailOperation::Trash => WireMailOperation::Trash,
        MailOperation::PermanentDelete => WireMailOperation::PermanentDelete,
        MailOperation::SetSpam { to } => WireMailOperation::SetSpam { to },
        MailOperation::SetStarred { to } => WireMailOperation::SetStarred { to },
        MailOperation::SetRead { to } => WireMailOperation::SetRead { to },
        MailOperation::SetPinned { to } => WireMailOperation::SetPinned { to },
        MailOperation::SetMuted { to } => WireMailOperation::SetMuted { to },
        MailOperation::MoveToFolder { dest, source } => WireMailOperation::MoveToFolder {
            dest: folder_to_wire(dest),
            source: source.map(folder_to_wire),
        },
        MailOperation::AddLabel { label_id } => WireMailOperation::AddLabel {
            label_id: label_to_wire(label_id),
        },
        MailOperation::RemoveLabel { label_id } => WireMailOperation::RemoveLabel {
            label_id: label_to_wire(label_id),
        },
        MailOperation::ApplyLabelGroup { group_id } => WireMailOperation::ApplyLabelGroup {
            group_id: label_group_to_wire(group_id),
        },
        MailOperation::RemoveLabelGroup { group_id } => WireMailOperation::RemoveLabelGroup {
            group_id: label_group_to_wire(group_id),
        },
        MailOperation::Snooze { until } => WireMailOperation::Snooze { until },
        MailOperation::Unsnooze => WireMailOperation::Unsnooze,
    }
}

fn wire_folder_to_folder(w: WireFolderId) -> FolderId {
    FolderId(w.0)
}

fn folder_to_wire(f: FolderId) -> WireFolderId {
    WireFolderId(f.0)
}

fn wire_label_to_label(w: WireLabelId) -> LabelId {
    LabelId(w.0)
}

fn label_to_wire(t: LabelId) -> WireLabelId {
    WireLabelId(t.0)
}

fn wire_label_group_to_label_group(w: WireLabelGroupId) -> LabelGroupId {
    LabelGroupId(w.0)
}

fn label_group_to_wire(t: LabelGroupId) -> WireLabelGroupId {
    WireLabelGroupId(t.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_to_mail_round_trips_archive() {
        let m = wire_to_mail(WireMailOperation::Archive);
        assert_eq!(m, MailOperation::Archive);
    }

    #[test]
    fn round_trip_preserves_every_variant() {
        let cases = [
            WireMailOperation::Archive,
            WireMailOperation::Trash,
            WireMailOperation::PermanentDelete,
            WireMailOperation::SetSpam { to: true },
            WireMailOperation::SetStarred { to: false },
            WireMailOperation::SetRead { to: true },
            WireMailOperation::SetPinned { to: false },
            WireMailOperation::SetMuted { to: true },
            WireMailOperation::MoveToFolder {
                dest: WireFolderId("inbox".into()),
                source: Some(WireFolderId("archive".into())),
            },
            WireMailOperation::MoveToFolder {
                dest: WireFolderId("inbox".into()),
                source: None,
            },
            WireMailOperation::AddLabel {
                label_id: WireLabelId("work".into()),
            },
            WireMailOperation::RemoveLabel {
                label_id: WireLabelId("work".into()),
            },
            WireMailOperation::ApplyLabelGroup {
                group_id: WireLabelGroupId(7),
            },
            WireMailOperation::RemoveLabelGroup {
                group_id: WireLabelGroupId(7),
            },
            WireMailOperation::Snooze { until: 1_700_000_000 },
            WireMailOperation::Unsnooze,
        ];
        for w in cases {
            let m = wire_to_mail(w.clone());
            let back = mail_to_wire(m);
            assert_eq!(w, back, "wire -> mail -> wire must round-trip");
        }
    }

    /// Mail-side mirror exhaustiveness pin. Force-constructs every
    /// `MailOperation` variant and converts via `mail_to_wire`, then
    /// round-trips back. The match below has no `_` wildcard, so a
    /// new `MailOperation` variant added in core fails to compile
    /// here - even if `mail_to_wire` itself stops being used in
    /// production code. This is the explicit bidirectional regression
    /// guard the Phase 2 plan called for; it complements the
    /// wire-side enumeration in `round_trip_preserves_every_variant`.
    #[test]
    fn mail_side_mirror_is_exhaustive() {
        let cases: Vec<MailOperation> = vec![
            MailOperation::Archive,
            MailOperation::Trash,
            MailOperation::PermanentDelete,
            MailOperation::SetSpam { to: true },
            MailOperation::SetStarred { to: false },
            MailOperation::SetRead { to: true },
            MailOperation::SetPinned { to: false },
            MailOperation::SetMuted { to: true },
            MailOperation::MoveToFolder {
                dest: FolderId("inbox".into()),
                source: Some(FolderId("archive".into())),
            },
            MailOperation::AddLabel {
                label_id: LabelId("work".into()),
            },
            MailOperation::RemoveLabel {
                label_id: LabelId("work".into()),
            },
            MailOperation::ApplyLabelGroup {
                group_id: LabelGroupId(7),
            },
            MailOperation::RemoveLabelGroup {
                group_id: LabelGroupId(7),
            },
            MailOperation::Snooze { until: 1_700_000_000 },
            MailOperation::Unsnooze,
        ];
        for m in cases {
            // Exhaustive match on MailOperation, no wildcard. A new
            // variant added in core without a wire mirror fails
            // compilation here, before any assertion runs.
            let tag: &str = match &m {
                MailOperation::Archive => "Archive",
                MailOperation::Trash => "Trash",
                MailOperation::PermanentDelete => "PermanentDelete",
                MailOperation::SetSpam { .. } => "SetSpam",
                MailOperation::SetStarred { .. } => "SetStarred",
                MailOperation::SetRead { .. } => "SetRead",
                MailOperation::SetPinned { .. } => "SetPinned",
                MailOperation::SetMuted { .. } => "SetMuted",
                MailOperation::MoveToFolder { .. } => "MoveToFolder",
                MailOperation::AddLabel { .. } => "AddLabel",
                MailOperation::RemoveLabel { .. } => "RemoveLabel",
                MailOperation::ApplyLabelGroup { .. } => "ApplyLabelGroup",
                MailOperation::RemoveLabelGroup { .. } => "RemoveLabelGroup",
                MailOperation::Snooze { .. } => "Snooze",
                MailOperation::Unsnooze => "Unsnooze",
            };
            let w = mail_to_wire(m.clone());
            let back = wire_to_mail(w);
            assert_eq!(m, back, "mail -> wire -> mail must round-trip ({tag})");
        }
    }
}
