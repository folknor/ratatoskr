//! `FromRow` implementations for the main database types.
//!
//! Generated via `impl_from_row!` to match the existing hand-written
//! `row_to_*` functions. Types with non-trivial row mapping (e.g.
//! `DbMessage` which hardcodes `body_html`/`body_text` to `None`)
//! are intentionally excluded.

use super::types::*;
use crate::impl_from_row;

// ── Account ─────────────────────────────────────────────────

impl_from_row!(DbAccount {
    id,
    email,
    display_name,
    avatar_url,
    access_token,
    refresh_token,
    token_expires_at,
    history_id,
    initial_sync_completed,
    last_sync_at,
    is_active,
    created_at,
    updated_at,
    provider,
    imap_host,
    imap_port,
    imap_security,
    smtp_host,
    smtp_port,
    smtp_security,
    auth_method,
    imap_password,
    oauth_provider,
    oauth_client_id,
    oauth_client_secret,
    imap_username,
    smtp_username,
    smtp_password,
    caldav_url,
    caldav_username,
    caldav_password,
    caldav_principal_url,
    caldav_home_url,
    calendar_provider,
    accept_invalid_certs,
    jmap_url,
    account_color,
    account_name,
    sort_order,
});

// ── Thread ──────────────────────────────────────────────────

impl_from_row!(DbThread {
    id,
    account_id,
    subject,
    snippet,
    last_message_at,
    message_count,
    bool is_read,
    bool is_starred,
    bool is_important,
    bool has_attachments,
    bool is_snoozed,
    snooze_until,
    bool is_pinned,
    bool is_muted,
    from_name,
    from_address,
});

// ── Label ───────────────────────────────────────────────────

impl_from_row!(DbLabel {
    id,
    account_id,
    name,
    label_type as "type",
    label_kind,
    color_bg,
    color_fg,
    bool visible,
    sort_order,
    imap_folder_path,
    imap_special_use,
    parent_label_id,
    optbool right_read,
    optbool right_add,
    optbool right_remove,
    optbool right_set_seen,
    optbool right_set_keywords,
    optbool right_create_child,
    optbool right_rename,
    optbool right_delete,
    optbool right_submit,
});

// ── Category ────────────────────────────────────────────────

impl_from_row!(DbCategory {
    id,
    account_id,
    display_name,
    color_preset,
    color_bg,
    color_fg,
    provider_id,
    sync_state,
    sort_order,
});

// ── Contact ─────────────────────────────────────────────────

impl_from_row!(DbContact {
    id,
    email,
    display_name,
    avatar_url,
    frequency,
    last_contacted_at,
    notes,
    email2,
    phone,
    company,
    account_id,
    server_id,
    source,
});

// ── Attachment ──────────────────────────────────────────────

impl_from_row!(DbAttachment {
    id,
    message_id,
    account_id,
    filename,
    mime_type,
    size,
    gmail_attachment_id,
    content_id,
    bool is_inline,
    local_path,
    content_hash,
});

// ── Filter Rule ─────────────────────────────────────────────

impl_from_row!(DbFilterRule {
    id,
    account_id,
    name,
    bool is_enabled,
    criteria_json,
    actions_json,
    sort_order,
    created_at,
});

// ── Smart Folder ────────────────────────────────────────────

impl_from_row!(DbSmartFolder {
    id,
    account_id,
    name,
    query,
    icon,
    color,
    sort_order,
    bool is_default,
    created_at,
});

// ── Smart Label Rule ────────────────────────────────────────

impl_from_row!(DbSmartLabelRule {
    id,
    account_id,
    label_id,
    ai_description,
    criteria_json,
    bool is_enabled,
    sort_order,
    created_at,
});

// ── Follow-Up Reminder ──────────────────────────────────────

impl_from_row!(DbFollowUpReminder {
    id,
    account_id,
    thread_id,
    message_id,
    remind_at,
    status,
    created_at,
});

// ── Quick Step ──────────────────────────────────────────────

