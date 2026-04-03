use std::collections::HashMap;

use rand::Rng;
use rusqlite::Connection;

use crate::accounts::Account;
use crate::contacts;
use crate::people::{I18N_LOCALES, PeoplePools, Person};
use crate::templates::{self, CATEGORIES, CATEGORY_WEIGHTS, Category};

/// Fixed reference timestamp for deterministic output.
/// 2026-03-15 12:00:00 UTC — arbitrary but stable across runs.
const FIXED_NOW: i64 = 1_773_768_000;

struct AttachmentInfo {
    filename: &'static str,
    mime_type: &'static str,
    base_size: i64,
}

static ATTACHMENT_POOL: &[AttachmentInfo] = &[
    AttachmentInfo {
        filename: "report.pdf",
        mime_type: "application/pdf",
        base_size: 245_000,
    },
    AttachmentInfo {
        filename: "screenshot.png",
        mime_type: "image/png",
        base_size: 890_000,
    },
    AttachmentInfo {
        filename: "proposal.docx",
        mime_type: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        base_size: 156_000,
    },
    AttachmentInfo {
        filename: "data.xlsx",
        mime_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        base_size: 78_000,
    },
    AttachmentInfo {
        filename: "design-v3.fig",
        mime_type: "application/octet-stream",
        base_size: 2_400_000,
    },
    AttachmentInfo {
        filename: "meeting-notes.md",
        mime_type: "text/markdown",
        base_size: 4_200,
    },
    AttachmentInfo {
        filename: "invoice-2024.pdf",
        mime_type: "application/pdf",
        base_size: 67_000,
    },
    AttachmentInfo {
        filename: "photo.jpg",
        mime_type: "image/jpeg",
        base_size: 3_100_000,
    },
    AttachmentInfo {
        filename: "logo.svg",
        mime_type: "image/svg+xml",
        base_size: 12_000,
    },
    AttachmentInfo {
        filename: "archive.zip",
        mime_type: "application/zip",
        base_size: 15_600_000,
    },
    AttachmentInfo {
        filename: "presentation.pptx",
        mime_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        base_size: 4_500_000,
    },
    AttachmentInfo {
        filename: "contract-final.pdf",
        mime_type: "application/pdf",
        base_size: 189_000,
    },
    AttachmentInfo {
        filename: "wireframes.pdf",
        mime_type: "application/pdf",
        base_size: 1_200_000,
    },
    AttachmentInfo {
        filename: "budget.csv",
        mime_type: "text/csv",
        base_size: 23_000,
    },
];

/// Weighted random selection.
fn weighted_choice<T: Copy>(rng: &mut impl Rng, items: &[T], weights: &[f64]) -> T {
    let total: f64 = weights.iter().sum();
    let mut r: f64 = rng.random::<f64>() * total;
    for (item, weight) in items.iter().zip(weights.iter()) {
        r -= weight;
        if r <= 0.0 {
            return *item;
        }
    }
    items[items.len() - 1]
}

/// Body to be inserted into the body store after the main transaction.
pub struct PendingBody {
    pub message_id: String,
    pub body_html: String,
    pub body_text: String,
}

pub struct SeedStats {
    pub threads: u32,
    pub messages: u32,
    pub attachments: u32,
}

