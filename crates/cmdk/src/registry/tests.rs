#![allow(clippy::unwrap_used)]

use super::scoring::{category_relevance, recency_bonus};
use super::{CommandRegistry, UsageTracker};
use crate::context::{CommandContext, FocusedRegion, ViewType};
use crate::id::CommandId;
use crate::input::InputMode;

fn empty_context() -> CommandContext {
    CommandContext {
        selected_thread_ids: vec![],
        active_message_id: None,
        current_view: ViewType::Inbox,
        current_item_id: None,
        active_account_id: None,
        provider_kind: None,
        thread_is_read: None,
        thread_is_starred: None,
        thread_is_muted: None,
        thread_is_pinned: None,
        thread_is_draft: None,
        thread_in_trash: None,
        thread_in_spam: None,
        is_online: true,
        composer_is_open: false,
        focused_region: None,
        search_query: None,
        active_pinned_search: None,
        has_pinned_searches: false,
        may_remove_items: None,
        may_set_seen: None,
        may_set_keywords: None,
        may_submit: None,
    }
}

fn context_with_selection() -> CommandContext {
    let mut ctx = empty_context();
    ctx.selected_thread_ids = vec!["thread-1".to_string()];
    ctx.active_account_id = Some("acc-1".to_string());
    ctx
}

#[test]
fn registry_covers_all_command_ids() {
    let registry = CommandRegistry::new();
    for id in CommandId::all() {
        assert!(
            registry.get(*id).is_some(),
            "CommandId::{id:?} not registered"
        );
    }
}

#[test]
fn empty_query_returns_all_commands() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "");
    assert_eq!(results.len(), CommandId::all().len());
}

#[test]
fn empty_query_available_first_then_alphabetical() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "");

    let first_unavailable = results.iter().position(|r| !r.available);
    let last_available = results.iter().rposition(|r| r.available);
    if let (Some(first_un), Some(last_av)) = (first_unavailable, last_available) {
        assert!(
            last_av < first_un,
            "available commands should precede unavailable ones"
        );
    }

    let available: Vec<_> = results.iter().filter(|r| r.available).collect();
    for window in available.windows(2) {
        let a = window[0];
        let b = window[1];
        assert!(
            (a.category, a.label) <= (b.category, b.label)
                || category_relevance(a.category, &ctx) >= category_relevance(b.category, &ctx),
            "available group misordered: {} > {} before {} > {}",
            a.category,
            a.label,
            b.category,
            b.label
        );
    }
}

#[test]
fn no_selection_marks_email_unavailable() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(archive.is_some());
    assert!(!archive.is_none_or(|a| a.available));
}

#[test]
fn selection_marks_email_available() {
    let registry = CommandRegistry::new();
    let ctx = context_with_selection();
    let results = registry.query(&ctx, "");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(archive.is_some_and(|a| a.available));
}

#[test]
fn trash_vs_permanent_delete_exclusivity() {
    let registry = CommandRegistry::new();

    let ctx = context_with_selection();
    let results = registry.query(&ctx, "");
    let trash = results.iter().find(|r| r.id == CommandId::EmailTrash);
    let perm = results
        .iter()
        .find(|r| r.id == CommandId::EmailPermanentDelete);
    assert!(trash.is_some_and(|t| t.available));
    assert!(!perm.is_none_or(|p| p.available));

    let mut ctx2 = context_with_selection();
    ctx2.thread_in_trash = Some(true);
    let results2 = registry.query(&ctx2, "");
    let trash2 = results2.iter().find(|r| r.id == CommandId::EmailTrash);
    let perm2 = results2
        .iter()
        .find(|r| r.id == CommandId::EmailPermanentDelete);
    assert!(!trash2.is_none_or(|t| t.available));
    assert!(perm2.is_some_and(|p| p.available));
}

#[test]
fn fuzzy_search_finds_archive() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "arch");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(archive.is_some());
    assert!(archive.is_some_and(|a| a.score > 0));
}

#[test]
fn fuzzy_first_letter_matching() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "ea");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(
        archive.is_some(),
        "\"ea\" should match \"Email > Archive\" via first-letter-of-each-word"
    );
}

#[test]
fn fuzzy_results_sorted_by_score_desc() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "go");
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "results not sorted by score desc"
        );
    }
}

