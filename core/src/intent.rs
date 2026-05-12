use std::time::Duration;

use crate::Intent;

const INTENT_PROMPT_TEMPLATE: &str = include_str!("intent_prompt.txt");
const ALLOWED_PLACEHOLDER: &str = "{{ALLOWED_SHORTCUTS}}";

pub struct IntentRequest<'a> {
    pub endpoint: &'a str,
    pub model_name: &'a str,
    pub timeout: Duration,
}

pub enum IntentResult {
    /// Model emitted a concrete `run_shortcut` intent. The shortcut name has
    /// not been validated against the allowlist yet — caller must enforce.
    Matched(Intent),
    /// Model emitted `{"tool":"none"}` — no side effect to fire.
    NoIntent,
    /// Model returned content that didn't deserialize as an `Intent`.
    Malformed { raw: String },
    /// HTTP-level failure (unreachable, timeout, non-2xx, parse error).
    Failed { error: String },
}

/// Send the cleaned transcript to llama-server with a templated intent-
/// extraction prompt. Returns `Matched(RunShortcut)` for actionable notes,
/// `NoIntent` for thought-dumps. Always disables thinking-mode and uses
/// JSON-mode response formatting; both validated by the Phase 0 spike.
pub async fn extract_intent(
    cleaned: &str,
    allowed_shortcuts: &[String],
    request: &IntentRequest<'_>,
) -> IntentResult {
    if allowed_shortcuts.is_empty() {
        // Caller should have gated on this — defence in depth so we don't
        // ever call llama with an empty allowlist (model has nothing valid
        // to emit and would either produce "none" or hallucinate a name).
        return IntentResult::NoIntent;
    }
    let raw = match call_intent(cleaned, allowed_shortcuts, request).await {
        Ok(r) => r,
        Err(e) => return IntentResult::Failed { error: e },
    };
    match serde_json::from_str::<Intent>(&raw) {
        Ok(Intent::None) => IntentResult::NoIntent,
        Ok(intent @ Intent::RunShortcut { .. }) => IntentResult::Matched(intent),
        Err(_) => IntentResult::Malformed { raw },
    }
}

async fn call_intent(
    cleaned: &str,
    allowed_shortcuts: &[String],
    request: &IntentRequest<'_>,
) -> Result<String, String> {
    let allowlist_block = allowed_shortcuts
        .iter()
        .map(|s| format!("- \"{s}\""))
        .collect::<Vec<_>>()
        .join("\n");
    let system_prompt = INTENT_PROMPT_TEMPLATE.replace(ALLOWED_PLACEHOLDER, &allowlist_block);

    let body = serde_json::json!({
        "model": request.model_name,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user",   "content": cleaned }
        ],
        "temperature": 0.1,
        "max_tokens": 512,
        "response_format": { "type": "json_object" },
        "chat_template_kwargs": { "enable_thinking": false }
    });

    let client = reqwest::Client::builder()
        .timeout(request.timeout)
        .build()
        .map_err(|e| format!("client build: {e}"))?;

    let resp: serde_json::Value = client
        .post(request.endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send: {e}"))?
        .error_for_status()
        .map_err(|e| format!("status: {e}"))?
        .json()
        .await
        .map_err(|e| format!("parse: {e}"))?;

    resp["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing content in response: {resp}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_shortcut_response() {
        let raw = r#"{"tool":"run_shortcut","shortcut":"create-reminder","input":"Call Sarah at 3 PM"}"#;
        match serde_json::from_str::<Intent>(raw).unwrap() {
            Intent::RunShortcut { shortcut, input } => {
                assert_eq!(shortcut, "create-reminder");
                assert_eq!(input, "Call Sarah at 3 PM");
            }
            _ => panic!("expected RunShortcut"),
        }
    }

    #[test]
    fn parses_none_response() {
        let raw = r#"{"tool":"none"}"#;
        assert_eq!(serde_json::from_str::<Intent>(raw).unwrap(), Intent::None);
    }

    #[test]
    fn rejects_malformed_json() {
        let raw = r#"{"tool":"run_shortcut" no really"#;
        assert!(serde_json::from_str::<Intent>(raw).is_err());
    }

    #[test]
    fn system_prompt_template_substitutes_allowlist() {
        let allowlist = ["create-reminder".to_string(), "schedule-event".to_string()];
        let block = allowlist
            .iter()
            .map(|s| format!("- \"{s}\""))
            .collect::<Vec<_>>()
            .join("\n");
        let rendered = INTENT_PROMPT_TEMPLATE.replace(ALLOWED_PLACEHOLDER, &block);
        assert!(rendered.contains("- \"create-reminder\""));
        assert!(rendered.contains("- \"schedule-event\""));
        assert!(!rendered.contains(ALLOWED_PLACEHOLDER));
    }
}
