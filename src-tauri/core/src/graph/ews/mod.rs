mod client;
mod parsers;
mod xml_helpers;

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

// Re-export parsers and xml_helpers for internal use within the ews module
use self::parsers::*;
use self::xml_helpers::*;

const EWS_URL: &str = "https://outlook.office365.com/EWS/Exchange.asmx";

// ── Client ──────────────────────────────────────────────────

pub struct EwsClient {
    http: reqwest::Client,
    ews_url: String,
}

// ── Types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EwsFolder {
    pub folder_id: String,
    pub display_name: String,
    pub folder_class: Option<String>,
    pub total_count: u32,
    pub unread_count: u32,
    pub child_folder_count: u32,
    pub effective_rights: EwsEffectiveRights,
    pub replica_list: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct EwsEffectiveRights {
    pub create_associated: bool,
    pub create_contents: bool,
    pub create_hierarchy: bool,
    pub delete: bool,
    pub modify: bool,
    pub read: bool,
}

#[derive(Debug, Clone)]
pub struct EwsItem {
    pub item_id: String,
    pub change_key: Option<String>,
    pub subject: Option<String>,
    pub sender_email: Option<String>,
    pub sender_name: Option<String>,
    pub received_at: Option<String>,
    pub body_preview: Option<String>,
    pub body_html: Option<String>,
    pub is_read: bool,
    pub item_class: String,
    pub to_recipients: Vec<EwsRecipient>,
    pub cc_recipients: Vec<EwsRecipient>,
}

#[derive(Debug, Clone)]
pub struct EwsRecipient {
    pub email: String,
    pub name: Option<String>,
}

pub struct FindItemsResult {
    pub items: Vec<EwsItem>,
    pub total_count: u32,
    pub includes_last: bool,
}

pub struct EwsHeaders {
    pub anchor_mailbox: Option<String>,
    pub public_folder_mailbox: Option<String>,
}

// ── PR_REPLICA_LIST decoding ────────────────────────────────

/// Decode `PR_REPLICA_LIST` (0x6698) binary data into GUID strings.
///
/// The binary format is a sequence of null-terminated ASCII hex GUID strings.
/// Each GUID is in the form `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}\0`.
pub fn decode_replica_list(base64_data: &str) -> Result<Vec<String>, String> {
    let bytes = BASE64
        .decode(base64_data)
        .map_err(|e| format!("Failed to decode base64 replica list: {e}"))?;

    let mut guids = Vec::new();
    let mut start = 0;

    for (i, &b) in bytes.iter().enumerate() {
        if b == 0 {
            if i > start
                && let Ok(s) = std::str::from_utf8(&bytes[start..i])
            {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    guids.push(trimmed.to_string());
                }
            }
            start = i + 1;
        }
    }

    Ok(guids)
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soap_envelope_wraps_body() {
        let body = r#"<m:FindFolder Traversal="Shallow"/>"#;
        let envelope = build_soap_envelope(body);
        assert!(envelope.contains("soap:Envelope"));
        assert!(envelope.contains("soap:Header"));
        assert!(envelope.contains("RequestServerVersion"));
        assert!(envelope.contains("Exchange2016"));
        assert!(envelope.contains("soap:Body"));
        assert!(envelope.contains(body));
    }

    #[test]
    fn soap_fault_detected() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <soap:Fault>
      <faultcode>soap:Client</faultcode>
      <faultstring>The request failed schema validation.</faultstring>
    </soap:Fault>
  </soap:Body>
</soap:Envelope>"#;

        let result = check_soap_fault(xml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("schema validation"));
    }

    #[test]
    fn no_soap_fault_passes() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
  <soap:Body>
    <m:FindFolderResponse>
      <m:ResponseMessages/>
    </m:FindFolderResponse>
  </soap:Body>
