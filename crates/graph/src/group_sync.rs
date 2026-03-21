use std::collections::HashSet;

use rusqlite::params;
use serde::Deserialize;

use ratatoskr_db::db::DbState;

use super::client::GraphClient;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ResolvedGroupMember {
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GroupResolutionResult {
    pub members: Vec<ResolvedGroupMember>,
    pub total_count: usize,
    pub resolved_count: usize,
}

#[derive(Debug, Clone)]
pub struct ExchangeGroup {
    pub id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub group_type: ExchangeGroupType,
}

#[derive(Debug, Clone)]
pub enum ExchangeGroupType {
    M365Group,
    DistributionList,
    MailEnabledSecurityGroup,
}

impl ExchangeGroupType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::M365Group => "m365",
            Self::DistributionList => "distribution_list",
            Self::MailEnabledSecurityGroup => "mail_security",
        }
    }
}

// ---------------------------------------------------------------------------
// Graph API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct GraphGroupsResponse {
    pub value: Vec<GraphGroup>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGroup {
    pub id: String,
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub group_types: Option<Vec<String>>,
    pub mail_enabled: Option<bool>,
    pub security_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct GraphMembersResponse {
    pub value: Vec<GraphGroupMember>,
    #[serde(rename = "@odata.nextLink")]
    pub next_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphGroupMember {
    pub display_name: Option<String>,
    pub mail: Option<String>,
    pub user_principal_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Resolve all members of a group via transitive membership expansion.
///
/// Uses `GET /groups/{id}/transitiveMembers/microsoft.graph.user` which handles
/// recursive expansion and cycle detection server-side.
pub async fn resolve_group_members(
    client: &GraphClient,
    db: &DbState,
    group_id: &str,
) -> Result<GroupResolutionResult, String> {
    let enc_id = urlencoding::encode(group_id);
    let initial_url = format!(
        "/groups/{enc_id}/transitiveMembers/microsoft.graph.user\
         ?$select=displayName,mail,userPrincipalName&$top=999"
    );

    let mut all_members = Vec::new();
    let mut next_link: Option<String> = None;

    loop {
        let page: GraphMembersResponse = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        for member in &page.value {
            if let Some(resolved) = extract_member_email(member) {
                all_members.push(resolved);
            }
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    let total_count = all_members.len();
    Ok(GroupResolutionResult {
        resolved_count: total_count,
        total_count,
        members: all_members,
    })
}

/// Fetch all mail-enabled groups the user belongs to.
///
/// Uses `GET /me/memberOf/microsoft.graph.group` (or shared mailbox equivalent)
/// and filters to only mail-enabled groups.
pub async fn fetch_user_groups(
    client: &GraphClient,
    db: &DbState,
) -> Result<Vec<ExchangeGroup>, String> {
    let prefix = client.api_path_prefix();
    let initial_url = format!(
        "{prefix}/memberOf/microsoft.graph.group\
         ?$filter=mailEnabled eq true\
         &$select=id,displayName,mail,groupTypes,mailEnabled,securityEnabled\
         &$top=999"
    );

    let mut all_groups = Vec::new();
    let mut next_link: Option<String> = None;

    loop {
        let page: GraphGroupsResponse = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        for group in &page.value {
            if let Some(classified) = classify_group(group) {
                all_groups.push(classified);
            }
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    Ok(all_groups)
}

/// Main sync entry point: fetch groups, resolve members, persist to DB.
///
/// Returns the count of synced groups.
pub(crate) async fn sync_exchange_groups(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
) -> Result<usize, String> {
    log::debug!("[Graph] Syncing Exchange groups for account {account_id}");
    let groups = fetch_user_groups(client, db).await?;
    if groups.is_empty() {
        // Prune any previously synced groups for this account
        prune_all_account_groups(db, account_id).await?;
        return Ok(0);
    }

    let group_count = groups.len();
    let mut seen_server_ids = HashSet::new();

    for group in &groups {
        seen_server_ids.insert(group.id.clone());

        // Upsert the group
        let g = group.clone();
        let aid = account_id.to_string();
        db.with_conn(move |conn| upsert_group(conn, &aid, &g))
            .await?;

        // Resolve and persist members
        match resolve_group_members(client, db, &group.id).await {
            Ok(result) => {
                let gid = group.id.clone();
                let aid = account_id.to_string();
                let members = result.members;
                db.with_conn(move |conn| {
                    persist_group_members(conn, &aid, &gid, &members)
                })
                .await?;
                log::info!(
                    "Resolved {} members for group '{}' ({})",
                    result.resolved_count,
                    group.display_name,
                    group.id
                );
            }
            Err(e) => {
                log::warn!(
                    "Failed to resolve members for group '{}' ({}): {e}",
                    group.display_name,
                    group.id
                );
            }
        }
    }

    // Prune stale groups that no longer exist on the server
    let aid = account_id.to_string();
    db.with_conn(move |conn| prune_stale_groups(conn, &aid, &seen_server_ids))
        .await?;

    log::info!("Exchange group sync complete: {group_count} groups for account {account_id}");
    Ok(group_count)
}

// ---------------------------------------------------------------------------
// Classification helpers
// ---------------------------------------------------------------------------

/// Classify a Graph group into our type system, returning None for
/// security-only groups that should be excluded.
fn classify_group(group: &GraphGroup) -> Option<ExchangeGroup> {
    let mail_enabled = group.mail_enabled.unwrap_or(false);
    if !mail_enabled {
        return None;
    }

    let group_types = group.group_types.as_deref().unwrap_or(&[]);
    let security_enabled = group.security_enabled.unwrap_or(false);

    let group_type = if group_types.iter().any(|t| t == "Unified") {
        ExchangeGroupType::M365Group
    } else if security_enabled {
        ExchangeGroupType::MailEnabledSecurityGroup
    } else {
        ExchangeGroupType::DistributionList
    };

    Some(ExchangeGroup {
        id: group.id.clone(),
        display_name: group
            .display_name
            .clone()
            .unwrap_or_else(|| "Unnamed Group".to_string()),
        email: group.mail.clone(),
        group_type,
    })
}

/// Extract email from a group member, preferring `mail` over `userPrincipalName`.
fn extract_member_email(member: &GraphGroupMember) -> Option<ResolvedGroupMember> {
    let email = member
        .mail
        .as_deref()
        .filter(|m| !m.is_empty())
        .or_else(|| member.user_principal_name.as_deref().filter(|u| !u.is_empty()))?;

    Some(ResolvedGroupMember {
        email: email.to_lowercase(),
        display_name: member.display_name.clone(),
    })
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

fn upsert_group(
    conn: &rusqlite::Connection,
    account_id: &str,
    group: &ExchangeGroup,
) -> Result<(), String> {
    let local_id = format!("exchange-{account_id}-{}", group.id);

    conn.execute(
        "INSERT INTO contact_groups (id, name, source, account_id, server_id, email, group_type) \
         VALUES (?1, ?2, 'exchange', ?3, ?4, ?5, ?6) \
         ON CONFLICT(id) DO UPDATE SET \
           name = excluded.name, \
           email = excluded.email, \
           group_type = excluded.group_type, \
           updated_at = unixepoch()",
        params![
            local_id,
            group.display_name,
            account_id,
            group.id,
            group.email,
            group.group_type.as_str(),
        ],
    )
    .map_err(|e| format!("upsert group: {e}"))?;

    Ok(())
}

fn persist_group_members(
    conn: &rusqlite::Connection,
    account_id: &str,
    server_group_id: &str,
    members: &[ResolvedGroupMember],
) -> Result<(), String> {
    let local_id = format!("exchange-{account_id}-{server_group_id}");

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin tx: {e}"))?;

    // Clear existing members for this group and repopulate
    tx.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![local_id],
    )
    .map_err(|e| format!("clear group members: {e}"))?;

    for member in members {
        tx.execute(
            "INSERT OR IGNORE INTO contact_group_members (group_id, member_type, member_value) \
             VALUES (?1, 'email', ?2)",
            params![local_id, member.email],
        )
        .map_err(|e| format!("insert group member: {e}"))?;
    }

    tx.commit().map_err(|e| format!("commit tx: {e}"))?;
    Ok(())
}

fn prune_stale_groups(
    conn: &rusqlite::Connection,
    account_id: &str,
    seen_server_ids: &HashSet<String>,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, server_id FROM contact_groups \
             WHERE account_id = ?1 AND source = 'exchange'",
        )
        .map_err(|e| format!("prepare stale group lookup: {e}"))?;

    let stale: Vec<String> = stmt
        .query_map(params![account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query stale groups: {e}"))?
        .filter_map(Result::ok)
        .filter(|(_, server_id)| !seen_server_ids.contains(server_id))
        .map(|(local_id, _)| local_id)
        .collect();

    drop(stmt);

    for local_id in &stale {
        // Members cascade-deleted via FK
        conn.execute(
            "DELETE FROM contact_groups WHERE id = ?1",
            params![local_id],
        )
        .map_err(|e| format!("delete stale group: {e}"))?;
    }

    if !stale.is_empty() {
        log::info!(
            "Pruned {} stale Exchange groups for account {account_id}",
            stale.len()
        );
    }

    Ok(())
}

async fn prune_all_account_groups(db: &DbState, account_id: &str) -> Result<(), String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM contact_groups WHERE account_id = ?1 AND source = 'exchange'",
            params![aid],
        )
        .map_err(|e| format!("prune all account groups: {e}"))?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_m365_group() {
        let group = GraphGroup {
            id: "g1".to_string(),
            display_name: Some("Engineering".to_string()),
            mail: Some("eng@contoso.com".to_string()),
            group_types: Some(vec!["Unified".to_string()]),
            mail_enabled: Some(true),
            security_enabled: Some(false),
        };
        let result = classify_group(&group).expect("should classify");
        assert_eq!(result.display_name, "Engineering");
        assert!(matches!(result.group_type, ExchangeGroupType::M365Group));
    }

    #[test]
    fn classify_distribution_list() {
        let group = GraphGroup {
            id: "g2".to_string(),
            display_name: Some("All Staff".to_string()),
            mail: Some("allstaff@contoso.com".to_string()),
            group_types: Some(vec![]),
            mail_enabled: Some(true),
            security_enabled: Some(false),
        };
        let result = classify_group(&group).expect("should classify");
        assert!(matches!(
            result.group_type,
            ExchangeGroupType::DistributionList
        ));
    }

    #[test]
    fn classify_mail_enabled_security_group() {
        let group = GraphGroup {
            id: "g3".to_string(),
            display_name: Some("Security Team".to_string()),
            mail: Some("sec@contoso.com".to_string()),
            group_types: Some(vec![]),
            mail_enabled: Some(true),
            security_enabled: Some(true),
        };
        let result = classify_group(&group).expect("should classify");
        assert!(matches!(
            result.group_type,
            ExchangeGroupType::MailEnabledSecurityGroup
        ));
    }

    #[test]
    fn exclude_security_only_group() {
        let group = GraphGroup {
            id: "g4".to_string(),
            display_name: Some("Admins".to_string()),
            mail: None,
            group_types: Some(vec![]),
            mail_enabled: Some(false),
            security_enabled: Some(true),
        };
        assert!(classify_group(&group).is_none());
    }

    #[test]
    fn extract_email_prefers_mail() {
        let member = GraphGroupMember {
            display_name: Some("Alice".to_string()),
            mail: Some("Alice@Contoso.com".to_string()),
            user_principal_name: Some("alice@contoso.onmicrosoft.com".to_string()),
        };
        let result = extract_member_email(&member).expect("should extract");
        assert_eq!(result.email, "alice@contoso.com");
        assert_eq!(result.display_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn extract_email_falls_back_to_upn() {
        let member = GraphGroupMember {
            display_name: Some("Bob".to_string()),
            mail: None,
            user_principal_name: Some("Bob@Contoso.com".to_string()),
        };
        let result = extract_member_email(&member).expect("should extract");
        assert_eq!(result.email, "bob@contoso.com");
    }

    #[test]
    fn extract_email_none_when_both_missing() {
        let member = GraphGroupMember {
            display_name: Some("Ghost".to_string()),
            mail: None,
            user_principal_name: None,
        };
        assert!(extract_member_email(&member).is_none());
    }

    #[test]
    fn extract_email_skips_empty_mail() {
        let member = GraphGroupMember {
            display_name: None,
            mail: Some("".to_string()),
            user_principal_name: Some("user@contoso.com".to_string()),
        };
        let result = extract_member_email(&member).expect("should fall back to UPN");
        assert_eq!(result.email, "user@contoso.com");
    }

    #[test]
    fn deserialize_graph_group() {
        let json = r#"{
            "id": "abc-123",
            "displayName": "Test Group",
            "mail": "test@example.com",
            "groupTypes": ["Unified"],
            "mailEnabled": true,
            "securityEnabled": false
        }"#;
        let group: GraphGroup = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(group.id, "abc-123");
        assert_eq!(group.display_name.as_deref(), Some("Test Group"));
        assert_eq!(group.mail.as_deref(), Some("test@example.com"));
        assert!(group.group_types.as_ref().map_or(false, |t| t.contains(&"Unified".to_string())));
        assert_eq!(group.mail_enabled, Some(true));
        assert_eq!(group.security_enabled, Some(false));
    }

    #[test]
    fn deserialize_graph_member() {
        let json = r#"{
            "displayName": "Jane Doe",
            "mail": "jane@example.com",
            "userPrincipalName": "jane@example.onmicrosoft.com"
        }"#;
        let member: GraphGroupMember = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(member.display_name.as_deref(), Some("Jane Doe"));
        assert_eq!(member.mail.as_deref(), Some("jane@example.com"));
    }

    #[test]
    fn classify_group_with_no_group_types() {
        // groupTypes can be null in the API response
        let group = GraphGroup {
            id: "g5".to_string(),
            display_name: Some("Legacy DL".to_string()),
            mail: Some("legacydl@contoso.com".to_string()),
            group_types: None,
            mail_enabled: Some(true),
            security_enabled: Some(false),
        };
        let result = classify_group(&group).expect("should classify");
        assert!(matches!(
            result.group_type,
            ExchangeGroupType::DistributionList
        ));
    }

    #[test]
    fn group_type_as_str() {
        assert_eq!(ExchangeGroupType::M365Group.as_str(), "m365");
        assert_eq!(
            ExchangeGroupType::DistributionList.as_str(),
            "distribution_list"
        );
        assert_eq!(
            ExchangeGroupType::MailEnabledSecurityGroup.as_str(),
            "mail_security"
        );
    }
}
