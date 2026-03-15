use serde::Serialize;

/// Common fields shared by all provider-specific parsed message types.
///
/// Each provider embeds this as `pub base: ParsedMessageBase` and adds
/// its own provider-specific fields alongside it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMessageBase {
    pub id: String,
    pub thread_id: String,
    pub from_address: Option<String>,
    pub from_name: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub reply_to: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub raw_size: i64,
    pub internal_date: i64,
    pub label_ids: Vec<String>,
    pub has_attachments: bool,
    pub message_id_header: Option<String>,
    pub references_header: Option<String>,
    pub in_reply_to_header: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub list_unsubscribe_post: Option<String>,
    pub auth_results: Option<String>,
    pub mdn_requested: bool,
}

impl crate::seen_addresses::MessageAddresses for ParsedMessageBase {
    fn sender_address(&self) -> Option<&str> {
        self.from_address.as_deref()
    }
    fn sender_name(&self) -> Option<&str> {
        self.from_name.as_deref()
    }
    fn to_addresses(&self) -> Option<&str> {
        self.to_addresses.as_deref()
    }
    fn cc_addresses(&self) -> Option<&str> {
        self.cc_addresses.as_deref()
    }
    fn bcc_addresses(&self) -> Option<&str> {
        self.bcc_addresses.as_deref()
    }
    fn msg_date_ms(&self) -> i64 {
        self.date
    }
}

/// Implement `MessageAddresses` for a type that has a `base: ParsedMessageBase` field.
macro_rules! impl_message_addresses {
    ($ty:ty) => {
        impl crate::seen_addresses::MessageAddresses for $ty {
            fn sender_address(&self) -> Option<&str> {
                self.base.sender_address()
            }
            fn sender_name(&self) -> Option<&str> {
                self.base.sender_name()
            }
            fn to_addresses(&self) -> Option<&str> {
                crate::seen_addresses::MessageAddresses::to_addresses(&self.base)
            }
            fn cc_addresses(&self) -> Option<&str> {
                crate::seen_addresses::MessageAddresses::cc_addresses(&self.base)
            }
            fn bcc_addresses(&self) -> Option<&str> {
                crate::seen_addresses::MessageAddresses::bcc_addresses(&self.base)
            }
            fn msg_date_ms(&self) -> i64 {
                self.base.msg_date_ms()
            }
        }
    };
}

pub(crate) use impl_message_addresses;
