//! Chat fixtures for the dev database.
//!
//! For each account, seeds N designated chat contacts plus 1-3 strictly-1:1
//! threads of short conversational messages per contact. Sets `is_chat_thread = 1`,
//! populates `thread_participants`, and writes a summarised `chat_contacts` row.
//!
//! Does not exercise the live `designate_chat_contact_sync` path - that
//! function recomputes `is_chat_thread` flags via SQL aggregation, which
//! depends on every message already being inserted with consistent
//! `thread_participants` data. Doing the writes inline here is simpler and
//! deterministic.

use std::collections::HashSet;

use rand::RngExt;
use rusqlite::Connection;

use crate::accounts::Account;
use crate::contacts;
use crate::people::{I18N_LOCALES, PeoplePools, Person};
use crate::templates;
use crate::threads::{PendingBody, PendingInlineImage, SeedStats};
use crate::weighted_choice;

const CHAT_IMAP_UID_BASE: u32 = 100_000;

/// Probability that a chat message also carries an inline image.
const CHAT_INLINE_IMAGE_PROBABILITY: f64 = 0.20;

/// Solid-color placeholder PNGs used as inline image attachments. Each
/// palette entry is precomputed once per `seed_chats` invocation so identical
/// images share one row in `inline_images.db` (content-addressed by xxh3).
struct ChatImagePalette {
    entries: Vec<ChatImageEntry>,
}

struct ChatImageEntry {
    bytes: Vec<u8>,
    content_hash: db::blob_hash::BlobHash,
    mime_type: &'static str,
}

impl ChatImagePalette {
    fn new() -> Self {
        // Distinct hues so the seeded images look like different photos
        // rather than copies of one image. Sized small enough to keep the
        // inline_images.db payload trivial - each PNG is well under 1 KB.
        const SWATCHES: &[(u8, u8, u8, u32, u32)] = &[
            (0xee, 0x8a, 0x6f, 320, 240),
            (0x6f, 0xa3, 0xee, 320, 240),
            (0x8a, 0xc6, 0x70, 320, 240),
            (0xee, 0xc8, 0x6f, 320, 240),
            (0xb0, 0x88, 0xee, 320, 240),
            (0x6f, 0xee, 0xd5, 320, 240),
        ];
        let entries = SWATCHES
            .iter()
            .map(|&(r, g, b, w, h)| {
                let bytes = solid_png(r, g, b, w, h);
                let hash = db::blob_hash::BlobHash::hash(&bytes);
                ChatImageEntry {
                    bytes,
                    content_hash: hash,
                    mime_type: "image/png",
                }
            })
            .collect();
        Self { entries }
    }
}

/// Encode a solid-colour PNG via the `image` crate.
fn solid_png(r: u8, g: u8, b: u8, w: u32, h: u32) -> Vec<u8> {
    let buf = image::ImageBuffer::from_fn(w, h, |_, _| image::Rgb([r, g, b]));
    let mut out = std::io::Cursor::new(Vec::new());
    buf.write_to(&mut out, image::ImageFormat::Png)
        .expect("encode solid PNG");
    out.into_inner()
}

static CHAT_SUBJECTS: &[&str] = &[
    "quick question",
    "lunch tomorrow?",
    "did you see this?",
    "thoughts?",
    "any update?",
    "got a sec?",
    "can you check this?",
    "hey",
    "yo",
    "tomorrow",
    "Friday lunch",
    "thanks!",
    "saw your message",
    "fyi",
    "small thing",
    "weekend?",
    "the deck",
    "follow up",
    "calendar",
    "one more thing",
];

