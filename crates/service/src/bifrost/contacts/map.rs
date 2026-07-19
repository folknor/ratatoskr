use bifrost_types::ContactCard;
use db::db::queries_extra::ContactWriteRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactProjection {
    PackSecondEmail,
    FanOutPerEmail,
}

pub fn contact_write_rows(
    card: &ContactCard,
    account_id: &str,
    source: &str,
    projection: ContactProjection,
) -> Vec<ContactWriteRow> {
    let emails: Vec<String> = card
        .emails
        .iter()
        .map(|email| email.value.trim().to_lowercase())
        .filter(|email| !email.is_empty())
        .collect();
    let Some(first) = emails.first() else {
        return Vec::new();
    };
    let phone = card.phones.first().map(|phone| phone.value.clone());
    let company = card
        .organizations
        .first()
        .map(|organization| organization.name.clone());
    let make_row = |email: &String, email2: Option<String>| ContactWriteRow {
        id: format!("{source}-{account_id}-{email}"),
        email: email.clone(),
        display_name: card.display_name.clone().or_else(|| Some(email.clone())),
        email2,
        phone: phone.clone(),
        company: company.clone(),
        notes: card.notes.clone(),
        avatar_url: card.photo_url.clone(),
        source: source.to_string(),
        account_id: account_id.to_string(),
        server_id: Some(card.native_id.clone()),
    };
    match projection {
        ContactProjection::PackSecondEmail => vec![make_row(first, emails.get(1).cloned())],
        ContactProjection::FanOutPerEmail => {
            emails.iter().map(|email| make_row(email, None)).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_types::{
        ContactCorpus, ContactEmail, ContactId, ContactOrganization, ContactPhone,
        ContactProvenance, ProtocolKind,
    };

    fn card(emails: &[&str]) -> ContactCard {
        ContactCard {
            id: ContactId("native".into()),
            address_book_id: None,
            native_id: "native".into(),
            etag: None,
            provenance: ContactProvenance {
                provider: ProtocolKind::Jmap,
                native: "native".into(),
                address_book_native: None,
            },
            corpus: ContactCorpus::Main,
            display_name: None,
            emails: emails
                .iter()
                .map(|value| ContactEmail {
                    value: (*value).into(),
                    kind: None,
                    is_primary: false,
                })
                .collect(),
            phones: vec![],
            organizations: vec![],
            addresses: vec![],
            notes: None,
            photo_url: None,
            photo: None,
        }
    }
    #[test]
    fn contact_write_rows_projects_identity_and_fanout() {
        assert!(
            contact_write_rows(&card(&[]), "a", "jmap", ContactProjection::PackSecondEmail)
                .is_empty()
        );
        let packed = contact_write_rows(
            &card(&["A@Example.test", "b@example.test"]),
            "a",
            "jmap",
            ContactProjection::PackSecondEmail,
        );
        assert_eq!(packed.len(), 1);
        assert_eq!(packed[0].id, "jmap-a-a@example.test");
        assert_eq!(packed[0].email2.as_deref(), Some("b@example.test"));
        assert_eq!(packed[0].display_name.as_deref(), Some("a@example.test"));
        assert_eq!(packed[0].server_id.as_deref(), Some("native"));
        let fanned = contact_write_rows(
            &card(&["A@Example.test", "b@example.test"]),
            "a",
            "graph",
            ContactProjection::FanOutPerEmail,
        );
        assert_eq!(fanned.len(), 2);
        assert!(fanned.iter().all(|row| row.email2.is_none()));
    }

    #[test]
    fn contact_write_rows_maps_the_first_local_fields() {
        let mut source = card(&["contact@example.test"]);
        source.display_name = Some("Contact Name".into());
        source.phones = vec![ContactPhone {
            value: "+47 123 45 678".into(),
            kind: None,
            is_primary: true,
        }];
        source.organizations = vec![ContactOrganization {
            name: "Ratatoskr AS".into(),
            title: None,
        }];
        source.notes = Some("A note".into());
        source.photo_url = Some("https://example.test/photo".into());
        let rows = contact_write_rows(
            &source,
            "account",
            "carddav",
            ContactProjection::PackSecondEmail,
        );
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, "carddav-account-contact@example.test");
        assert_eq!(row.display_name.as_deref(), Some("Contact Name"));
        assert_eq!(row.phone.as_deref(), Some("+47 123 45 678"));
        assert_eq!(row.company.as_deref(), Some("Ratatoskr AS"));
        assert_eq!(row.notes.as_deref(), Some("A note"));
        assert_eq!(
            row.avatar_url.as_deref(),
            Some("https://example.test/photo")
        );
    }
}
