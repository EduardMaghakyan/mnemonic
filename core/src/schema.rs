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

/// What the intent-router LLM call returns for a transcribed note. Either a
/// concrete request to run a registered Shortcut (with the input the user
/// dictated) or an explicit "no intent here, nothing to do."
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Intent {
    RunShortcut { shortcut: String, input: String },
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_run_shortcut_roundtrip() {
        let raw = r#"{"tool":"run_shortcut","shortcut":"create-reminder","input":"Call Sarah at 3 PM"}"#;
        let parsed: Intent = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed,
            Intent::RunShortcut {
                shortcut: "create-reminder".into(),
                input: "Call Sarah at 3 PM".into(),
            }
        );
        let reserialized = serde_json::to_string(&parsed).unwrap();
        assert!(reserialized.contains("\"tool\":\"run_shortcut\""));
        assert!(reserialized.contains("\"shortcut\":\"create-reminder\""));
    }

    #[test]
    fn intent_none_roundtrip() {
        let raw = r#"{"tool":"none"}"#;
        let parsed: Intent = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed, Intent::None);
        let reserialized = serde_json::to_string(&parsed).unwrap();
        assert_eq!(reserialized, r#"{"tool":"none"}"#);
    }

    #[test]
    fn intent_unknown_tool_rejected() {
        let raw = r#"{"tool":"send_carrier_pigeon","shortcut":"x","input":"y"}"#;
        let result: Result<Intent, _> = serde_json::from_str(raw);
        assert!(result.is_err(), "expected error for unknown tool");
    }
}