#[test]
fn toggle_label_resolves_based_on_state() {
    let registry = CommandRegistry::new();

    let ctx = context_with_selection();
    let results = registry.query(&ctx, "");
    let star = results.iter().find(|r| r.id == CommandId::EmailStar);
    assert_eq!(star.map(|s| s.label), Some("Star"));

    let mut ctx2 = context_with_selection();
    ctx2.thread_is_starred = Some(true);
    let results2 = registry.query(&ctx2, "");
    let star2 = results2.iter().find(|r| r.id == CommandId::EmailStar);
    assert_eq!(star2.map(|s| s.label), Some("Unstar"));
}

#[test]
fn parameterized_command_has_input_mode() {
    let registry = CommandRegistry::new();
    let ctx = context_with_selection();
    let results = registry.query(&ctx, "");
    let move_to = results
        .iter()
        .find(|r| r.id == CommandId::EmailMoveToFolder);
    assert!(move_to.is_some());
    assert!(
        matches!(
            move_to.map(|m| m.input_mode),
            Some(InputMode::Parameterized { .. })
        ),
        "EmailMoveToFolder should be Parameterized"
    );
}

#[test]
fn direct_command_has_direct_mode() {
    let registry = CommandRegistry::new();
    let ctx = context_with_selection();
    let results = registry.query(&ctx, "");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(archive.is_some());
    assert!(
        matches!(archive.map(|m| m.input_mode), Some(InputMode::Direct)),
        "EmailArchive should be Direct"
    );
}

#[test]
fn validate_param_request_accepts_valid_list_picker() {
    let registry = CommandRegistry::new();
    let result = registry.validate_param_request(CommandId::EmailMoveToFolder, 0, &[]);
    assert!(result.is_ok());
    assert!(result.is_ok_and(|p| p.is_list_picker()));
}

#[test]
fn validate_param_request_rejects_non_parameterized() {
    let registry = CommandRegistry::new();
    let result = registry.validate_param_request(CommandId::EmailArchive, 0, &[]);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("not a parameterized command"),
        "should reject non-parameterized command"
    );
}

#[test]
fn validate_param_request_rejects_out_of_bounds() {
    let registry = CommandRegistry::new();
    let result =
        registry.validate_param_request(CommandId::EmailMoveToFolder, 1, &["x".into()]);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("out of bounds"),
        "should reject out-of-bounds param_index"
    );
}

#[test]
fn validate_param_request_rejects_wrong_prior_selections_count() {
    let registry = CommandRegistry::new();
    let result = registry.validate_param_request(
        CommandId::EmailMoveToFolder,
        0,
        &["unexpected".into()],
    );
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("prior selections"),
        "should reject mismatched prior_selections count"
    );
}

#[test]
fn sync_folder_needs_account() {
    let registry = CommandRegistry::new();

    let ctx = empty_context();
    let results = registry.query(&ctx, "");
    let sync = results.iter().find(|r| r.id == CommandId::AppSyncFolder);
    assert!(!sync.is_none_or(|s| s.available));

    let mut ctx2 = empty_context();
    ctx2.active_account_id = Some("acc-1".to_string());
    let results2 = registry.query(&ctx2, "");
    let sync2 = results2.iter().find(|r| r.id == CommandId::AppSyncFolder);
    assert!(sync2.is_some_and(|s| s.available));
}

#[test]
fn keyword_search_finds_trash_via_delete() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "delete");
    let trash = results.iter().find(|r| r.id == CommandId::EmailTrash);
    assert!(
        trash.is_some(),
        "\"delete\" should match Move to Trash via keyword"
    );
}

#[test]
fn keyword_search_finds_archive_via_done() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "done");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(
        archive.is_some(),
        "\"done\" should match Archive via keyword"
    );
}

#[test]
fn keyword_search_finds_compose_via_write() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "write");
    let compose = results.iter().find(|r| r.id == CommandId::ComposeNew);
    assert!(
        compose.is_some(),
        "\"write\" should match Compose New Email via keyword"
    );
}

#[test]
fn keyword_search_finds_mark_read_via_seen() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "seen");
    let mark_read = results.iter().find(|r| r.id == CommandId::EmailMarkRead);
    assert!(
        mark_read.is_some(),
        "\"seen\" should match Mark as Read via keyword"
    );
}

#[test]
fn keyword_search_finds_move_to_folder_via_organize() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "organize");
    let m = results
        .iter()
        .find(|r| r.id == CommandId::EmailMoveToFolder);
    assert!(
        m.is_some(),
        "\"organize\" should match Move to Folder via keyword"
    );
}

