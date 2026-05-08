use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredNote {
    pub title: String,
    pub tags: Vec<String>,
    pub summary: String,
    pub cleaned: String,
    pub actions: Vec<String>,
    pub questions: Vec<String>,
    pub entities: Entities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entities {
    pub people: Vec<String>,
    pub projects: Vec<String>,
    pub places: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteStatus {
    Ok,
    Silent,
    Malformed,
    Failed,
}