static CHAT_BODY_SHORT: &[&str] = &[
    "yeah that works",
    "sounds good",
    "got it, thanks!",
    "haha okay",
    "on it",
    "kk",
    "great, see you then",
    "tomorrow at 2?",
    "lol",
    "oof, sorry",
    "wait what",
    "no worries",
    "yes please",
    "perfect",
    "let me check",
    "give me 5 min",
    "running late",
    "be there in 10",
    "agreed",
    "totally",
    "hmm not sure",
    "send it over when you can",
    "can do",
    "👍",
    "did you ever hear back from them?",
    "any thoughts on the timing?",
    "I can do Wednesday if that works",
    "moving this to next week if that's okay",
    "did you mean the other one?",
    "I'll loop you in once I have an answer",
    "ping me when you're back",
    "let's sync about this offline",
    "noted, will follow up",
    "want me to send a calendar invite?",
    "free tomorrow afternoon?",
    "ah okay that makes sense",
    "yeah I saw that earlier",
    "agreed, let's go with option 2",
    "let me chew on it tonight",
    "happy to chat through it whenever",
];

static CHAT_BODY_LONG: &[&str] = &[
    "Just wanted to flag that I won't be around Friday afternoon. Can we move the sync to Thursday? Same time works for me.",
    "Quick recap from yesterday: we're aligned on the scope, but the timeline depends on when ops can free up the staging cluster. I'll chase them today.",
    "Read through the doc - mostly looks great. Two small nits in the data model section, otherwise lgtm. Want me to leave inline comments or just chat about it?",
    "Hey! Realised I never responded to your earlier message - sorry about that. Yes I'm in for the trip, let me know what dates you're thinking and I'll book.",
    "Got the numbers from finance. Short version: we're under budget this quarter, so the extra headcount is on the table if we want to push for it. Worth discussing.",
    "Heads up - the PR I sent earlier needs another pass after the review feedback. I'll have an updated version up tomorrow morning. No rush on your side.",
    "Saw the announcement! Congrats, that's a big one. Drinks soon to celebrate? My calendar is mostly clear next week.",
    "FYI the meeting got rescheduled to 3pm. Same room. Sorry for the late notice - the room got double-booked.",
    "Spent some time digging into the bug report. It's reproducible but only on the older clients - newer ones short-circuit the path entirely. Will write it up properly tomorrow.",
    "Quick favour - could you forward me the email from legal about the new terms? I can't find it in my inbox and I think it ended up in spam.",
];

/// Generate the body for a chat message. ~85 % short, ~15 % long.
fn chat_body(rng: &mut impl RngExt) -> String {
    let pool: &[&str] = if rng.random::<f64>() < 0.15 {
        CHAT_BODY_LONG
    } else {
        CHAT_BODY_SHORT
    };
    let line = pool[rng.random_range(0..pool.len())];
    format!("<p>{line}</p>")
}

/// Pick a per-account locale (per spec: depends on `locale_mode`).
fn pick_locale_idx(rng: &mut impl RngExt, locale_mode: &str) -> Option<usize> {
    match locale_mode {
        "latin" => None,
        "intl" => Some(rng.random_range(0..I18N_LOCALES.len())),
        _ => {
            if rng.random::<f64>() < 0.30 {
                Some(rng.random_range(0..I18N_LOCALES.len()))
            } else {
                None
            }
        }
    }
}

