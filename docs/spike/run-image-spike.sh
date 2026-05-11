#!/usr/bin/env bash
# Phase 0 spike for v0.3: post audio + image to llama-server in one
# /v1/chat/completions request and capture the response. The result
# tells us whether the v0.3 image-attachment feature can use a single
# request (preferred) or must fall back to two sequential requests.
#
# Run after llama-server is up at http://127.0.0.1:5809 with Gemma 4
# E4B + audio mmproj.
set -euo pipefail

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WAV="$SPIKE_DIR/audio/sample.wav"
IMG="${MNEMONIC_SPIKE_IMAGE:-$SPIKE_DIR/images/test-stacktrace.png}"
ENDPOINT="${MNEMONIC_LLAMA_ENDPOINT:-http://127.0.0.1:5809}/v1/chat/completions"
OUT_DIR="$SPIKE_DIR/responses"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
TAG="${MNEMONIC_SPIKE_TAG:-image}"
REQUEST_FILE="$OUT_DIR/${TAG}-request-$TS.json"
RESPONSE_FILE="$OUT_DIR/${TAG}-response-$TS.json"

mkdir -p "$OUT_DIR"
[ -f "$WAV" ] || { echo "missing $WAV"; exit 1; }
[ -f "$IMG" ] || { echo "missing $IMG (run images/make-test-image.py first)"; exit 1; }

AUDIO_B64="$(base64 -i "$WAV" | tr -d '\n')"
IMG_B64="$(base64 -i "$IMG" | tr -d '\n')"
IMG_MIME="image/png"

SYSTEM_PROMPT='You are taking a short voice note FOR THE USER. The user is the author. Output a single JSON object. The user may attach an image alongside the audio.

Schema:
{
  "cleaned": "string. The user'\''s spoken note as a single paragraph of plain text. Preserve their wording. Remove fillers and false-starts. No headings, no narrator voice (no \"the speaker\", \"the user\", etc). If silent, empty string.",
  "image_note": null | { "kind": "text", "text": "verbatim text extracted from the image, exactly as it appears" } | { "kind": "caption", "caption": "one-line description (<=80 chars) of what the image shows, when no extractable text is present" }
}

Rules:
- Output JSON only, no preamble.
- If no image is attached, return image_note: null.
- If the image contains terminal output, code, error messages, or any clear text, return kind="text" with the verbatim text. Do not paraphrase.
- If the image is mostly visual (chart, mockup, photo, diagram), return kind="caption" with a short factual description.
- If the image is blank, noise, or empty, return image_note: null.
- Never combine text and caption. Pick one or null.'

USER_TEXT='Process this voice memo. An image is attached. Return only the JSON object.'

jq -n \
  --arg system "$SYSTEM_PROMPT" \
  --arg audio "$AUDIO_B64" \
  --arg img "$IMG_B64" \
  --arg mime "$IMG_MIME" \
  --arg user_text "$USER_TEXT" \
  '{
    model: "gemma-4-e4b-it",
    messages: [
      { role: "system", content: $system },
      { role: "user", content: [
          { type: "input_audio", input_audio: { data: $audio, format: "wav" } },
          { type: "image_url", image_url: { url: ("data:" + $mime + ";base64," + $img) } },
          { type: "text", text: $user_text }
      ]}
    ],
    temperature: 0.2,
    max_tokens: 2048,
    response_format: { type: "json_object" }
  }' > "$REQUEST_FILE"

REQ_KB=$(du -k "$REQUEST_FILE" | cut -f1)
echo "POST $ENDPOINT (request ${REQ_KB} KB, image $(basename "$IMG"))"
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
