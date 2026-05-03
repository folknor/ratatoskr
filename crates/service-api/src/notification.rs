use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CoalesceKey(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationClass {
    Coalesce { key: CoalesceKey },
    Drop,
    MustDeliver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Notification {}

impl Notification {
    pub fn class(&self) -> NotificationClass {
        match *self {}
    }

    pub fn method_name(&self) -> &'static str {
        match *self {}
    }
}