#[allow(clippy::too_many_lines)]
pub fn generate_threads(
    conn: &Connection,
    rng: &mut impl Rng,
    accounts: &[Account],
    pools: &PeoplePools,
    locale_mode: &str,
    num_threads: u32,
) -> Result<(Vec<PendingBody>, SeedStats), String> {
    let mut bodies = Vec::new();
    let mut stats = SeedStats {
        threads: 0,
        messages: 0,
        attachments: 0,
    };
    let mut imap_uid_counter: HashMap<(String, String), u32> = HashMap::new();
    let now = FIXED_NOW;

    for _ in 0..num_threads {
        let acc = &accounts[rng.random_range(0..accounts.len())];
        let cat = weighted_choice(rng, CATEGORIES, CATEGORY_WEIGHTS);

        // Pick locale for this thread
        let locale_idx: Option<usize> = match locale_mode {
            "latin" => None,
            "intl" => Some(rng.random_range(0..I18N_LOCALES.len())),
            _ => {
                // mixed: ~30% non-Latin
                if rng.random::<f64>() < 0.30 {
                    Some(rng.random_range(0..I18N_LOCALES.len()))
                } else {
                    None
                }
            }
        };

        let locale_data = locale_idx.map(|i| &I18N_LOCALES[i]);
        let re_prefix = locale_data.map_or("Re:", |l| l.re_prefix);
        let thread_people: &[Person] = if let Some(idx) = locale_idx {
            &pools.i18n[idx]
        } else {
            &pools.latin
        };

        if thread_people.is_empty() {
            continue;
        }

        let subject = templates::generate_subject(rng, cat, locale_data);

        // Thread timing: random start within last 365 days
        let days_back = rng.random_range(0..365);
        let hours_back = rng.random_range(0..24);
        let mins_back = rng.random_range(0..60);
        let thread_start = now - (days_back * 86400 + hours_back * 3600 + mins_back * 60) as i64;

        // Number of messages
        let num_msgs: u32 = match cat {
            Category::Newsletter | Category::Notification => 1,
            Category::Commerce => weighted_choice(rng, &[1, 2, 3], &[0.7, 0.2, 0.1]),
            Category::Work => weighted_choice(
                rng,
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 12],
                &[0.10, 0.12, 0.12, 0.10, 0.13, 0.10, 0.10, 0.10, 0.08, 0.05],
            ),
            Category::Personal => weighted_choice(
                rng,
                &[1, 2, 3, 4, 5, 8, 12],
                &[0.30, 0.25, 0.15, 0.10, 0.10, 0.05, 0.05],
            ),
        };

        // Participants
        let max_participants = thread_people.len().min(5);
        let num_participants: usize =
            weighted_choice(rng, &[1usize, 2, 3, 4, 5], &[0.1, 0.4, 0.25, 0.15, 0.1])
                .min(max_participants);

        // Sample participants (Fisher-Yates partial shuffle on indices)
        let mut indices: Vec<usize> = (0..thread_people.len()).collect();
        for i in 0..num_participants {
            let j = rng.random_range(i..indices.len());
            indices.swap(i, j);
        }
        let participants: Vec<&Person> = indices[..num_participants]
            .iter()
            .map(|&i| &thread_people[i])
            .collect();

        let thread_id = crate::next_uuid(rng);
        let is_read = rng.random::<f64>() < 0.7;
        let is_starred = rng.random::<f64>() < 0.08;
        let is_pinned = rng.random::<f64>() < 0.02;
        let is_snoozed = rng.random::<f64>() < 0.03;
        let snooze_until: Option<i64> = if is_snoozed {
            Some(now + i64::from(rng.random_range(1..15)) * 86400)
        } else {
            None
        };
        let is_important = rng.random::<f64>() < 0.05;
        let is_muted = rng.random::<f64>() < 0.01;
        let mut has_attachments = false;

        // Folder
        let folder_name = weighted_choice(
            rng,
            &["INBOX", "Sent", "Archive", "Trash", "Spam", "Drafts"],
            &[0.70, 0.10, 0.12, 0.03, 0.02, 0.03],
        );

        // Insert thread first (FK target for messages), update with final values after
        conn.execute(
            "INSERT INTO threads (id, account_id, subject, snippet, last_message_at,
             message_count, is_read, is_starred, has_attachments,
             is_important, is_pinned, is_snoozed, snooze_until, is_muted)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                thread_id,
                acc.id,
                subject,
                thread_start,
                num_msgs,
                is_read as i32,
                is_starred as i32,
                is_important as i32,
                is_pinned as i32,
                is_snoozed as i32,
                snooze_until,
                is_muted as i32,
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
        stats.threads += 1;

        // Build messages
        let mut msg_refs: Vec<String> = Vec::new();
        let mut latest_date: i64 = 0;
        let mut latest_snippet = String::new();

        for mi in 0..num_msgs {
            let msg_id = crate::next_uuid(rng);
            let msg_id_header = crate::next_message_id(rng);

            let in_reply_to = if mi == 0 {
                None
            } else {
                msg_refs.last().cloned()
            };
            let references = if mi == 0 {
                None
            } else {
                Some(msg_refs.join(" "))
            };
            msg_refs.push(msg_id_header.clone());

            // Alternate sender between participants and self
            let (sender_name, sender_email, to_addr);
            if num_msgs == 1 {
                sender_name = participants[0].display_name.clone();
                sender_email = participants[0].email.clone();
                to_addr = format!("{} <{}>", acc.display_name, acc.email);
            } else if mi % 2 == 0 {
                let p = &participants[(mi as usize) % participants.len()];
                sender_name = p.display_name.clone();
                sender_email = p.email.clone();
                to_addr = format!("{} <{}>", acc.display_name, acc.email);
            } else {
                sender_name = acc.display_name.clone();
                sender_email = acc.email.clone();
                to_addr = participants
                    .iter()
                    .take(3)
                    .map(|p| format!("{} <{}>", p.display_name, p.email))
                    .collect::<Vec<_>>()
                    .join(", ");
            }

            // CC sometimes
            let cc: Option<String> = if num_participants > 2 && rng.random::<f64>() < 0.3 {
                Some(
                    participants[2..participants.len().min(4)]
                        .iter()
                        .map(|p| format!("{} <{}>", p.display_name, p.email))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
            } else {
                None
            };

            // Message date
            let msg_date = thread_start
                + i64::from(mi) * i64::from(rng.random_range(1..49)) * 3600
                + i64::from(rng.random_range(0..60)) * 60;

            let msg_subject = if mi == 0 {
                subject.clone()
            } else {
                format!("{re_prefix} {subject}")
            };
            let snippet = msg_subject.chars().take(200).collect::<String>();

            let msg_is_read = if mi < num_msgs - 1 { true } else { is_read };

            // Attachments (~20% of work/personal/commerce)
            let mut msg_attachment_ids: Vec<(String, &'static str, &'static str, i64)> = Vec::new();
            if rng.random::<f64>() < 0.20
                && matches!(
                    cat,
                    Category::Work | Category::Personal | Category::Commerce
                )
            {
                let num_att: u32 = weighted_choice(rng, &[1, 2, 3], &[0.6, 0.3, 0.1]);
                for _ in 0..num_att {
                    let att = &ATTACHMENT_POOL[rng.random_range(0..ATTACHMENT_POOL.len())];
                    let size_var = att.base_size / 4;
                    let size = att.base_size + rng.random_range(-size_var..size_var);
                    msg_attachment_ids.push((
                        crate::next_uuid(rng),
                        att.filename,
                        att.mime_type,
                        size,
                    ));
                }
                has_attachments = true;
            }

            // IMAP UID
            let folder_key = (acc.id.clone(), folder_name.to_string());
            let uid_counter = imap_uid_counter.entry(folder_key).or_insert(0);
            *uid_counter += 1;
            let imap_uid = *uid_counter;

            // List-Unsubscribe for newsletters
            let (list_unsub, list_unsub_post) = if cat == Category::Newsletter {
                let mut bytes = [0u8; 6];
                rng.fill(&mut bytes);
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                (
                    Some(format!(
                        "<https://newsletter.example.com/unsubscribe/{hex}>"
                    )),
                    Some("List-Unsubscribe=One-Click".to_string()),
                )
            } else {
                (None, None)
            };

            let raw_size = rng.random_range(2000..50001);

            // Insert message
            conn.execute(
                "INSERT INTO messages (id, account_id, thread_id, from_address, from_name,
                 to_addresses, cc_addresses, subject, snippet, date,
                 is_read, is_starred, body_cached, raw_size, internal_date,
                 message_id_header, references_header, in_reply_to_header,
                 imap_uid, imap_folder, list_unsubscribe, list_unsubscribe_post)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                         ?11, ?12, 1, ?13, ?10, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                rusqlite::params![
                    msg_id,
                    acc.id,
                    thread_id,
                    sender_email,
                    sender_name,
                    to_addr,
                    cc,
                    msg_subject,
                    snippet,
                    msg_date,
                    msg_is_read as i32,
                    (is_starred && mi == 0) as i32,
                    raw_size,
                    msg_id_header,
                    references,
                    in_reply_to,
                    imap_uid,
                    folder_name,
                    list_unsub,
                    list_unsub_post,
                ],
            )
            .map_err(|e| format!("insert message: {e}"))?;
            stats.messages += 1;

            // Generate body
            let body_html = templates::generate_body(rng, cat, locale_data);
            let body_text = templates::strip_html(&body_html);
            bodies.push(PendingBody {
                message_id: msg_id.clone(),
                body_html,
                body_text,
            });

            // Insert attachments
            for (att_id, filename, mime_type, size) in &msg_attachment_ids {
                conn.execute(
                    "INSERT INTO attachments (id, message_id, account_id, filename, mime_type, size)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![att_id, msg_id, acc.id, filename, mime_type, size],
                )
                .map_err(|e| format!("insert attachment: {e}"))?;
                stats.attachments += 1;
            }

            // Upsert sender into contacts
            contacts::upsert_contact(conn, rng, &sender_email, &sender_name, &acc.id, msg_date)?;

            // Upsert recipients into contacts
            for p in &participants {
                contacts::upsert_contact(conn, rng, &p.email, &p.display_name, &acc.id, msg_date)?;
            }

            if msg_date > latest_date {
                latest_date = msg_date;
                latest_snippet.clone_from(&snippet);
            }
        }

        // Update thread with final computed values
        conn.execute(
            "UPDATE threads SET snippet = ?1, last_message_at = ?2, has_attachments = ?3
             WHERE account_id = ?4 AND id = ?5",
            rusqlite::params![
                latest_snippet,
                latest_date,
                has_attachments as i32,
                acc.id,
                thread_id,
            ],
        )
        .map_err(|e| format!("update thread: {e}"))?;

        // Thread labels: folder label
        if let Some((_, label_id)) = acc.labels.iter().find(|(name, _)| name == folder_name) {
            conn.execute(
                "INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![thread_id, acc.id, label_id],
            )
            .map_err(|e| format!("insert thread_label (folder): {e}"))?;
        }

        if !matches!(folder_name, "Trash" | "Spam" | "Drafts")
            && let Some((_, label_id)) = acc.labels.iter().find(|(name, _)| name == "All Mail")
        {
            conn.execute(
                "INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![thread_id, acc.id, label_id],
            )
            .map_err(|e| format!("insert thread_label (all mail): {e}"))?;
        }

        // Thread labels: user label based on category
        let category_label = match cat {
            Category::Work => acc.category_labels.work,
            Category::Personal => acc.category_labels.personal,
            Category::Newsletter => acc.category_labels.newsletters,
            Category::Commerce => acc.category_labels.receipts,
            Category::Notification => None,
        };
        if let Some(target) = category_label {
            if rng.random::<f64>() < 0.6 {
                if let Some((_, label_id)) = acc.labels.iter().find(|(name, _)| name == target) {
                    conn.execute(
                        "INSERT OR IGNORE INTO thread_labels (thread_id, account_id, label_id)
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![thread_id, acc.id, label_id],
                    )
                    .map_err(|e| format!("insert thread_label (user): {e}"))?;
                }
            }
        }
    }

    Ok((bodies, stats))
}