impl_from_row!(DbQuickStep {
    id,
    account_id,
    name,
    description,
    shortcut,
    actions_json,
    icon,
    bool is_enabled,
    bool continue_on_error,
    sort_order,
    created_at,
});

// ── Task ────────────────────────────────────────────────────

impl_from_row!(DbTask {
    id,
    account_id,
    title,
    description,
    priority,
    is_completed,
    completed_at,
    due_date,
    parent_id,
    thread_id,
    thread_account_id,
    sort_order,
    recurrence_rule,
    next_recurrence_at,
    tags_json,
    created_at,
    updated_at,
});

// ── Task Tag ────────────────────────────────────────────────

impl_from_row!(DbTaskTag {
    tag,
    account_id,
    color,
    sort_order,
    created_at,
});

// ── Bundle Rule ─────────────────────────────────────────────

impl_from_row!(DbBundleRule {
    id,
    account_id,
    category,
    is_bundled,
    delivery_enabled,
    delivery_schedule,
    last_delivered_at,
    created_at,
});

// ── Calendar ────────────────────────────────────────────────

impl_from_row!(DbCalendar {
    id,
    account_id,
    provider,
    remote_id,
    display_name,
    color,
    is_primary,
    is_visible,
    sync_token,
    ctag,
    created_at,
    updated_at,
    sort_order,
    is_default,
    provider_id,
});

// ── Calendar Event ──────────────────────────────────────────

impl_from_row!(DbCalendarEvent {
    id,
    account_id,
    google_event_id,
    summary,
    description,
    location,
    start_time,
    end_time,
    is_all_day,
    status,
    organizer_email,
    attendees_json,
    html_link,
    updated_at,
    calendar_id,
    remote_event_id,
    etag,
    ical_data,
    uid,
    title,
    timezone,
    recurrence_rule,
    organizer_name,
    rsvp_status,
    created_at,
    availability,
    visibility,
});

// ── Calendar Attendee ──────────────────────────────────────

impl_from_row!(DbCalendarAttendee {
    event_id,
    account_id,
    email,
    name,
    rsvp_status,
    is_organizer,
});

// ── Calendar Reminder ─────────────────────────────────────

impl_from_row!(DbCalendarReminder {
    id,
    event_id,
    account_id,
    minutes_before,
    method,
});

// ── Writing Style Profile ──────────────────────────────────

impl_from_row!(DbWritingStyleProfile {
    id,
    account_id,
    profile_text,
    sample_count,
    created_at,
    updated_at,
});

// ── Folder Sync State ──────────────────────────────────────

impl_from_row!(DbFolderSyncState {
    account_id,
    folder_path,
    uidvalidity,
    last_uid,
    modseq,
    last_sync_at,
});

// ── Notification VIP ────────────────────────────────────────

impl_from_row!(DbNotificationVip {
    id,
    account_id,
    email_address,
    display_name,
    created_at,
});

// ── Image Allowlist ─────────────────────────────────────────

impl_from_row!(DbAllowlistEntry {
    id,
    account_id,
    sender_address,
    created_at,
});

// ── Phishing Allowlist ──────────────────────────────────────

impl_from_row!(DbPhishingAllowlistEntry {
    id,
    sender_address,
    created_at,
});

// ── Template ────────────────────────────────────────────────

impl_from_row!(DbTemplate {
    id,
    account_id,
    name,
    subject,
    body_html,
    shortcut,
    sort_order,
    created_at,
});

// ── Signature ───────────────────────────────────────────────

impl_from_row!(DbSignature {
    id,
    account_id,
    name,
    body_html,
    body_text,
    is_default,
    is_reply_default,
    sort_order,
    source,
    server_id,
    server_html_hash,
    last_synced_at,
    created_at,
});

// ── Send-As Alias ──────────────────────────────────────────

impl_from_row!(DbSendAsAlias {
    id,
    account_id,
    email,
    display_name,
    reply_to_address,
    signature_id,
    is_primary,
    is_default,
    treat_as_alias,
    verification_status,
    created_at,
});

