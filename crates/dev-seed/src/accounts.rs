use rand::Rng;
use rusqlite::Connection;

#[derive(Clone, Copy)]
pub struct UserLabel {
    pub name: &'static str,
    pub color_bg: &'static str,
    pub color_fg: &'static str,
}

#[derive(Clone, Copy)]
pub struct CategoryLabels {
    pub work: Option<&'static str>,
    pub personal: Option<&'static str>,
    pub newsletters: Option<&'static str>,
    pub receipts: Option<&'static str>,
}

pub struct AccountPreset {
    pub email: &'static str,
    pub display_name: &'static str,
    pub account_name: &'static str,
    pub provider: &'static str,
    pub host: &'static str,
    pub color: &'static str,
    pub user_labels: &'static [UserLabel],
    pub category_labels: CategoryLabels,
}

static PERSONAL_LABELS: &[UserLabel] = &[
    UserLabel {
        name: "Family",
        color_bg: "#0b8043",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Personal",
        color_bg: "#4285f4",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Travel",
        color_bg: "#db4437",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Receipts",
        color_bg: "#00acc1",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Newsletters",
        color_bg: "#ab47bc",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Photos",
        color_bg: "#f4b400",
        color_fg: "#000000",
    },
];

static WORK_LABELS: &[UserLabel] = &[
    UserLabel {
        name: "Projects",
        color_bg: "#ff7043",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Customers",
        color_bg: "#4285f4",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Waiting",
        color_bg: "#8d6e63",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Receipts",
        color_bg: "#00acc1",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Newsletters",
        color_bg: "#ab47bc",
        color_fg: "#ffffff",
    },
];

static OFFICE_LABELS: &[UserLabel] = &[
    UserLabel {
        name: "Clients",
        color_bg: "#4285f4",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Personal",
        color_bg: "#0b8043",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Travel",
        color_bg: "#db4437",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Receipts",
        color_bg: "#00acc1",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Alerts",
        color_bg: "#f4b400",
        color_fg: "#000000",
    },
];

static FASTMAIL_LABELS: &[UserLabel] = &[
    UserLabel {
        name: "Projects",
        color_bg: "#ff7043",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Personal",
        color_bg: "#0b8043",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Newsletters",
        color_bg: "#ab47bc",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Receipts",
        color_bg: "#00acc1",
        color_fg: "#ffffff",
    },
    UserLabel {
        name: "Reference",
        color_bg: "#8d6e63",
        color_fg: "#ffffff",
    },
];

pub static ACCOUNT_PRESETS: &[AccountPreset] = &[
    AccountPreset {
        email: "alex.morgan@gmail.com",
        display_name: "Alex Morgan",
        account_name: "Personal",
        provider: "gmail_api",
        host: "imap.gmail.com",
        color: "#4285f4",
        user_labels: PERSONAL_LABELS,
        category_labels: CategoryLabels {
            work: None,
            personal: Some("Personal"),
            newsletters: Some("Newsletters"),
            receipts: Some("Receipts"),
        },
    },
    AccountPreset {
        email: "alex.morgan@company.io",
        display_name: "Alex Morgan",
        account_name: "Work",
        provider: "imap",
        host: "mail.company.io",
        color: "#ea4335",
        user_labels: WORK_LABELS,
        category_labels: CategoryLabels {
            work: Some("Projects"),
            personal: None,
            newsletters: Some("Newsletters"),
            receipts: Some("Receipts"),
        },
    },
    AccountPreset {
        email: "a.morgan@outlook.com",
        display_name: "Alex Morgan",
        account_name: "Office",
        provider: "graph",
        host: "outlook.office365.com",
        color: "#fbbc04",
        user_labels: OFFICE_LABELS,
        category_labels: CategoryLabels {
            work: Some("Clients"),
            personal: Some("Personal"),
            newsletters: None,
            receipts: Some("Receipts"),
        },
    },
    AccountPreset {
        email: "alex@fastmail.com",
        display_name: "Alex Morgan",
        account_name: "Fastmail",
        provider: "jmap",
        host: "jmap.fastmail.com",
        color: "#34a853",
        user_labels: FASTMAIL_LABELS,
        category_labels: CategoryLabels {
            work: Some("Projects"),
            personal: Some("Personal"),
            newsletters: Some("Newsletters"),
            receipts: Some("Receipts"),
        },
    },
];

