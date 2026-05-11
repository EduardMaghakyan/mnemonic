use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredNote {
    pub cleaned: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_note: Option<ImageNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ImageNote {
    Text { text: String },
    Caption { caption: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteStatus {
    Ok,
    Silent,
    Malformed,
    Failed,
}
