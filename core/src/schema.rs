use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredNote {
    pub title: String,
    pub cleaned: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteStatus {
    Ok,
    Silent,
    Malformed,
    Failed,
}
