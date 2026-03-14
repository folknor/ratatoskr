use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use quick_xml::Reader;
use quick_xml::events::Event;

const EWS_URL: &str = "https://outlook.office365.com/EWS/Exchange.asmx";

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

// ── Client ──────────────────────────────────────────────────

pub struct EwsClient {
    http: reqwest::Client,
    ews_url: String,
}

impl Default for EwsClient {
    fn default() -> Self {
        Self {
            http: reqwest::Client::new(),
            ews_url: EWS_URL.to_string(),
        }
    }
}

impl EwsClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_url(url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            ews_url: url,
        }
    }

    /// Execute a raw EWS SOAP request. Wraps `body_xml` in the SOAP envelope,
    /// sends it, checks for SOAP faults, and returns the response body.
    pub async fn execute(
        &self,
        access_token: &str,
        body_xml: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<String, String> {
        let envelope = build_soap_envelope(body_xml);

        let mut req = self
            .http
            .post(&self.ews_url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("Authorization", format!("Bearer {access_token}"));

        if let Some(h) = headers {
            if let Some(ref anchor) = h.anchor_mailbox {
                req = req.header("X-AnchorMailbox", anchor);
            }
            if let Some(ref pf) = h.public_folder_mailbox {
                req = req.header("X-PublicFolderMailbox", pf);
            }
        }

        let resp = req
            .body(envelope)
            .send()
            .await
            .map_err(|e| format!("EWS request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("EWS returned {status}: {body}"));
        }

        let xml = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read EWS response: {e}"))?;

        check_soap_fault(&xml)?;
        Ok(xml)
    }

    // ── Operations ──────────────────────────────────────────

    /// Browse child folders under a parent folder.
    /// Use `"publicfoldersroot"` for the top-level public folder hierarchy.
    pub async fn find_folder(
        &self,
        access_token: &str,
        parent_folder_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<Vec<EwsFolder>, String> {
        let distinguished = is_distinguished_folder_id(parent_folder_id);
        let escaped_id = xml_escape(parent_folder_id);
        let parent_xml = if distinguished {
            format!(r#"<t:DistinguishedFolderId Id="{escaped_id}"/>"#)
        } else {
            format!(r#"<t:FolderId Id="{escaped_id}"/>"#)
        };

        let body_xml = format!(
            r#"<m:FindFolder Traversal="Shallow">
  <m:FolderShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="folder:EffectiveRights"/>
      <t:FieldURI FieldURI="folder:FolderClass"/>
    </t:AdditionalProperties>
  </m:FolderShape>
  <m:ParentFolderIds>
    {parent_xml}
  </m:ParentFolderIds>
</m:FindFolder>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_find_folder_response(&xml)
    }

    /// Get detailed info for a single folder, including `PR_REPLICA_LIST`.
    pub async fn get_folder(
        &self,
        access_token: &str,
        folder_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<EwsFolder, String> {
        let escaped_id = xml_escape(folder_id);
        let body_xml = format!(
            r#"<m:GetFolder>
  <m:FolderShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="folder:EffectiveRights"/>
      <t:FieldURI FieldURI="folder:FolderClass"/>
      <t:ExtendedFieldURI PropertyTag="0x6698" PropertyType="Binary"/>
    </t:AdditionalProperties>
  </m:FolderShape>
  <m:FolderIds>
    <t:FolderId Id="{escaped_id}"/>
  </m:FolderIds>
</m:GetFolder>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_get_folder_response(&xml)
    }

    /// Find items (messages) in a folder with paging and optional date filter.
    pub async fn find_items(
        &self,
        access_token: &str,
        folder_id: &str,
        since: Option<&str>,
        offset: u32,
        max_entries: u32,
        headers: Option<&EwsHeaders>,
    ) -> Result<FindItemsResult, String> {
        let restriction = match since {
            Some(dt) => {
                let escaped_dt = xml_escape(dt);
                format!(
                    r#"<m:Restriction>
    <t:IsGreaterThanOrEqualTo>
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
      <t:FieldURIOrConstant>
        <t:Constant Value="{escaped_dt}"/>
      </t:FieldURIOrConstant>
    </t:IsGreaterThanOrEqualTo>
  </m:Restriction>"#
                )
            }
            None => String::new(),
        };

        let escaped_folder_id = xml_escape(folder_id);
        let body_xml = format!(
            r#"<m:FindItem Traversal="Shallow">
  <m:ItemShape>
    <t:BaseShape>IdOnly</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="item:Subject"/>
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
      <t:FieldURI FieldURI="message:From"/>
      <t:FieldURI FieldURI="item:Preview"/>
      <t:FieldURI FieldURI="message:IsRead"/>
      <t:FieldURI FieldURI="item:ItemClass"/>
    </t:AdditionalProperties>
  </m:ItemShape>
  <m:IndexedPageItemView MaxEntriesReturned="{max_entries}" Offset="{offset}" BasePoint="Beginning"/>
  {restriction}
  <m:SortOrder>
    <t:FieldOrder Order="Descending">
      <t:FieldURI FieldURI="item:DateTimeReceived"/>
    </t:FieldOrder>
  </m:SortOrder>
  <m:ParentFolderIds>
    <t:FolderId Id="{escaped_folder_id}"/>
  </m:ParentFolderIds>
</m:FindItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_find_items_response(&xml)
    }

    /// Get full details of a single item (message).
    pub async fn get_item(
        &self,
        access_token: &str,
        item_id: &str,
        headers: Option<&EwsHeaders>,
    ) -> Result<EwsItem, String> {
        let escaped_id = xml_escape(item_id);
        let body_xml = format!(
            r#"<m:GetItem>
  <m:ItemShape>
    <t:BaseShape>Default</t:BaseShape>
    <t:AdditionalProperties>
      <t:FieldURI FieldURI="item:Body"/>
      <t:FieldURI FieldURI="item:Attachments"/>
      <t:FieldURI FieldURI="message:ToRecipients"/>
      <t:FieldURI FieldURI="message:CcRecipients"/>
    </t:AdditionalProperties>
    <t:BodyType>HTML</t:BodyType>
  </m:ItemShape>
  <m:ItemIds>
    <t:ItemId Id="{escaped_id}"/>
  </m:ItemIds>
</m:GetItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_get_item_response(&xml)
    }

    /// Create a new message item in a public folder.
    pub async fn create_item(
        &self,
        access_token: &str,
        folder_id: &str,
        subject: &str,
        body_html: &str,
        to_recipients: &[(&str, &str)],
        headers: Option<&EwsHeaders>,
    ) -> Result<String, String> {
        let mut recipients_xml = String::new();
        for (email, name) in to_recipients {
            let escaped_email = xml_escape(email);
            let escaped_name = xml_escape(name);
            recipients_xml.push_str(&format!(
                r#"<t:Mailbox>
          <t:EmailAddress>{escaped_email}</t:EmailAddress>
          <t:Name>{escaped_name}</t:Name>
        </t:Mailbox>"#
            ));
        }

        let escaped_subject = xml_escape(subject);
        let escaped_body = xml_escape(body_html);
        let escaped_folder_id = xml_escape(folder_id);

        let body_xml = format!(
            r#"<m:CreateItem MessageDisposition="SaveOnly">
  <m:SavedItemFolderId>
    <t:FolderId Id="{escaped_folder_id}"/>
  </m:SavedItemFolderId>
  <m:Items>
    <t:Message>
      <t:Subject>{escaped_subject}</t:Subject>
      <t:Body BodyType="HTML">{escaped_body}</t:Body>
      <t:ToRecipients>
        {recipients_xml}
      </t:ToRecipients>
    </t:Message>
  </m:Items>
</m:CreateItem>"#
        );

        let xml = self.execute(access_token, &body_xml, headers).await?;
        parse_create_item_response(&xml)
    }
}

// ── SOAP envelope ───────────────────────────────────────────

fn build_soap_envelope(body_xml: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types"
               xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages">
  <soap:Header>
    <t:RequestServerVersion Version="Exchange2016"/>
  </soap:Header>
  <soap:Body>
    {body_xml}
  </soap:Body>
</soap:Envelope>"#
    )
}

// ── XML helpers ─────────────────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Strip namespace prefixes from element names for easier matching.
/// e.g. "t:FolderId" -> "FolderId", "soap:Fault" -> "Fault"
fn strip_ns(name: &str) -> &str {
    match name.find(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Well-known distinguished folder IDs that EWS treats specially.
fn is_distinguished_folder_id(id: &str) -> bool {
    matches!(
        id,
        "publicfoldersroot"
            | "inbox"
            | "drafts"
            | "sentitems"
            | "deleteditems"
            | "junkemail"
            | "outbox"
            | "calendar"
            | "contacts"
            | "tasks"
            | "notes"
            | "root"
            | "msgfolderroot"
    )
}

// ── SOAP fault check ────────────────────────────────────────

fn check_soap_fault(xml: &str) -> Result<(), String> {
    let mut reader = Reader::from_str(xml);
    let mut in_fault = false;
    let mut in_faultstring = false;
    let mut fault_message = String::new();
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if local == "Fault" {
                    in_fault = true;
                }
                if in_fault && local == "faultstring" {
                    in_faultstring = true;
                }
                buf.clear();
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_faultstring && local == "faultstring" {
                    fault_message = buf.trim().to_string();
                    in_faultstring = false;
                }
                if local == "Fault" {
                    break;
                }
                buf.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if in_fault || !fault_message.is_empty() {
        let msg = if fault_message.is_empty() {
            "Unknown SOAP fault".to_string()
        } else {
            fault_message
        };
        return Err(format!("EWS SOAP Fault: {msg}"));
    }

    Ok(())
}

// ── Response parsers ────────────────────────────────────────

fn parse_find_folder_response(xml: &str) -> Result<Vec<EwsFolder>, String> {
    let mut reader = Reader::from_str(xml);
    let mut folders = Vec::new();

    let mut in_folder = false;
    let mut in_effective_rights = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    // Current folder being built
    let mut folder_id = String::new();
    let mut display_name = String::new();
    let mut folder_class: Option<String> = None;
    let mut total_count: u32 = 0;
    let mut unread_count: u32 = 0;
    let mut child_folder_count: u32 = 0;
    let mut rights = EwsEffectiveRights::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);

                if local == "Folder" || local == "ContactsFolder" || local == "CalendarFolder" || local == "TasksFolder" {
                    in_folder = true;
                    folder_id.clear();
                    display_name.clear();
                    folder_class = None;
                    total_count = 0;
                    unread_count = 0;
                    child_folder_count = 0;
                    rights = EwsEffectiveRights::default();
                }
                if in_folder && local == "EffectiveRights" {
                    in_effective_rights = true;
                }
                current_tag = local.to_string();
                buf.clear();

                // Extract FolderId from attribute
                if in_folder && local == "FolderId" {
                    folder_id = extract_attribute(e, "Id");
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_folder && local == "FolderId" {
                    folder_id = extract_attribute(e, "Id");
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                let trimmed = buf.trim();

                if in_effective_rights {
                    match current_tag.as_str() {
                        "CreateAssociated" => rights.create_associated = trimmed == "true",
                        "CreateContents" => rights.create_contents = trimmed == "true",
                        "CreateHierarchy" => rights.create_hierarchy = trimmed == "true",
                        "Delete" => rights.delete = trimmed == "true",
                        "Modify" => rights.modify = trimmed == "true",
                        "Read" => rights.read = trimmed == "true",
                        _ => {}
                    }
                    if local == "EffectiveRights" {
                        in_effective_rights = false;
                    }
                } else if in_folder {
                    match current_tag.as_str() {
                        "DisplayName" => display_name = trimmed.to_string(),
                        "FolderClass" => folder_class = Some(trimmed.to_string()),
                        "TotalCount" => total_count = trimmed.parse().unwrap_or(0),
                        "UnreadCount" => unread_count = trimmed.parse().unwrap_or(0),
                        "ChildFolderCount" => child_folder_count = trimmed.parse().unwrap_or(0),
                        _ => {}
                    }
                }

                if (local == "Folder" || local == "ContactsFolder" || local == "CalendarFolder" || local == "TasksFolder") && in_folder {
                    if !folder_id.is_empty() {
                        folders.push(EwsFolder {
                            folder_id: folder_id.clone(),
                            display_name: display_name.clone(),
                            folder_class: folder_class.clone(),
                            total_count,
                            unread_count,
                            child_folder_count,
                            effective_rights: rights.clone(),
                            replica_list: None,
                        });
                    }
                    in_folder = false;
                }

                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    Ok(folders)
}

fn parse_get_folder_response(xml: &str) -> Result<EwsFolder, String> {
    let mut reader = Reader::from_str(xml);

    let mut in_folder = false;
    let mut in_effective_rights = false;
    let mut in_extended_property = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    let mut folder_id = String::new();
    let mut display_name = String::new();
    let mut folder_class: Option<String> = None;
    let mut total_count: u32 = 0;
    let mut unread_count: u32 = 0;
    let mut child_folder_count: u32 = 0;
    let mut rights = EwsEffectiveRights::default();
    let mut replica_list: Option<Vec<u8>> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);

                if local == "Folder" || local == "ContactsFolder" || local == "CalendarFolder" || local == "TasksFolder" {
                    in_folder = true;
                }
                if in_folder && local == "EffectiveRights" {
                    in_effective_rights = true;
                }
                if in_folder && local == "ExtendedProperty" {
                    in_extended_property = true;
                }
                current_tag = local.to_string();
                buf.clear();

                if in_folder && local == "FolderId" {
                    folder_id = extract_attribute(e, "Id");
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_folder && local == "FolderId" {
                    folder_id = extract_attribute(e, "Id");
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                let trimmed = buf.trim();

                if in_effective_rights {
                    match current_tag.as_str() {
                        "CreateAssociated" => rights.create_associated = trimmed == "true",
                        "CreateContents" => rights.create_contents = trimmed == "true",
                        "CreateHierarchy" => rights.create_hierarchy = trimmed == "true",
                        "Delete" => rights.delete = trimmed == "true",
                        "Modify" => rights.modify = trimmed == "true",
                        "Read" => rights.read = trimmed == "true",
                        _ => {}
                    }
                    if local == "EffectiveRights" {
                        in_effective_rights = false;
                    }
                } else if in_extended_property {
                    if current_tag == "Value"
                        && !trimmed.is_empty()
                        && let Ok(bytes) = BASE64.decode(trimmed)
                    {
                        replica_list = Some(bytes);
                    }
                    if local == "ExtendedProperty" {
                        in_extended_property = false;
                    }
                } else if in_folder {
                    match current_tag.as_str() {
                        "DisplayName" => display_name = trimmed.to_string(),
                        "FolderClass" => folder_class = Some(trimmed.to_string()),
                        "TotalCount" => total_count = trimmed.parse().unwrap_or(0),
                        "UnreadCount" => unread_count = trimmed.parse().unwrap_or(0),
                        "ChildFolderCount" => child_folder_count = trimmed.parse().unwrap_or(0),
                        _ => {}
                    }
                }

                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if folder_id.is_empty() {
        return Err("No folder found in GetFolder response".to_string());
    }

    Ok(EwsFolder {
        folder_id,
        display_name,
        folder_class,
        total_count,
        unread_count,
        child_folder_count,
        effective_rights: rights,
        replica_list,
    })
}

fn parse_find_items_response(xml: &str) -> Result<FindItemsResult, String> {
    let mut reader = Reader::from_str(xml);
    let mut items = Vec::new();

    let mut total_count: u32 = 0;
    let mut includes_last = false;

    let mut in_message = false;
    let mut in_from = false;
    let mut in_mailbox = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    // Current item
    let mut item_id = String::new();
    let mut change_key: Option<String> = None;
    let mut subject: Option<String> = None;
    let mut sender_email: Option<String> = None;
    let mut sender_name: Option<String> = None;
    let mut received_at: Option<String> = None;
    let mut body_preview: Option<String> = None;
    let mut is_read = false;
    let mut item_class = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);

                if local == "Message" {
                    in_message = true;
                    item_id.clear();
                    change_key = None;
                    subject = None;
                    sender_email = None;
                    sender_name = None;
                    received_at = None;
                    body_preview = None;
                    is_read = false;
                    item_class.clear();
                }
                if in_message && local == "From" {
                    in_from = true;
                }
                if in_from && local == "Mailbox" {
                    in_mailbox = true;
                }
                if local == "RootFolder" {
                    total_count = extract_attribute(e, "TotalItemsInView")
                        .parse()
                        .unwrap_or(0);
                    includes_last = extract_attribute(e, "IncludesLastItemInRange") == "true";
                }

                current_tag = local.to_string();
                buf.clear();

                if in_message && local == "ItemId" {
                    item_id = extract_attribute(e, "Id");
                    change_key = Some(extract_attribute(e, "ChangeKey"))
                        .filter(|s| !s.is_empty());
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_message && local == "ItemId" {
                    item_id = extract_attribute(e, "Id");
                    change_key = Some(extract_attribute(e, "ChangeKey"))
                        .filter(|s| !s.is_empty());
                }
                if local == "RootFolder" {
                    total_count = extract_attribute(e, "TotalItemsInView")
                        .parse()
                        .unwrap_or(0);
                    includes_last = extract_attribute(e, "IncludesLastItemInRange") == "true";
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                let trimmed = buf.trim();

                if in_mailbox && in_from {
                    match current_tag.as_str() {
                        "EmailAddress" => sender_email = Some(trimmed.to_string()),
                        "Name" => sender_name = Some(trimmed.to_string()),
                        _ => {}
                    }
                    if local == "Mailbox" {
                        in_mailbox = false;
                    }
                } else if in_message {
                    match current_tag.as_str() {
                        "Subject" => subject = Some(trimmed.to_string()),
                        "DateTimeReceived" => received_at = Some(trimmed.to_string()),
                        "Preview" => body_preview = Some(trimmed.to_string()),
                        "IsRead" => is_read = trimmed == "true",
                        "ItemClass" => item_class = trimmed.to_string(),
                        _ => {}
                    }
                }

                if local == "From" {
                    in_from = false;
                }
                if local == "Message" && in_message {
                    if !item_id.is_empty() {
                        items.push(EwsItem {
                            item_id: item_id.clone(),
                            change_key: change_key.clone(),
                            subject: subject.clone(),
                            sender_email: sender_email.clone(),
                            sender_name: sender_name.clone(),
                            received_at: received_at.clone(),
                            body_preview: body_preview.clone(),
                            body_html: None,
                            is_read,
                            item_class: if item_class.is_empty() {
                                "IPM.Note".to_string()
                            } else {
                                item_class.clone()
                            },
                            to_recipients: Vec::new(),
                            cc_recipients: Vec::new(),
                        });
                    }
                    in_message = false;
                }

                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    Ok(FindItemsResult {
        items,
        total_count,
        includes_last,
    })
}

fn parse_get_item_response(xml: &str) -> Result<EwsItem, String> {
    let mut reader = Reader::from_str(xml);

    let mut in_message = false;
    let mut in_from = false;
    let mut in_to = false;
    let mut in_cc = false;
    let mut in_mailbox = false;
    let mut current_tag = String::new();
    let mut buf = String::new();

    let mut item_id = String::new();
    let mut change_key: Option<String> = None;
    let mut subject: Option<String> = None;
    let mut sender_email: Option<String> = None;
    let mut sender_name: Option<String> = None;
    let mut received_at: Option<String> = None;
    let mut body_html: Option<String> = None;
    let mut is_read = false;
    let mut item_class = String::new();
    let mut to_recipients: Vec<EwsRecipient> = Vec::new();
    let mut cc_recipients: Vec<EwsRecipient> = Vec::new();

    // Current recipient being parsed
    let mut recip_email = String::new();
    let mut recip_name: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);

                if local == "Message" {
                    in_message = true;
                }
                if in_message {
                    match local {
                        "From" => in_from = true,
                        "ToRecipients" => in_to = true,
                        "CcRecipients" => in_cc = true,
                        _ => {}
                    }
                }
                if (in_from || in_to || in_cc) && local == "Mailbox" {
                    in_mailbox = true;
                    recip_email.clear();
                    recip_name = None;
                }

                current_tag = local.to_string();
                buf.clear();

                if in_message && local == "ItemId" {
                    item_id = extract_attribute(e, "Id");
                    change_key = Some(extract_attribute(e, "ChangeKey"))
                        .filter(|s| !s.is_empty());
                }
                if in_message && local == "Body" {
                    // Body element — content will come in Text event
                }
            }
            Ok(Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if in_message && local == "ItemId" {
                    item_id = extract_attribute(e, "Id");
                    change_key = Some(extract_attribute(e, "ChangeKey"))
                        .filter(|s| !s.is_empty());
                }
            }
            Ok(Event::Text(ref e)) => {
                if let Ok(text) = e.unescape() {
                    buf.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                let trimmed = buf.trim();

                if in_mailbox {
                    match current_tag.as_str() {
                        "EmailAddress" => recip_email = trimmed.to_string(),
                        "Name" => recip_name = Some(trimmed.to_string()),
                        _ => {}
                    }
                    if local == "Mailbox" {
                        in_mailbox = false;
                        if !recip_email.is_empty() {
                            let recipient = EwsRecipient {
                                email: recip_email.clone(),
                                name: recip_name.clone(),
                            };
                            if in_from {
                                sender_email = Some(recip_email.clone());
                                sender_name = recip_name.clone();
                            } else if in_to {
                                to_recipients.push(recipient);
                            } else if in_cc {
                                cc_recipients.push(recipient);
                            }
                        }
                    }
                } else if in_message {
                    match current_tag.as_str() {
                        "Subject" => subject = Some(trimmed.to_string()),
                        "DateTimeReceived" => received_at = Some(trimmed.to_string()),
                        "Body" => body_html = Some(trimmed.to_string()),
                        "IsRead" => is_read = trimmed == "true",
                        "ItemClass" => item_class = trimmed.to_string(),
                        _ => {}
                    }
                }

                match local {
                    "From" => in_from = false,
                    "ToRecipients" => in_to = false,
                    "CcRecipients" => in_cc = false,
                    _ => {}
                }

                buf.clear();
                current_tag.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    if item_id.is_empty() {
        return Err("No item found in GetItem response".to_string());
    }

    Ok(EwsItem {
        item_id,
        change_key,
        subject,
        sender_email,
        sender_name,
        received_at,
        body_preview: None,
        body_html,
        is_read,
        item_class: if item_class.is_empty() {
            "IPM.Note".to_string()
        } else {
            item_class
        },
        to_recipients,
        cc_recipients,
    })
}

fn parse_create_item_response(xml: &str) -> Result<String, String> {
    let mut reader = Reader::from_str(xml);

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                let local = strip_ns(&name);
                if local == "ItemId" {
                    let id = extract_attribute(e, "Id");
                    if !id.is_empty() {
                        return Ok(id);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    Err("No ItemId found in CreateItem response".to_string())
}

// ── Attribute extraction ────────────────────────────────────

fn extract_attribute(e: &quick_xml::events::BytesStart, attr_name: &str) -> String {
    for attr in e.attributes().flatten() {
        if String::from_utf8_lossy(attr.key.as_ref()) == attr_name {
            return String::from_utf8_lossy(&attr.value).to_string();
        }
    }
    String::new()
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
