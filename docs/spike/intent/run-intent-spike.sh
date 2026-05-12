#!/usr/bin/env bash
# Phase 0 spike for v0.5: post each transcript in transcripts.jsonl to
# llama-server with an intent-extraction system prompt. Capture each
# response, time the call, and score precision/recall against the
# hand labels. Result tells us whether a separate "second-call"
# intent router on Gemma 4 E4B is fast enough and accurate enough.
#
# Run after llama-server is up at http://127.0.0.1:5809 with the same
# Gemma 4 E4B build the app uses.
set -euo pipefail

SPIKE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURES="$SPIKE_DIR/transcripts.jsonl"
ENDPOINT="${MNEMONIC_LLAMA_ENDPOINT:-http://127.0.0.1:5809}/v1/chat/completions"
OUT_DIR="$SPIKE_DIR/responses"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="$OUT_DIR/$TS"
SUMMARY="$RUN_DIR/summary.tsv"

mkdir -p "$RUN_DIR"
[ -f "$FIXTURES" ] || { echo "missing $FIXTURES"; exit 1; }
command -v jq >/dev/null || { echo "jq is required"; exit 1; }

SYSTEM_PROMPT='You are an intent-detection router for a voice-notes app.

You receive ONE short note the user just dictated. Output a single JSON object — nothing else, no prose, no markdown fences.

The user has registered exactly three shortcuts. You may only emit these names:
- "create-reminder"  — for time-anchored or deadline-driven todos
- "schedule-event"   — for calendar events with a specific time/day
- "send-message"     — for direct requests to message a named person

Schema:
{ "tool": "run_shortcut", "shortcut": "<one of the three names above>", "input": "<a one-line, plain-language version of the task>" }
OR
{ "tool": "none" }

Rules:
- Output JSON only.
- If the note is hedged, hypothetical, retrospective, observational, or a thought-dump, return {"tool":"none"}. Phrases like "I was thinking", "maybe I should", "I wonder if", "if only I had", "note to self that I never" are NOT actions.
- If the note is a clear, present-tense request to record/schedule/message something, return run_shortcut.
- "input" must be a faithful one-line rendering of what the user asked for. Do not paraphrase the time or the person if they were named.
- Never invent a shortcut name. Never combine multiple intents — pick the strongest if there are several.

Examples:
User: "Remind me to call Sarah at 3 PM."
{ "tool": "run_shortcut", "shortcut": "create-reminder", "input": "Call Sarah at 3 PM" }

User: "I was thinking I should probably remind Sarah but she already knows."
{ "tool": "none" }

User: "Schedule a 1:1 with Priya for Thursday at 4."
{ "tool": "run_shortcut", "shortcut": "schedule-event", "input": "1:1 with Priya Thursday at 4 PM" }

User: "The merge bug finally reproduces."
{ "tool": "none" }'

# Header for the summary TSV.
printf "id\tlabel\texpected\tactual_tool\tactual_shortcut\telapsed_ms\thttp\tmatch\n" > "$SUMMARY"

OK_MATCH=0
OK_TOTAL=0
NONE_MATCH=0
NONE_TOTAL=0

while IFS= read -r line; do
  [ -z "$line" ] && continue
  id=$(jq -r '.id' <<<"$line")
  label=$(jq -r '.label' <<<"$line")
  expected=$(jq -r '.expected_shortcut // "null"' <<<"$line")
  text=$(jq -r '.text' <<<"$line")

  req_file="$RUN_DIR/${id}-request.json"
  resp_file="$RUN_DIR/${id}-response.json"

  jq -n \
    --arg system "$SYSTEM_PROMPT" \
    --arg user "$text" \
    '{
      model: "gemma-4-e4b-it",
      messages: [
        { role: "system", content: $system },
        { role: "user",   content: $user }
      ],
      temperature: 0.1,
      max_tokens: 512,
      response_format: { type: "json_object" },
      chat_template_kwargs: { enable_thinking: false }
    }' > "$req_file"

  START_NS=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time_ns()))')
  HTTP_CODE=$(curl -sS -o "$resp_file" -w "%{http_code}" \
    -H "Content-Type: application/json" \
    --data-binary @"$req_file" \
    "$ENDPOINT" || echo "000")
  END_NS=$(date +%s%N 2>/dev/null || python3 -c 'import time; print(int(time.time_ns()))')
  ELAPSED_MS=$(( (END_NS - START_NS) / 1000000 ))

  content=$(jq -r '.choices[0].message.content // ""' "$resp_file" 2>/dev/null || echo "")
  actual_tool=$(echo "$content" | jq -r '.tool // "parse_error"' 2>/dev/null || echo "parse_error")
  actual_shortcut=$(echo "$content" | jq -r '.shortcut // "null"' 2>/dev/null || echo "null")

  # Match logic: actionable rows must hit the expected shortcut exactly;
  # "none" rows must produce tool=="none".
  match="MISS"
  if [ "$label" = "actionable" ]; then
    OK_TOTAL=$((OK_TOTAL + 1))
    if [ "$actual_tool" = "run_shortcut" ] && [ "$actual_shortcut" = "$expected" ]; then
      match="HIT"
      OK_MATCH=$((OK_MATCH + 1))
    fi
  else
    NONE_TOTAL=$((NONE_TOTAL + 1))
    if [ "$actual_tool" = "none" ]; then
      match="HIT"
      NONE_MATCH=$((NONE_MATCH + 1))
    fi
  fi

  printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n" \
    "$id" "$label" "$expected" "$actual_tool" "$actual_shortcut" "$ELAPSED_MS" "$HTTP_CODE" "$match" \
    >> "$SUMMARY"
  printf "%-7s %-10s expected=%-20s actual=%-12s shortcut=%-20s %4dms http=%s [%s]\n" \
    "$id" "$label" "$expected" "$actual_tool" "$actual_shortcut" "$ELAPSED_MS" "$HTTP_CODE" "$match"
done < "$FIXTURES"

echo "---"
echo "actionable hits: $OK_MATCH / $OK_TOTAL"
echo "none-hits:       $NONE_MATCH / $NONE_TOTAL"
echo "summary:         $SUMMARY"
echo
echo "next: copy the verdict into PHASE-0-INTENT-FINDINGS.md and decide go/no-go on the v0.5 line of work."