// ── Local Draft ────────────────────────────────────────────

impl_from_row!(DbLocalDraft {
    id,
    account_id,
    to_addresses,
    cc_addresses,
    bcc_addresses,
    subject,
    body_html,
    reply_to_message_id,
    thread_id,
    from_email,
    signature_id,
    remote_draft_id,
    attachments,
    created_at,
    updated_at,
    sync_status,
});

// ── Scheduled Email ────────────────────────────────────────

impl_from_row!(DbScheduledEmail {
    id,
    account_id,
    to_addresses,
    cc_addresses,
    bcc_addresses,
    subject,
    body_html,
    reply_to_message_id,
    thread_id,
    scheduled_at,
    signature_id,
    attachment_paths,
    status,
    created_at,
    delegation,
    remote_message_id,
    remote_status,
    timezone,
    from_email,
    error_message,
    retry_count,
});

// ── Subscription Entry ─────────────────────────────────────

impl_from_row!(SubscriptionEntry {
    from_address,
    from_name,
    latest_unsubscribe_header,
    latest_unsubscribe_post,
    message_count,
    latest_date,
    status,
});

// ── Simple helper types ─────────────────────────────────────

impl_from_row!(CategoryCount {
    category,
    count,
});

impl_from_row!(ThreadCategoryRow {
    thread_id,
    category,
});

impl_from_row!(SnoozedThread {
    id,
    account_id,
});

impl_from_row!(TriggeredFollowUp {
    id,
    account_id,
    thread_id,
    subject,
});

impl_from_row!(SortOrderItem {
    id,
    sort_order,
});

impl_from_row!(LabelSortOrderItem {
    id,
    sort_order,
});

impl_from_row!(ImapMessageRow {
    id,
    imap_uid,
    imap_folder,
});

impl_from_row!(SpecialFolderRow {
    imap_folder_path,
    name,
});

impl_from_row!(ThreadInfoRow {
    id,
    subject,
    snippet,
    from_address,
});

impl_from_row!(FolderUnreadCount {
    folder_id,
    unread_count,
});

impl_from_row!(FolderAccountUnreadCount {
    folder_id,
    account_id,
    unread_count,
});

impl_from_row!(ThreadCategoryWithManual {
    category,
    bool is_manual,
});

impl_from_row!(AttachmentSender {
    from_address,
    from_name,
    count,
});

// ── Contact Stats ───────────────────────────────────────────

impl_from_row!(ContactStats {
    email_count as "cnt",
    first_email as "first_date",
    last_email as "last_date",
});

// ── Same Domain Contact ────────────────────────────────────

impl_from_row!(SameDomainContact {
    email,
    display_name,
    avatar_url,
});

// ── Recent Thread ──────────────────────────────────────────

impl_from_row!(RecentThread {
    thread_id,
    subject,
    last_message_at,
});

// ── Contact Attachment Row ─────────────────────────────────

impl_from_row!(ContactAttachmentRow {
    filename,
    mime_type,
    size,
    date,
});

// ── Cached Attachment Row ──────────────────────────────────

impl_from_row!(CachedAttachmentRow {
    id,
    local_path,
    cache_size,
    content_hash,
});

// ── Contact Group ──────────────────────────────────────────

impl_from_row!(DbContactGroup {
    id,
    name,
    member_count,
    created_at,
    updated_at,
});

// ── Contact Group Member ───────────────────────────────────

impl_from_row!(DbContactGroupMember {
    member_type,
    member_value,
});

// ── Backfill Row ───────────────────────────────────────────

impl_from_row!(BackfillRow {
    thread_id,
    subject,
    snippet,
    from_address,
    from_name,
    to_addresses,
    has_attachments,
    id,
});

// ── Uncached Attachment ────────────────────────────────────

impl_from_row!(UncachedAttachment {
    id,
    message_id,
    account_id,
    size,
    gmail_attachment_id,
    imap_part_id,
});
