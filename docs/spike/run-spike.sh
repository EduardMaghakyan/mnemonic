#!/usr/bin/env bash
# Phase 0 spike: post a 16 kHz mono WAV to llama-server and capture the JSON response.
# Run after llama-server is up at http://127.0.0.1:5809 with Gemma 4 E4B + audio mmproj.
set -euo pipefail

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WAV="$SPIKE_DIR/audio/sample.wav"
SYSTEM_PROMPT_FILE="$SPIKE_DIR/system-prompt.txt"
ENDPOINT="${MNEMONIC_LLAMA_ENDPOINT:-http://127.0.0.1:5809}/v1/chat/completions"
OUT_DIR="$SPIKE_DIR/responses"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
REQUEST_FILE="$OUT_DIR/request-$TS.json"
RESPONSE_FILE="$OUT_DIR/response-$TS.json"

mkdir -p "$OUT_DIR"

[ -f "$WAV" ] || { echo "missing $WAV"; exit 1; }
[ -f "$SYSTEM_PROMPT_FILE" ] || { echo "missing $SYSTEM_PROMPT_FILE"; exit 1; }

AUDIO_B64="$(base64 -i "$WAV" | tr -d '\n')"
SYSTEM_PROMPT="$(cat "$SYSTEM_PROMPT_FILE")"

jq -n \
  --arg system "$SYSTEM_PROMPT" \
  --arg audio "$AUDIO_B64" \
  --arg user_text "Process this voice memo. Return only the JSON object." \
  '{
    model: "gemma-4-e4b-it",
    messages: [
      { role: "system", content: $system },
      { role: "user", content: [
          { type: "input_audio", input_audio: { data: $audio, format: "wav" } },
          { type: "text", text: $user_text }
      ]}
    ],
    temperature: 0.2,
    max_tokens: 2048,
    response_format: { type: "json_object" }
  }' > "$REQUEST_FILE"

echo "POST $ENDPOINT"
echo "request: $REQUEST_FILE ($(du -h "$REQUEST_FILE" | cut -f1))"
START="$(date +%s)"
HTTP_CODE="$(curl -sS -o "$RESPONSE_FILE" -w "%{http_code}" \
  -H "Content-Type: application/json" \
  --data-binary @"$REQUEST_FILE" \
  "$ENDPOINT")"
ELAPSED=$(( $(date +%s) - START ))
echo "http=$HTTP_CODE elapsed=${ELAPSED}s"
echo "response: $RESPONSE_FILE"
echo "---"
jq . "$RESPONSE_FILE" 2>/dev/null || cat "$RESPONSE_FILE"
