mod google_contacts;
mod other_contacts;

use serde::Deserialize;

// Re-export public API
pub(crate) use google_contacts::sync_google_contacts;
pub(crate) use other_contacts::sync_google_other_contacts;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a Google contacts sync.
#[derive(Debug)]
pub(crate) struct SyncContactsResult {
    pub synced: usize,
    pub deleted: usize,
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleConnectionsResponse {
    pub connections: Option<Vec<Person>>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
    pub total_people: Option<i32>,
    pub total_items: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherContactsResponse {
    pub other_contacts: Option<Vec<Person>>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
    pub total_size: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub resource_name: Option<String>,
    pub etag: Option<String>,
    pub metadata: Option<PersonMetadata>,
    pub names: Option<Vec<Name>>,
    pub email_addresses: Option<Vec<EmailAddress>>,
    pub phone_numbers: Option<Vec<PhoneNumber>>,
    pub organizations: Option<Vec<Organization>>,
    pub photos: Option<Vec<Photo>>,
}

#[derive(Debug, Deserialize)]
pub struct PersonMetadata {
    pub deleted: Option<bool>,
    pub sources: Option<Vec<Source>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(rename = "type")]
    pub source_type: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Name {
    pub display_name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub email_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneNumber {
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub phone_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub name: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Photo {
    pub url: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

pub(crate) const PEOPLE_API_BASE: &str = "https://people.googleapis.com/v1";
pub(crate) const PAGE_SIZE: u32 = 1000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the first valid, lowercased email address from a Person.
pub(crate) fn extract_primary_email(person: &Person) -> Option<String> {
    person
        .email_addresses
        .as_ref()?
        .iter()
        .find_map(|e| e.value.as_deref().filter(|v| !v.is_empty()))
        .map(str::to_lowercase)
}

/// Extract display name, falling back to the email.
pub(crate) fn extract_display_name(person: &Person, fallback_email: &str) -> String {
    person
        .names
        .as_ref()
        .and_then(|names| names.first())
        .and_then(|n| n.display_name.as_deref())
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback_email)
        .to_string()
}

/// Extract avatar URL from the first photo.
pub(crate) fn extract_avatar_url(person: &Person) -> Option<String> {
    person
        .photos
        .as_ref()?
        .first()
        .and_then(|p| p.url.clone())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_person(
        resource_name: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        photo_url: Option<&str>,
        deleted: bool,
    ) -> Person {
        Person {
            resource_name: Some(resource_name.to_string()),
            etag: None,
            metadata: if deleted {
                Some(PersonMetadata {
                    deleted: Some(true),
                    sources: None,
                })
            } else {
                None
            },
            names: display_name.map(|n| {
                vec![Name {
                    display_name: Some(n.to_string()),
                    given_name: None,
                    family_name: None,
                }]
            }),
            email_addresses: email.map(|e| {
                vec![EmailAddress {
                    value: Some(e.to_string()),
                    email_type: Some("home".to_string()),
                }]
            }),
            phone_numbers: None,
            organizations: None,
            photos: photo_url.map(|u| vec![Photo { url: Some(u.to_string()) }]),
        }
    }

    #[test]
    fn test_deserialize_people_response() {
        let json = r#"{
            "connections": [
                {
                    "resourceName": "people/c12345",
                    "etag": "abc",
                    "names": [{"displayName": "Alice Smith"}],
                    "emailAddresses": [{"value": "alice@example.com", "type": "home"}],
                    "photos": [{"url": "https://photo.example.com/alice.jpg"}]
                }
            ],
            "nextSyncToken": "sync_token_abc",
            "totalPeople": 1,
            "totalItems": 1
        }"#;

        let response: PeopleConnectionsResponse = serde_json::from_str(json).expect("deserialize");
        assert!(response.connections.is_some());
        let connections = response.connections.as_ref().expect("connections");
        assert_eq!(connections.len(), 1);
        assert_eq!(
            connections[0].resource_name.as_deref(),
            Some("people/c12345")
        );
        assert_eq!(response.next_sync_token.as_deref(), Some("sync_token_abc"));
    }

    #[test]
    fn test_extract_primary_email() {
        let person = make_person("people/1", Some("Alice@Example.COM"), None, None, false);
        assert_eq!(
            extract_primary_email(&person),
            Some("alice@example.com".to_string())
        );
    }

    #[test]
    fn test_extract_primary_email_none() {
        let person = make_person("people/1", None, None, None, false);
        assert_eq!(extract_primary_email(&person), None);
    }

    #[test]
    fn test_extract_primary_email_empty() {
        let person = Person {
            resource_name: Some("people/1".to_string()),
            etag: None,
            metadata: None,
            names: None,
            email_addresses: Some(vec![EmailAddress {
                value: Some(String::new()),
                email_type: None,
            }]),
            phone_numbers: None,
            organizations: None,
            photos: None,
        };
        assert_eq!(extract_primary_email(&person), None);
    }

    #[test]
    fn test_extract_display_name_with_name() {
        let person = make_person("people/1", Some("a@b.com"), Some("Alice"), None, false);
        assert_eq!(extract_display_name(&person, "a@b.com"), "Alice");
    }

    #[test]
    fn test_extract_display_name_fallback() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        assert_eq!(extract_display_name(&person, "a@b.com"), "a@b.com");
    }

    #[test]
    fn test_extract_avatar_url() {
        let person = make_person(
            "people/1",
            Some("a@b.com"),
            None,
            Some("https://photo.example.com/a.jpg"),
            false,
        );
        assert_eq!(
            extract_avatar_url(&person),
            Some("https://photo.example.com/a.jpg".to_string())
        );
    }

    #[test]
    fn test_extract_avatar_url_none() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        assert_eq!(extract_avatar_url(&person), None);
    }

    #[test]
    fn test_deleted_contact_metadata() {
        let person = make_person("people/1", Some("a@b.com"), None, None, true);
        let is_deleted = person
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(is_deleted);
    }

    #[test]
    fn test_not_deleted_contact() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        let is_deleted = person
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(!is_deleted);
    }