</soap:Envelope>"#;

        assert!(check_soap_fault(xml).is_ok());
    }

    #[test]
    fn parse_find_folder() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="2" IncludesLastItemInRange="true">
            <t:Folders>
              <t:Folder>
                <t:FolderId Id="AAMkAGFk=" ChangeKey="AQAAAB"/>
                <t:DisplayName>Company Announcements</t:DisplayName>
                <t:TotalCount>42</t:TotalCount>
                <t:ChildFolderCount>3</t:ChildFolderCount>
                <t:UnreadCount>5</t:UnreadCount>
                <t:FolderClass>IPF.Note</t:FolderClass>
                <t:EffectiveRights>
                  <t:CreateAssociated>false</t:CreateAssociated>
                  <t:CreateContents>true</t:CreateContents>
                  <t:CreateHierarchy>false</t:CreateHierarchy>
                  <t:Delete>false</t:Delete>
                  <t:Modify>false</t:Modify>
                  <t:Read>true</t:Read>
                </t:EffectiveRights>
              </t:Folder>
              <t:Folder>
                <t:FolderId Id="BBNkAHJk=" ChangeKey="BQAAAC"/>
                <t:DisplayName>IT Helpdesk</t:DisplayName>
                <t:TotalCount>128</t:TotalCount>
                <t:ChildFolderCount>0</t:ChildFolderCount>
                <t:UnreadCount>12</t:UnreadCount>
                <t:FolderClass>IPF.Note</t:FolderClass>
                <t:EffectiveRights>
                  <t:CreateAssociated>true</t:CreateAssociated>
                  <t:CreateContents>true</t:CreateContents>
                  <t:CreateHierarchy>true</t:CreateHierarchy>
                  <t:Delete>true</t:Delete>
                  <t:Modify>true</t:Modify>
                  <t:Read>true</t:Read>
                </t:EffectiveRights>
              </t:Folder>
            </t:Folders>
          </m:RootFolder>
        </m:FindFolderResponseMessage>
      </m:ResponseMessages>
    </m:FindFolderResponse>
  </s:Body>
</s:Envelope>"#;

        let folders = parse_find_folder_response(xml).expect("parse should succeed");
        assert_eq!(folders.len(), 2);

        assert_eq!(folders[0].folder_id, "AAMkAGFk=");
        assert_eq!(folders[0].display_name, "Company Announcements");
        assert_eq!(folders[0].total_count, 42);
        assert_eq!(folders[0].unread_count, 5);
        assert_eq!(folders[0].child_folder_count, 3);
        assert_eq!(folders[0].folder_class.as_deref(), Some("IPF.Note"));
        assert!(folders[0].effective_rights.read);
        assert!(folders[0].effective_rights.create_contents);
        assert!(!folders[0].effective_rights.delete);
        assert!(!folders[0].effective_rights.modify);

        assert_eq!(folders[1].folder_id, "BBNkAHJk=");
        assert_eq!(folders[1].display_name, "IT Helpdesk");
        assert_eq!(folders[1].total_count, 128);
        assert!(folders[1].effective_rights.delete);
        assert!(folders[1].effective_rights.modify);
    }

    #[test]
    fn parse_find_folder_empty() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="0" IncludesLastItemInRange="true">
            <t:Folders/>
          </m:RootFolder>
        </m:FindFolderResponseMessage>
      </m:ResponseMessages>
    </m:FindFolderResponse>
  </s:Body>