#[test]
fn keyword_search_finds_reply_via_respond() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "respond");
    let reply = results.iter().find(|r| r.id == CommandId::ComposeReply);
    assert!(
        reply.is_some(),
        "\"respond\" should match Reply via keyword"
    );
}

#[test]
fn keyword_search_finds_forward_via_share() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "share");
    let fwd = results.iter().find(|r| r.id == CommandId::ComposeForward);
    assert!(fwd.is_some(), "\"share\" should match Forward via keyword");
}

#[test]
fn available_commands_rank_above_unavailable_in_fuzzy() {
    let registry = CommandRegistry::new();
    let ctx = context_with_selection();
    let results = registry.query(&ctx, "arch");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(archive.is_some_and(|a| a.available));
    assert!(archive.is_some_and(|a| a.score >= 1000));

    let ctx2 = empty_context();
    let results2 = registry.query(&ctx2, "arch");
    let archive2 = results2.iter().find(|r| r.id == CommandId::EmailArchive);
    assert!(!archive2.is_none_or(|a| a.available));
    assert!(archive2.is_none_or(|a| a.score < 1000));
}

#[test]
fn context_boost_favors_email_in_inbox() {
    let registry = CommandRegistry::new();
    let ctx = context_with_selection();
    let results = registry.query(&ctx, "arch");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    let score = archive.map_or(0, |a| a.score);
    assert!(
        score > 1000,
        "archive score {score} should exceed 1000 (availability bonus + context boost)"
    );
}

#[test]
fn composer_focus_boosts_compose_commands() {
    let registry = CommandRegistry::new();
    let mut ctx = context_with_selection();
    ctx.focused_region = Some(FocusedRegion::Composer);
    ctx.composer_is_open = true;
    let results = registry.query(&ctx, "reply");
    let reply = results.iter().find(|r| r.id == CommandId::ComposeReply);
    let reply_score = reply.map_or(0, |r| r.score);

    let ctx2 = context_with_selection();
    let results2 = registry.query(&ctx2, "reply");
    let reply2 = results2.iter().find(|r| r.id == CommandId::ComposeReply);
    let reply_score2 = reply2.map_or(0, |r| r.score);

    assert!(
        reply_score > reply_score2,
        "compose commands should score higher with composer focused: {reply_score} vs {reply_score2}"
    );
}

#[test]
fn empty_query_groups_available_before_unavailable() {
    let registry = CommandRegistry::new();
    let ctx = empty_context();
    let results = registry.query(&ctx, "");

    let mut seen_unavailable = false;
    for r in &results {
        if !r.available {
            seen_unavailable = true;
        } else if seen_unavailable {
            panic!(
                "available command {} > {} after unavailable ones",
                r.category, r.label
            );
        }
    }
}

#[test]
fn tasks_view_boosts_tasks_category() {
    let mut ctx = empty_context();
    ctx.current_view = ViewType::Tasks;
    let relevance = category_relevance("Tasks", &ctx);
    assert_eq!(
        relevance, 4,
        "Tasks category should have max relevance on Tasks view"
    );

    let ctx2 = empty_context();
    let relevance2 = category_relevance("Tasks", &ctx2);
    assert!(
        relevance > relevance2,
        "Tasks relevance should be higher on Tasks view ({relevance}) than Inbox ({relevance2})"
    );
}

#[test]
fn recency_score_populated_from_usage_tracker() {
    let mut registry = CommandRegistry::new();
    registry.usage.record_usage(CommandId::EmailArchive);
    registry.usage.record_usage(CommandId::EmailArchive);
    registry.usage.record_usage(CommandId::ComposeNew);

    let ctx = empty_context();
    let results = registry.query(&ctx, "");

    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert_eq!(archive.map_or(0, |a| a.recency_score), 2);

    let compose = results.iter().find(|r| r.id == CommandId::ComposeNew);
    assert_eq!(compose.map_or(0, |c| c.recency_score), 1);

    let star = results.iter().find(|r| r.id == CommandId::EmailStar);
    assert_eq!(star.map_or(99, |s| s.recency_score), 0);
}

#[test]
fn usage_tracker_basics() {
    let mut tracker = UsageTracker::new();
    assert_eq!(tracker.usage_count(CommandId::NavNext), 0);

    tracker.record_usage(CommandId::NavNext);
    assert_eq!(tracker.usage_count(CommandId::NavNext), 1);

    tracker.record_usage(CommandId::NavNext);
    tracker.record_usage(CommandId::NavNext);
    assert_eq!(tracker.usage_count(CommandId::NavNext), 3);

    assert_eq!(tracker.usage_count(CommandId::NavPrev), 0);
}