/// Resolve the chat partner pool for a given locale. Falls back to Latin if
/// the chosen i18n pool happens to be empty.
fn partner_pool(pools: &PeoplePools, locale_idx: Option<usize>) -> &[Person] {
    match locale_idx {
        Some(idx) if !pools.i18n[idx].is_empty() => &pools.i18n[idx],
        _ => &pools.latin,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn seed_chats(
    conn: &Connection,
    rng: &mut impl RngExt,
    accounts: &[Account],
    pools: &PeoplePools,
    locale_mode: &str,
    chats: u32,
    bodies: &mut Vec<PendingBody>,
    inline_images: &mut Vec<PendingInlineImage>,
    stats: &mut SeedStats,
) -> Result<(), String> {
    if chats == 0 || accounts.is_empty() {
        return Ok(());
    }

    // Anchor chat timestamps to wall-clock time so freshly-seeded data always
    // looks recent in the sidebar, regardless of how far the project's
    // `threads.rs` FIXED_NOW has drifted into the past. Determinism within a
    // single run is preserved by the seeded RNG; the only drift across days
    // is the absolute timestamps themselves, which is what we want here.
    let now = chrono::Utc::now().timestamp();
    let palette = ChatImagePalette::new();
    let mut already_used: HashSet<String> = HashSet::new();
    let mut palette_pushed: HashSet<db::blob_hash::BlobHash> = HashSet::new();
    let mut sort_order: i64 = 0;
    let mut imap_uid_counter: u32 = CHAT_IMAP_UID_BASE;
    let mut picked_total: u32 = 0;

    // Bound the search so a tiny pool can't loop forever.
    let pool_size = pools.latin.len() + pools.i18n.iter().map(Vec::len).sum::<usize>();
    let pool_cap: u32 = u32::try_from(pool_size).unwrap_or(u32::MAX);
    let attempt_limit = chats.saturating_mul(20).max(pool_cap);
    let mut attempts: u32 = 0;

    while picked_total < chats && attempts < attempt_limit {
        attempts += 1;

        let locale_idx = pick_locale_idx(rng, locale_mode);
        let pool = partner_pool(pools, locale_idx);
        if pool.is_empty() {
            continue;
        }
        let partner = &pool[rng.random_range(0..pool.len())];

        // chat_contacts is keyed globally by email (no account_id) - the
        // sidebar shows one cross-account list. Each partner is hosted on a
        // single account for v1; multi-account-per-partner is a Phase 6
        // polish item.
        let acc = &accounts[(picked_total as usize) % accounts.len()];

        // Don't reuse a partner globally, and don't pick the hosting
        // account's own email as a partner.
        if partner.email.eq_ignore_ascii_case(&acc.email)
            || already_used.contains(&partner.email.to_lowercase())
        {
            continue;
        }
        already_used.insert(partner.email.to_lowercase());
        picked_total += 1;

        let n_threads: u32 = weighted_choice(rng, &[1u32, 2, 3], &[0.5, 0.3, 0.2]);

        // Track the contact-level summary across all the threads we'll
        // insert for this partner.
        let mut contact_latest_date: i64 = 0;
        let mut contact_latest_preview: Option<String> = None;
        let mut contact_unread: i64 = 0;

        // Designation pre-dates the conversation.
        let designated_at = now - i64::from(rng.random_range(30..120)) * 86_400;

        for _ in 0..n_threads {
            let thread_id = crate::next_uuid(rng);

            // Subject (replies in this thread reuse the same root subject
            // with a Re: prefix, mirroring threads::generate_threads).
            let subject = CHAT_SUBJECTS[rng.random_range(0..CHAT_SUBJECTS.len())].to_string();

            let n_msgs: u32 = weighted_choice(
                rng,
                &[4u32, 5, 6, 7, 8, 10, 12, 18, 25],
                &[0.18, 0.20, 0.15, 0.12, 0.10, 0.10, 0.07, 0.05, 0.03],
            );

            // Pick the latest message's timestamp uniformly across the
            // visible window, then walk backwards. This guarantees every
            // message lands at-or-before `now` regardless of n_msgs and gap
            // sizes - the previous "pick start, walk forward" approach
            // could spill messages into the future for long threads.
            let latest_at = now - i64::from(rng.random_range(300..(60 * 86_400)));

            // Walk backwards from latest_at to assign each message a date.
            // Gap range matches the conversational pace: 30 min - 8 h apart.
            let n_msgs_usize = n_msgs as usize;
            let mut msg_dates: Vec<i64> = vec![0; n_msgs_usize];
            msg_dates[n_msgs_usize - 1] = latest_at;
            for i in (0..n_msgs_usize.saturating_sub(1)).rev() {
                let gap = i64::from(rng.random_range(30..480)) * 60;
                msg_dates[i] = msg_dates[i + 1] - gap;
            }
            let thread_start = msg_dates[0];

            // Pre-create the thread row; we'll update aggregate fields
            // after inserting all messages.
            conn.execute(
                "INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
                 message_count, is_read, is_starred, has_attachments,
                 is_important, is_pinned, is_snoozed, snooze_until, is_muted,
                 is_chat_thread)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5, 1, 0, 0, 0, 0, 0, NULL, 0, 1)",
                rusqlite::params![thread_id, acc.id, subject, thread_start, n_msgs,],
            )
            .map_err(|e| format!("insert chat thread: {e}"))?;
            stats.chat_threads += 1;

            // Decide unread tail BEFORE generating messages, so we know
            // which trailing slice belongs to the partner.
            let has_unread_tail = rng.random::<f64>() < 0.40;
            let unread_tail_size: u32 = if has_unread_tail {
                rng.random_range(1..=3).min(n_msgs)
            } else {
                0
            };

            let mut msg_refs: Vec<String> = Vec::new();
            let mut latest_date: i64 = 0;
            let mut latest_snippet = String::new();
            let mut thread_is_read = true;

            for mi in 0..n_msgs {
                let msg_id = crate::next_uuid(rng);
                let msg_id_header = crate::next_message_id(rng);

                let in_reply_to = if mi == 0 { None } else { msg_refs.last().cloned() };
                let references = if mi == 0 { None } else { Some(msg_refs.join(" ")) };
                msg_refs.push(msg_id_header.clone());

                // Start with the partner (mi=0 is inbound), then alternate.
                let from_partner = mi % 2 == 0;
                let (sender_name, sender_email, to_addr, folder_name) = if from_partner {
                    (
                        partner.display_name.clone(),
                        partner.email.clone(),
                        format!("{} <{}>", acc.display_name, acc.email),
                        "INBOX",
                    )
                } else {
                    (
                        acc.display_name.clone(),
                        acc.email.clone(),
                        format!("{} <{}>", partner.display_name, partner.email),
                        "Sent",
                    )
                };

                let msg_date = msg_dates[mi as usize];

                let msg_subject = if mi == 0 {
                    subject.clone()
                } else {
                    format!("Re: {subject}")
                };

                // Body + snippet.
                let body_html = chat_body(rng);
                let body_text = templates::strip_html(&body_html);
                let snippet = body_text.chars().take(200).collect::<String>();

                // Unread state: trailing `unread_tail_size` partner
                // messages are unread; everything else is read.
                let is_in_unread_tail =
                    unread_tail_size > 0 && mi >= n_msgs - unread_tail_size;
                let msg_is_read = !(from_partner && is_in_unread_tail);
                if !msg_is_read {
                    thread_is_read = false;
                    contact_unread += 1;
                }

                imap_uid_counter += 1;
                let imap_uid = imap_uid_counter;

                let raw_size = rng.random_range(800..6_000);

                conn.execute(
                    "INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                     to_addresses, cc_addresses, subject, snippet, date,
                     is_read, is_starred, body_cached, raw_size, internal_date,
                     message_id_header, references_header, in_reply_to_header,
                     imap_uid, imap_folder, list_unsubscribe, list_unsubscribe_post)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9,
                             ?10, 0, 1, ?11, ?9, ?12, ?13, ?14, ?15, ?16, NULL, NULL)",
                    rusqlite::params![
                        msg_id,
                        acc.id,
                        thread_id,
                        sender_email,
                        sender_name,
                        to_addr,
                        msg_subject,
                        snippet,
                        msg_date,
                        msg_is_read as i32,
                        raw_size,
                        msg_id_header,
                        references,
                        in_reply_to,
                        imap_uid,
                        folder_name,
                    ],
                )
                .map_err(|e| format!("insert chat message: {e}"))?;
                stats.chat_messages += 1;
                stats.messages += 1;

                bodies.push(PendingBody {
                    message_id: msg_id.clone(),
                    body_html,
                    body_text,
                });

                // ~CHAT_INLINE_IMAGE_PROBABILITY of messages carry one inline
                // image. The image is one of the precomputed palette entries;
                // unique blobs are pushed into the inline image store once.
                if !palette.entries.is_empty()
                    && rng.random::<f64>() < CHAT_INLINE_IMAGE_PROBABILITY
                {
                    let entry = &palette.entries[rng.random_range(0..palette.entries.len())];
                    let attach_id = crate::next_uuid(rng);
                    let hash_hex = entry.content_hash.to_hex();
                    let cid = format!("<{hash_hex}@chat.ratatoskr.test>");
                    let size = i64::try_from(entry.bytes.len()).unwrap_or(i64::MAX);
                    let filename = format!("inline-{}.png", &hash_hex[..8]);

                    conn.execute(
                        "INSERT INTO attachments (id, message_id, account_id, filename, \
                         mime_type, size, content_id, is_inline, content_hash) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8)",
                        rusqlite::params![
                            attach_id,
                            msg_id,
                            acc.id,
                            filename,
                            entry.mime_type,
                            size,
                            cid,
                            entry.content_hash,
                        ],
                    )
                    .map_err(|e| format!("insert chat attachment: {e}"))?;
                    stats.attachments += 1;
                    stats.chat_inline_images += 1;

                    if palette_pushed.insert(entry.content_hash) {
                        inline_images.push(PendingInlineImage {
                            content_hash: hash_hex,
                            bytes: entry.bytes.clone(),
                            mime_type: entry.mime_type.to_string(),
                        });
                    }
                }

                contacts::upsert_contact(
                    conn,
                    rng,
                    &partner.email,
                    &partner.display_name,
                    &acc.id,
                    msg_date,
                )?;

                if msg_date > latest_date {
                    latest_date = msg_date;
                    latest_snippet.clone_from(&snippet);
                }
            }

            // Finalise thread aggregates.
            conn.execute(
                "UPDATE threads SET snippet = ?1, last_message_at = ?2, is_read = ?3
                 WHERE account_id = ?4 AND id = ?5",
                rusqlite::params![
                    latest_snippet,
                    latest_date,
                    thread_is_read as i32,
                    acc.id,
                    thread_id,
                ],
            )
            .map_err(|e| format!("update chat thread: {e}"))?;

            // thread_participants: exactly the two endpoints. The unique
            // PK guarantees us a 1:1 thread by construction.
            for endpoint in [&partner.email, &acc.email] {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_participants
                     (account_id, thread_id, email)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![acc.id, thread_id, endpoint],
                )
                .map_err(|e| format!("insert thread_participants: {e}"))?;
            }

            // INBOX label and All Mail label - mirrors the convention in
            // threads::generate_threads for INBOX threads.
            if let Some((_, label_id)) = acc.labels.iter().find(|(name, _)| name == "INBOX") {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![thread_id, acc.id, label_id],
                )
                .map_err(|e| format!("insert chat thread INBOX label: {e}"))?;
            }
            if let Some((_, label_id)) = acc.labels.iter().find(|(name, _)| name == "All Mail") {
                conn.execute(
                    "INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![thread_id, acc.id, label_id],
                )
                .map_err(|e| format!("insert chat thread All Mail label: {e}"))?;
            }

            if latest_date > contact_latest_date {
                contact_latest_date = latest_date;
                contact_latest_preview = Some(latest_snippet);
            }
        }

        // chat_contacts row, summarised from the threads we just inserted.
        conn.execute(
            "INSERT INTO chat_contacts
             (email, designated_at, sort_order, display_name,
              latest_message_at, latest_message_preview, unread_count, contact_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            rusqlite::params![
                partner.email,
                designated_at,
                sort_order,
                partner.display_name,
                contact_latest_date,
                contact_latest_preview,
                contact_unread,
            ],
        )
        .map_err(|e| format!("insert chat_contact: {e}"))?;
        stats.chat_contacts += 1;
        sort_order += 1;
    }

    Ok(())
}