struct SystemLabel {
    id: &'static str,
    name: &'static str,
    special: &'static str,
    sort: i32,
}

static SYSTEM_LABELS: &[SystemLabel] = &[
    SystemLabel {
        id: "INBOX",
        name: "INBOX",
        special: "inbox",
        sort: 0,
    },
    SystemLabel {
        id: "SENT",
        name: "Sent",
        special: "sent",
        sort: 1,
    },
    SystemLabel {
        id: "DRAFT",
        name: "Drafts",
        special: "drafts",
        sort: 2,
    },
    SystemLabel {
        id: "TRASH",
        name: "Trash",
        special: "trash",
        sort: 3,
    },
    SystemLabel {
        id: "archive",
        name: "Archive",
        special: "archive",
        sort: 4,
    },
    SystemLabel {
        id: "SPAM",
        name: "Spam",
        special: "junk",
        sort: 5,
    },
    SystemLabel {
        id: "all-mail",
        name: "All Mail",
        special: "all",
        sort: 6,
    },
];

fn seeded_user_label_id(provider: &str, name: &str) -> String {
    match provider {
        "gmail_api" => name.to_string(),
        "graph" => format!("cat:{name}"),
        "imap" | "jmap" => format!("kw:{name}"),
        _ => name.to_string(),
    }
}

/// Inserted account info needed by later stages.
pub struct Account {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub account_name: String,
    /// Map from label name -> label id
    pub labels: Vec<(String, String)>,
    pub category_labels: CategoryLabels,
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
                     'oauth2', ?6, ?7, ?8, 1, 1)",
            rusqlite::params![
                account_id,
                preset.email,
                preset.display_name,
                preset.provider,
                preset.host,
                preset.color,
                preset.account_name,
                i32::try_from(accounts.len()).unwrap_or(0),
            ],
        )
        .map_err(|e| format!("insert account: {e}"))?;

        let mut labels = Vec::new();

        for sl in SYSTEM_LABELS {
            conn.execute(
                "INSERT INTO labels (id, account_id, name, type, visible, sort_order, \
                 imap_special_use, imap_folder_path, label_kind) \
                 VALUES (?1, ?2, ?3, 'system', 1, ?4, ?5, ?3, 'container')",
                rusqlite::params![sl.id, account_id, sl.name, sl.sort, sl.special],
            )
            .map_err(|e| format!("insert system label: {e}"))?;
            labels.push((sl.name.to_string(), sl.id.to_string()));
        }

        for (i, ul) in preset.user_labels.iter().enumerate() {
            let label_id = seeded_user_label_id(preset.provider, ul.name);
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
                    i32::try_from(SYSTEM_LABELS.len() + i).unwrap_or(0),
                ],
            )
            .map_err(|e| format!("insert user label: {e}"))?;
            labels.push((ul.name.to_string(), label_id));
        }

        let sig_id = crate::next_uuid(rng);
        let sig_html = format!(
            "<p>Best regards,<br><strong>{}</strong><br>{}</p>",
            preset.display_name, preset.email
        );
        let sig_text = format!(
            "Best regards,\n{}\n{}",
            preset.display_name, preset.email
        );
        conn.execute(
            "INSERT INTO signatures (id, account_id, name, body_html, body_text, is_default) \
             VALUES (?1, ?2, 'Default', ?3, ?4, 1)",
            rusqlite::params![sig_id, account_id, sig_html, sig_text],
        )
        .map_err(|e| format!("insert signature: {e}"))?;

        accounts.push(Account {
            id: account_id,
            email: preset.email.to_string(),
            display_name: preset.display_name.to_string(),
            account_name: preset.account_name.to_string(),
            labels,
            category_labels: preset.category_labels,
        });
    }

    Ok(accounts)
}