</s:Envelope>"#;

        let folders = parse_find_folder_response(xml).expect("parse should succeed");
        assert!(folders.is_empty());
    }

    #[test]
    fn parse_find_items() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                        xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="150" IncludesLastItemInRange="false">
            <t:Items>
              <t:Message>
                <t:ItemId Id="AAMkItem1=" ChangeKey="CK1"/>
                <t:Subject>Q1 Results</t:Subject>
                <t:DateTimeReceived>2026-03-01T10:30:00Z</t:DateTimeReceived>
                <t:From>
                  <t:Mailbox>
                    <t:Name>Jane Doe</t:Name>
                    <t:EmailAddress>jane@contoso.com</t:EmailAddress>
                  </t:Mailbox>
                </t:From>
                <t:IsRead>true</t:IsRead>
                <t:Preview>Here are the Q1 financial results...</t:Preview>
                <t:ItemClass>IPM.Note</t:ItemClass>
              </t:Message>
              <t:Message>
                <t:ItemId Id="AAMkItem2=" ChangeKey="CK2"/>
                <t:Subject>Office Move Update</t:Subject>
                <t:DateTimeReceived>2026-02-28T14:15:00Z</t:DateTimeReceived>
                <t:From>
                  <t:Mailbox>
                    <t:Name>Facilities</t:Name>
                    <t:EmailAddress>facilities@contoso.com</t:EmailAddress>
                  </t:Mailbox>
                </t:From>
                <t:IsRead>false</t:IsRead>
                <t:Preview>The office move has been rescheduled...</t:Preview>
                <t:ItemClass>IPM.Note</t:ItemClass>
              </t:Message>
            </t:Items>
          </m:RootFolder>
        </m:FindItemResponseMessage>
      </m:ResponseMessages>
    </m:FindItemResponse>
  </s:Body>
</s:Envelope>"#;

        let result = parse_find_items_response(xml).expect("parse should succeed");
        assert_eq!(result.total_count, 150);
        assert!(!result.includes_last);
        assert_eq!(result.items.len(), 2);

        assert_eq!(result.items[0].item_id, "AAMkItem1=");
        assert_eq!(result.items[0].change_key.as_deref(), Some("CK1"));
        assert_eq!(result.items[0].subject.as_deref(), Some("Q1 Results"));
        assert_eq!(result.items[0].sender_email.as_deref(), Some("jane@contoso.com"));
        assert_eq!(result.items[0].sender_name.as_deref(), Some("Jane Doe"));
        assert!(result.items[0].is_read);

        assert_eq!(result.items[1].item_id, "AAMkItem2=");
        assert!(!result.items[1].is_read);
        assert_eq!(
            result.items[1].received_at.as_deref(),
            Some("2026-02-28T14:15:00Z")
        );
    }

    #[test]
    fn parse_find_items_empty() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                        xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="0" IncludesLastItemInRange="true">
            <t:Items/>
          </m:RootFolder>
        </m:FindItemResponseMessage>
      </m:ResponseMessages>
    </m:FindItemResponse>
  </s:Body>
</s:Envelope>"#;

        let result = parse_find_items_response(xml).expect("parse should succeed");
        assert_eq!(result.total_count, 0);
        assert!(result.includes_last);
        assert!(result.items.is_empty());
    }

    #[test]
    fn parse_get_item() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:GetItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                       xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:GetItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:Items>
            <t:Message>
              <t:ItemId Id="AAMkFull=" ChangeKey="CKFull"/>
              <t:Subject>Quarterly Review</t:Subject>
              <t:DateTimeReceived>2026-03-10T09:00:00Z</t:DateTimeReceived>
              <t:Body BodyType="HTML">&lt;html&gt;&lt;body&gt;Meeting notes here&lt;/body&gt;&lt;/html&gt;</t:Body>
              <t:IsRead>true</t:IsRead>
              <t:ItemClass>IPM.Note</t:ItemClass>
              <t:From>
                <t:Mailbox>
                  <t:Name>Alice Smith</t:Name>
                  <t:EmailAddress>alice@contoso.com</t:EmailAddress>
                </t:Mailbox>
              </t:From>
              <t:ToRecipients>
                <t:Mailbox>
                  <t:Name>Bob Jones</t:Name>
                  <t:EmailAddress>bob@contoso.com</t:EmailAddress>
                </t:Mailbox>
                <t:Mailbox>
                  <t:Name>Carol White</t:Name>
                  <t:EmailAddress>carol@contoso.com</t:EmailAddress>
                </t:Mailbox>
              </t:ToRecipients>
              <t:CcRecipients>
                <t:Mailbox>
                  <t:Name>Dave Brown</t:Name>
                  <t:EmailAddress>dave@contoso.com</t:EmailAddress>
                </t:Mailbox>
              </t:CcRecipients>
            </t:Message>
          </m:Items>
        </m:GetItemResponseMessage>
      </m:ResponseMessages>
    </m:GetItemResponse>
  </s:Body>