    #[test]
    fn test_deserialize_incremental_with_deleted() {
        let json = r#"{
            "connections": [
                {
                    "resourceName": "people/c999",
                    "metadata": {"deleted": true}
                },
                {
                    "resourceName": "people/c888",
                    "names": [{"displayName": "Bob"}],
                    "emailAddresses": [{"value": "bob@test.com"}]
                }
            ],
            "nextSyncToken": "new_token"
        }"#;

        let response: PeopleConnectionsResponse = serde_json::from_str(json).expect("deserialize");
        let connections = response.connections.expect("connections");

        // First entry is deleted
        let deleted = connections[0]
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(deleted);

        // Second entry is not deleted and has email
        let email = extract_primary_email(&connections[1]);
        assert_eq!(email, Some("bob@test.com".to_string()));
    }

    #[test]
    fn test_contacts_with_no_email_skipped() {
        let persons = vec![
            make_person("people/1", Some("valid@test.com"), Some("Valid"), None, false),
            make_person("people/2", None, Some("No Email"), None, false),
            make_person("people/3", Some("also@valid.com"), None, None, false),
        ];

        let with_email: Vec<_> = persons
            .iter()
            .filter(|p| extract_primary_email(p).is_some())
            .collect();
        assert_eq!(with_email.len(), 2);
    }

    #[test]
    fn test_deserialize_other_contacts_response() {
        let json = r#"{
            "otherContacts": [
                {
                    "resourceName": "otherContacts/c12345",
                    "etag": "abc",
                    "names": [{"displayName": "Alice Smith"}],
                    "emailAddresses": [{"value": "alice@example.com", "type": "home"}],
                    "photos": [{"url": "https://photo.example.com/alice.jpg"}]
                }
            ],
            "nextSyncToken": "other_sync_token_abc",
            "totalSize": 1
        }"#;

        let response: OtherContactsResponse = serde_json::from_str(json).expect("deserialize");
        assert!(response.other_contacts.is_some());
        let contacts = response.other_contacts.as_ref().expect("otherContacts");
        assert_eq!(contacts.len(), 1);
        assert_eq!(
            contacts[0].resource_name.as_deref(),
            Some("otherContacts/c12345")
        );
        assert_eq!(
            response.next_sync_token.as_deref(),
            Some("other_sync_token_abc")
        );
        assert_eq!(response.total_size, Some(1));
    }

    #[test]
    fn test_other_contacts_email_extraction() {
        let person = make_person(
            "otherContacts/c999",
            Some("Other@Example.COM"),
            Some("Other User"),
            None,
            false,
        );
        assert_eq!(
            extract_primary_email(&person),
            Some("other@example.com".to_string())
        );
        assert_eq!(extract_display_name(&person, "other@example.com"), "Other User");
    }

    #[test]
    fn test_deserialize_other_contacts_with_deleted() {
        let json = r#"{
            "otherContacts": [
                {
                    "resourceName": "otherContacts/c999",
                    "metadata": {"deleted": true}
                },
                {
                    "resourceName": "otherContacts/c888",
                    "names": [{"displayName": "Bob"}],
                    "emailAddresses": [{"value": "bob@test.com"}]
                }
            ],
            "nextSyncToken": "new_other_token"
        }"#;

        let response: OtherContactsResponse = serde_json::from_str(json).expect("deserialize");
        let contacts = response.other_contacts.expect("otherContacts");

        // First entry is deleted
        let deleted = contacts[0]
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(deleted);

        // Second entry is not deleted and has email
        let email = extract_primary_email(&contacts[1]);
        assert_eq!(email, Some("bob@test.com".to_string()));
    }

    #[test]
    fn test_other_contacts_empty_response() {
        let json = r#"{
            "nextSyncToken": "empty_token",
            "totalSize": 0
        }"#;

        let response: OtherContactsResponse = serde_json::from_str(json).expect("deserialize");
        assert!(response.other_contacts.is_none());
        assert_eq!(response.total_size, Some(0));
    }
}
