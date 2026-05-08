use std::time::Duration;

use base64::Engine;

use crate::StructuredNote;

pub const SYSTEM_PROMPT: &str = include_str!("system_prompt.txt");

const RETRY_REMINDER: &str =
    "Your previous response was not valid JSON. Output ONLY a JSON object matching the schema in the system prompt. No prose, no markdown fences, no commentary.";

pub struct StructureRequest<'a> {
    pub endpoint: &'a str,
    pub model_name: &'a str,
    pub timeout: Duration,
    pub thinking: bool,
}

pub enum StructuringResult {
    Ok(StructuredNote),
    /// Model returned non-JSON twice. `raw` is the second attempt.
    Malformed { raw: String },
    /// HTTP-level failure (unreachable, timeout, non-2xx, parse error).
    Failed { error: String },
}

pub async fn structure_audio(
    wav_bytes: &[u8],
    request: &StructureRequest<'_>,
) -> StructuringResult {
    let raw1 = match call_llama(wav_bytes, request, None).await {
        Ok(r) => r,
        Err(e) => return StructuringResult::Failed { error: e },
    };
    if let Ok(note) = serde_json::from_str::<StructuredNote>(&raw1) {
        return StructuringResult::Ok(note);
    }

    let raw2 = match call_llama(wav_bytes, request, Some(RETRY_REMINDER)).await {
        Ok(r) => r,
        Err(_) => return StructuringResult::Malformed { raw: raw1 },
    };
    if let Ok(note) = serde_json::from_str::<StructuredNote>(&raw2) {
        return StructuringResult::Ok(note);
    }
    StructuringResult::Malformed { raw: raw2 }
}

async fn call_llama(
    wav_bytes: &[u8],
    request: &StructureRequest<'_>,
    extra_reminder: Option<&str>,
) -> Result<String, String> {
    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(wav_bytes);
    let mut user_text = String::from("Process this voice memo. Return only the JSON object.");
    if let Some(reminder) = extra_reminder {
        user_text.push_str("\n\n");
        user_text.push_str(reminder);
    }
    let body = serde_json::json!({
        "model": request.model_name,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": [
                { "type": "input_audio", "input_audio": { "data": audio_b64, "format": "wav" } },
                { "type": "text", "text": user_text }
            ]}
        ],
        "temperature": 0.2,
        "max_tokens": 2048,
        "response_format": { "type": "json_object" },
        "chat_template_kwargs": { "enable_thinking": request.thinking }
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

pub fn is_silent(note: &StructuredNote) -> bool {
    note.title.eq_ignore_ascii_case("untranscribable")
}

/// Ping the configured llama-server endpoint's /health. Returns Ok if it
/// responds 2xx within the timeout, Err with a short reason otherwise.
pub async fn health_check(endpoint: &str, timeout: Duration) -> Result<(), String> {
    let url = format!("{}/health", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("client build: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("send: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    Ok(())
}
