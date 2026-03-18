use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use quick_xml::Reader;
use quick_xml::events::Event;

use super::{EwsEffectiveRights, EwsFolder, EwsItem, EwsRecipient, FindItemsResult};
use super::xml_helpers::{extract_attribute, strip_ns};

// ── Response parsers ────────────────────────────────────────

pub(super) fn parse_find_folder_response(xml: &str) -> Result<Vec<EwsFolder>, String> {
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

pub(super) fn parse_get_folder_response(xml: &str) -> Result<EwsFolder, String> {
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

pub(super) fn parse_find_items_response(xml: &str) -> Result<FindItemsResult, String> {
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

pub(super) fn parse_get_item_response(xml: &str) -> Result<EwsItem, String> {
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

pub(super) fn parse_create_item_response(xml: &str) -> Result<String, String> {
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