#[test]
fn recency_score_in_fuzzy_results() {
    let mut registry = CommandRegistry::new();
    registry.usage.record_usage(CommandId::EmailArchive);
    registry.usage.record_usage(CommandId::EmailArchive);
    registry.usage.record_usage(CommandId::EmailArchive);

    let ctx = empty_context();
    let results = registry.query(&ctx, "arch");
    let archive = results.iter().find(|r| r.id == CommandId::EmailArchive);
    assert_eq!(
        archive.map_or(0, |a| a.recency_score),
        3,
        "fuzzy results should carry recency_score"
    );
}

#[test]
fn descriptor_keywords_field() {
    let registry = CommandRegistry::new();
    let archive = registry.get(CommandId::EmailArchive);
    assert!(
        archive.is_some_and(|d| d.keywords.contains(&"done")),
        "Archive should have 'done' keyword"
    );
    let trash = registry.get(CommandId::EmailTrash);
    assert!(
        trash.is_some_and(|d| d.keywords.contains(&"delete")),
        "Trash should have 'delete' keyword"
    );
    let nav_next = registry.get(CommandId::NavNext);
    assert!(
        nav_next.is_some_and(|d| d.keywords.is_empty()),
        "NavNext should have no keywords"
    );
}

#[test]
fn settings_view_reduces_email_relevance() {
    let mut ctx = empty_context();
    ctx.current_view = ViewType::Settings;
    let email_rel = category_relevance("Email", &ctx);
    assert!(
        email_rel < 4,
        "Email relevance should be low on Settings view"
    );

    ctx.current_view = ViewType::Inbox;
    let email_rel_inbox = category_relevance("Email", &ctx);
    assert_eq!(email_rel_inbox, 4);
}

#[test]
fn recency_bonus_is_log_scaled() {
    assert_eq!(recency_bonus(0), 0);
    assert_eq!(recency_bonus(1), 16);
    assert_eq!(recency_bonus(7), 32);
    assert_eq!(recency_bonus(31), 48);
    assert_eq!(recency_bonus(127), 64);
    // Doubles in count add a constant 8 - log shape, no runaway.
    assert_eq!(recency_bonus(255), 72);
    // Stays well below the 1000 availability bonus even at saturation.
    assert!(recency_bonus(u32::MAX) < 300);
}

#[test]
fn fuzzy_recency_breaks_ties() {
    // "Set Theme: Light" / "Set Theme: Dark" / "Set Theme: System" all
    // share the matched prefix for "theme", so without recency the
    // ranking is determined by alphabetical/registration order. Pumping
    // usage on Dark should move it ahead of Light and System.
    let mut registry = CommandRegistry::new();
    let ctx = empty_context();

    let baseline = registry.query(&ctx, "theme");
    let baseline_dark_pos = baseline
        .iter()
        .position(|m| m.id == CommandId::ViewSetThemeDark)
        .expect("Dark in baseline");

    for _ in 0..20 {
        registry.usage.record_usage(CommandId::ViewSetThemeDark);
    }

    let after = registry.query(&ctx, "theme");
    let after_dark_pos = after
        .iter()
        .position(|m| m.id == CommandId::ViewSetThemeDark)
        .expect("Dark in after");

    assert!(
        after_dark_pos < baseline_dark_pos,
        "Dark should rise after heavy usage; baseline pos {baseline_dark_pos} -> after pos {after_dark_pos}"
    );
}

#[test]
fn fuzzy_recency_does_not_override_availability() {
    // EmailArchive requires a selected thread. With no selection it's
    // unavailable. Even with massive usage, an unavailable command must
    // not outrank an available command matching the same query.
    let mut registry = CommandRegistry::new();
    let ctx = empty_context(); // no selection -> EmailArchive unavailable

    for _ in 0..1000 {
        registry.usage.record_usage(CommandId::EmailArchive);
    }

    let results = registry.query(&ctx, "arch");
    let archive_pos = results
        .iter()
        .position(|m| m.id == CommandId::EmailArchive)
        .expect("Archive present even when unavailable");

    // Anything above Archive in the list must be available.
    for m in &results[..archive_pos] {
        assert!(
            m.available,
            "{:?} ranked above unavailable Archive but is itself unavailable",
            m.id
        );
    }
}