</s:Envelope>"#;

        let item = parse_get_item_response(xml).expect("parse should succeed");
        assert_eq!(item.item_id, "AAMkFull=");
        assert_eq!(item.change_key.as_deref(), Some("CKFull"));
        assert_eq!(item.subject.as_deref(), Some("Quarterly Review"));
        assert_eq!(item.sender_email.as_deref(), Some("alice@contoso.com"));
        assert_eq!(item.sender_name.as_deref(), Some("Alice Smith"));
        assert!(item.is_read);
        assert_eq!(item.item_class, "IPM.Note");
        assert!(item.body_html.is_some());
        assert!(item.body_html.as_deref().unwrap_or("").contains("<html>"));

        assert_eq!(item.to_recipients.len(), 2);
        assert_eq!(item.to_recipients[0].email, "bob@contoso.com");
        assert_eq!(item.to_recipients[0].name.as_deref(), Some("Bob Jones"));
        assert_eq!(item.to_recipients[1].email, "carol@contoso.com");

        assert_eq!(item.cc_recipients.len(), 1);
        assert_eq!(item.cc_recipients[0].email, "dave@contoso.com");
    }

    #[test]
    fn parse_get_item_empty_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:GetItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">
      <m:ResponseMessages>
        <m:GetItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:Items/>
        </m:GetItemResponseMessage>
      </m:ResponseMessages>
    </m:GetItemResponse>
  </s:Body>
</s:Envelope>"#;

        let result = parse_get_item_response(xml);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No item found"));
    }

    #[test]
    fn parse_create_item() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:CreateItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:CreateItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:Items>
            <t:Message>
              <t:ItemId Id="AAMkNewItem=" ChangeKey="CKNew"/>
            </t:Message>
          </m:Items>
        </m:CreateItemResponseMessage>
      </m:ResponseMessages>
    </m:CreateItemResponse>
  </s:Body>
</s:Envelope>"#;

        let item_id = parse_create_item_response(xml).expect("parse should succeed");
        assert_eq!(item_id, "AAMkNewItem=");
    }

    #[test]
    fn parse_create_item_empty_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:CreateItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">
      <m:ResponseMessages>
        <m:CreateItemResponseMessage ResponseClass="Error">
          <m:ResponseCode>ErrorAccessDenied</m:ResponseCode>
          <m:MessageText>Access is denied.</m:MessageText>
        </m:CreateItemResponseMessage>
      </m:ResponseMessages>
    </m:CreateItemResponse>
  </s:Body>
</s:Envelope>"#;

        let result = parse_create_item_response(xml);
        assert!(result.is_err());
    }

    #[test]
    fn effective_rights_parsing() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                          xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="1" IncludesLastItemInRange="true">
            <t:Folders>
              <t:Folder>
                <t:FolderId Id="AARead="/>
                <t:DisplayName>ReadOnly Folder</t:DisplayName>
                <t:TotalCount>10</t:TotalCount>
                <t:ChildFolderCount>0</t:ChildFolderCount>
                <t:UnreadCount>0</t:UnreadCount>
                <t:EffectiveRights>
                  <t:CreateAssociated>false</t:CreateAssociated>
                  <t:CreateContents>false</t:CreateContents>
                  <t:CreateHierarchy>false</t:CreateHierarchy>
                  <t:Delete>false</t:Delete>
                  <t:Modify>false</t:Modify>
                  <t:Read>true</t:Read>
                </t:EffectiveRights>
              </t:Folder>
            </t:Folders>
          </m:RootFolder>
        </m:FindFolderResponseMessage>
      </m:ResponseMessages>
    </m:FindFolderResponse>
  </s:Body>
