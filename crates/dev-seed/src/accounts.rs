use rand::Rng;
use rusqlite::Connection;

pub struct AccountPreset {
    pub email: &'static str,
    pub name: &'static str,
    pub provider: &'static str,
    pub host: &'static str,
    pub color: &'static str,
}

pub static ACCOUNT_PRESETS: &[AccountPreset] = &[
    AccountPreset { email: "alex.morgan@gmail.com", name: "Alex Morgan", provider: "gmail_api", host: "imap.gmail.com", color: "#4285f4" },
    AccountPreset { email: "alex.morgan@company.io", name: "Alex Morgan", provider: "imap", host: "mail.company.io", color: "#ea4335" },
    AccountPreset { email: "a.morgan@outlook.com", name: "Alex Morgan", provider: "graph", host: "outlook.office365.com", color: "#fbbc04" },
    AccountPreset { email: "alex@fastmail.com", name: "Alex Morgan", provider: "jmap", host: "jmap.fastmail.com", color: "#34a853" },
];

struct SystemLabel {
    name: &'static str,
    special: &'static str,
    sort: i32,
}

static SYSTEM_LABELS: &[SystemLabel] = &[
    SystemLabel { name: "INBOX", special: "inbox", sort: 0 },
    SystemLabel { name: "Sent", special: "sent", sort: 1 },
    SystemLabel { name: "Drafts", special: "drafts", sort: 2 },
    SystemLabel { name: "Trash", special: "trash", sort: 3 },
    SystemLabel { name: "Archive", special: "archive", sort: 4 },
    SystemLabel { name: "Spam", special: "junk", sort: 5 },
];

struct UserLabel {
    name: &'static str,
    color_bg: &'static str,
    color_fg: &'static str,
}

static USER_LABELS: &[UserLabel] = &[
    UserLabel { name: "Work", color_bg: "#4285f4", color_fg: "#ffffff" },
    UserLabel { name: "Personal", color_bg: "#0b8043", color_fg: "#ffffff" },
    UserLabel { name: "Finance", color_bg: "#f4b400", color_fg: "#000000" },
    UserLabel { name: "Travel", color_bg: "#db4437", color_fg: "#ffffff" },
    UserLabel { name: "Newsletters", color_bg: "#ab47bc", color_fg: "#ffffff" },
    UserLabel { name: "Receipts", color_bg: "#00acc1", color_fg: "#ffffff" },
    UserLabel { name: "Projects", color_bg: "#ff7043", color_fg: "#ffffff" },
    UserLabel { name: "Waiting", color_bg: "#8d6e63", color_fg: "#ffffff" },
];

/// Inserted account info needed by later stages.
pub struct Account {
    pub id: String,
    pub email: String,
    pub name: String,
    /// Map from label name -> label id
    pub labels: Vec<(String, String)>,
}

pub fn seed_accounts(
    conn: &Connection,
    rng: &mut impl Rng,
    num_accounts: u32,
) -> Result<Vec<Account>, String> {
    let count = (num_accounts as usize).min(ACCOUNT_PRESETS.len());
    let mut accounts = Vec::with_capacity(count);

    for preset in &ACCOUNT_PRESETS[..count] {
        let account_id = crate::next_uuid(rng);

        conn.execute(
            "INSERT INTO accounts (id, email, display_name, provider, imap_host, \
             imap_port, imap_security, smtp_host, smtp_port, smtp_security, \
             auth_method, account_color, account_name, sort_order, initial_sync_completed, is_active) \
             VALUES (?1, ?2, ?3, ?4, ?5, 993, 'tls', ?5, 587, 'starttls', \
                     'oauth2', ?6, ?3, ?7, 1, 1)",
            rusqlite::params![
                account_id,
                preset.email,
                preset.name,
                preset.provider,
                preset.host,
                preset.color,
                accounts.len() as i32,
            ],
        )
        .map_err(|e| format!("insert account: {e}"))?;

        // Labels
        let mut labels = Vec::new();

        for sl in SYSTEM_LABELS {
            let label_id = crate::next_uuid(rng);
            conn.execute(
                "INSERT INTO labels (id, account_id, name, type, visible, sort_order, \
                 imap_special_use, label_kind) \
                 VALUES (?1, ?2, ?3, 'system', 1, ?4, ?5, 'container')",
                rusqlite::params![label_id, account_id, sl.name, sl.sort, sl.special],
            )
            .map_err(|e| format!("insert system label: {e}"))?;
            labels.push((sl.name.to_string(), label_id));
        }

        for (i, ul) in USER_LABELS.iter().enumerate() {
            let label_id = crate::next_uuid(rng);
            conn.execute(
                "INSERT INTO labels (id, account_id, name, type, color_bg, color_fg, \
                 visible, sort_order, label_kind) \
                 VALUES (?1, ?2, ?3, 'user', ?4, ?5, 1, ?6, 'tag')",
                rusqlite::params![
                    label_id,
                    account_id,
                    ul.name,
                    ul.color_bg,
                    ul.color_fg,
                    (SYSTEM_LABELS.len() + i) as i32,
                ],
            )
            .map_err(|e| format!("insert user label: {e}"))?;
            labels.push((ul.name.to_string(), label_id));
        }

        // Signature
        let sig_id = crate::next_uuid(rng);
        let sig_html = format!(
            "<p>Best regards,<br><strong>{}</strong><br>{}</p>",
            preset.name, preset.email
        );
        let sig_text = format!("Best regards,\n{}\n{}", preset.name, preset.email);
        conn.execute(
            "INSERT INTO signatures (id, account_id, name, body_html, body_text, is_default) \
             VALUES (?1, ?2, 'Default', ?3, ?4, 1)",
            rusqlite::params![sig_id, account_id, sig_html, sig_text],
        )
        .map_err(|e| format!("insert signature: {e}"))?;

        accounts.push(Account {
            id: account_id,
            email: preset.email.to_string(),
            name: preset.name.to_string(),
            labels,
        });
    }

    Ok(accounts)
}