</s:Envelope>"#;

        let folders = parse_find_folder_response(xml).expect("parse should succeed");
        assert_eq!(folders.len(), 1);
        let rights = &folders[0].effective_rights;
        assert!(!rights.create_associated);
        assert!(!rights.create_contents);
        assert!(!rights.create_hierarchy);
        assert!(!rights.delete);
        assert!(!rights.modify);
        assert!(rights.read);
    }

    #[test]
    fn decode_replica_list_guids() {
        // Simulate PR_REPLICA_LIST: two null-terminated GUID strings
        let guid1 = "{1A2B3C4D-5E6F-7A8B-9C0D-1E2F3A4B5C6D}";
        let guid2 = "{AAAABBBB-CCCC-DDDD-EEEE-FFFF00001111}";
        let mut raw = Vec::new();
        raw.extend_from_slice(guid1.as_bytes());
        raw.push(0);
        raw.extend_from_slice(guid2.as_bytes());
        raw.push(0);

        let encoded = BASE64.encode(&raw);
        let guids = decode_replica_list(&encoded).expect("decode should succeed");
        assert_eq!(guids.len(), 2);
        assert_eq!(guids[0], guid1);
        assert_eq!(guids[1], guid2);
    }

    #[test]
    fn decode_replica_list_empty() {
        let encoded = BASE64.encode(b"");
        let guids = decode_replica_list(&encoded).expect("decode should succeed");
        assert!(guids.is_empty());
    }

    #[test]
    fn get_folder_with_replica_list() {
        // Build base64 of a single null-terminated GUID
        let guid = "{ABCD1234-EF56-7890-AB12-CDEF34567890}";
        let mut raw = Vec::new();
        raw.extend_from_slice(guid.as_bytes());
        raw.push(0);
        let b64 = BASE64.encode(&raw);

        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:GetFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
                         xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:GetFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:Folders>
            <t:Folder>
              <t:FolderId Id="AAMkPF=" ChangeKey="CK1"/>
              <t:DisplayName>Public Docs</t:DisplayName>
              <t:TotalCount>55</t:TotalCount>
              <t:ChildFolderCount>2</t:ChildFolderCount>
              <t:UnreadCount>3</t:UnreadCount>
              <t:FolderClass>IPF.Note</t:FolderClass>
              <t:EffectiveRights>
                <t:CreateAssociated>false</t:CreateAssociated>
                <t:CreateContents>true</t:CreateContents>
                <t:CreateHierarchy>false</t:CreateHierarchy>
                <t:Delete>false</t:Delete>
                <t:Modify>false</t:Modify>
                <t:Read>true</t:Read>
              </t:EffectiveRights>
              <t:ExtendedProperty>
                <t:ExtendedFieldURI PropertyTag="0x6698" PropertyType="Binary"/>
                <t:Value>{b64}</t:Value>
              </t:ExtendedProperty>
            </t:Folder>
          </m:Folders>
        </m:GetFolderResponseMessage>
      </m:ResponseMessages>
    </m:GetFolderResponse>
  </s:Body>
</s:Envelope>"#
        );

        let folder = parse_get_folder_response(&xml).expect("parse should succeed");
        assert_eq!(folder.folder_id, "AAMkPF=");
        assert_eq!(folder.display_name, "Public Docs");
        assert_eq!(folder.total_count, 55);
        assert!(folder.replica_list.is_some());

        // Verify the replica list decodes back to our GUID
        let replica_bytes = folder.replica_list.as_ref().expect("should have replica list");
        let b64_round = BASE64.encode(replica_bytes);
        let guids = decode_replica_list(&b64_round).expect("decode should succeed");
        assert_eq!(guids.len(), 1);
        assert_eq!(guids[0], guid);
    }

    #[test]
    fn xml_escape_special_chars() {
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
        assert_eq!(xml_escape(r#"say "hi""#), "say &quot;hi&quot;");
    }

    #[test]
    fn distinguished_folder_ids() {
        assert!(is_distinguished_folder_id("publicfoldersroot"));
        assert!(is_distinguished_folder_id("inbox"));
        assert!(!is_distinguished_folder_id("AAMkAGFk="));
        assert!(!is_distinguished_folder_id("some-custom-id"));
    }
}
