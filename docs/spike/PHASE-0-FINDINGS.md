# Phase 0 — Spike findings

**Date:** 2026-05-08
**Verdict:** PASS — proceed to Phase 1

## What was tested

Confirmed end-to-end that `llama-server` running Gemma 4 E4B with the audio-capable mmproj accepts a 16 kHz mono WAV via HTTP and returns JSON conforming to the schema in `docs/spec.md` §2.4.

Artifacts:
- WAV: `audio/sample.wav` (471 KB, 16 kHz mono 16-bit PCM, ~13 s of speech generated with macOS `say` + `afconvert`)
- System prompt: `system-prompt.txt`
- Runner: `run-spike.sh`
- Request/response: `responses/request-20260507T230721Z.json`, `responses/response-20260507T230721Z.json`

## Setup that worked

- Hardware: Apple M3 Pro, 18 GB unified memory, macOS 26.4
- `llama.cpp` from Homebrew, version **9050** (Homebrew's `7730` from January did **not** know the `gemma4` architecture — `brew upgrade llama.cpp` was required before anything worked)
- Model: `unsloth/gemma-4-E4B-it-GGUF`, quant `Q4_K_M` (4.6 GiB on disk)
- mmproj: `mmproj-BF16.gguf` (~1 GiB; auto-selected by `--mmproj-auto`). Reports `has vision encoder` and `has audio encoder`. Audio config: 16 kHz, FFT 512, hop 160 — matches the spec's capture format exactly.
- Server flags: `-hf unsloth/gemma-4-E4B-it-GGUF:Q4_K_M --port 5809 -ngl 99 -c 8192 --mmproj-auto`
- Chat template auto-loaded; thinking mode is **on by default** (`chat template, thinking = 1`).

## Request shape that worked

OpenAI-compatible `/v1/chat/completions` with an `input_audio` content part:

```json
{
  "model": "gemma-4-e4b-it",
  "messages": [
    { "role": "system", "content": "<schema + rules>" },
    { "role": "user", "content": [
      { "type": "input_audio", "input_audio": { "data": "<base64 WAV>", "format": "wav" } },
      { "type": "text", "text": "Process this voice memo. Return only the JSON object." }
    ]}
  ],
  "temperature": 0.2,
  "max_tokens": 2048,
  "response_format": { "type": "json_object" }
}
```

`response_format: json_object` works. The model's chain-of-thought lands in `choices[0].message.reasoning_content` and does **not** pollute the parsed JSON in `content`.

## Schema adherence

First-try output conformed to spec §2.4 with all required keys present and shaped correctly:

| Field | Output | Conforms |
|---|---|---|
| `title` | "Email Sarah about migration plan and CI build speed" | Yes — 8 words, present tense, no trailing punctuation |
| `tags` | `["email-sarah", "migration-plan", "ci-build", "lint-rules"]` | Yes — 1–5 entries, lowercase kebab-case, no `#` |
| `summary` | 2 neutral-tone sentences | Yes |
| `cleaned` | Preserved user wording, dropped "Also" | Yes |
| `actions` | 3 imperative-mood TODOs | Yes |
| `questions` | `["Why is the CI build taking so long lately?"]` | Yes |
| `entities.people` | `["Sarah"]` | Yes |
| `entities.projects` | `["migration plan"]` | Slightly loose — "migration plan" is a topic, not a named project. Acceptable. |
| `entities.places` | `[]` | Yes |

Transcription accuracy was strong: only minor differences from the input (correctly rendered "C I" as "CI", added an apostrophe in "actions' tab"). No hallucinations.

## Performance

| Phase | Tokens | Wall time |
|---|---|---|
| Prompt processing | 645 (incl. ~13 s audio) | 2.8 s @ 230 tok/s |
| Generation (thinking + JSON) | 1,427 | 43.0 s @ 33 tok/s |
| **Total** | **2,072** | **~46 s** |

Most of the 1,427 generation tokens were the `reasoning_content` (chain of thought, ~1,100 tokens). The actual JSON payload was only ~300 tokens.

## Open issues / follow-ups for Phase 1

1. **The spec's 120 s request timeout is tight with thinking on for longer audio.** A 13 s clip took 46 s; a 5 min clip with thinking will likely exceed 120 s. Either raise the timeout, scale it dynamically with audio length, or make thinking configurable per the spec's existing `[model] thinking` flag (it already is — but the default `true` will collide with the default 120 s timeout).

2. **Audio support in llama.cpp is marked experimental** in the server log: `audio input is in experimental stage and may have reduced quality`. Quality and request-shape compatibility may drift between llama.cpp versions. The README in §4 should warn users to pin a known-good `llama.cpp` version, and `mnemonic doctor` should report it.

3. **mmproj auto-selected BF16, not F16.** Both work; BF16 is ~1 GB and slightly more accurate. Worth pinning the mmproj choice explicitly in the run command we ship in the README so users get reproducible behavior.

4. **Minimum llama.cpp version is high.** 9050+ for the `gemma4` architecture. README + `mnemonic doctor` need a clear "your llama.cpp is too old" branch — this would have failed silently for the user otherwise.

5. **HTTP shape decision for v1:** the spec says "single request per recording carries the WAV audio plus the structuring system prompt" — the OpenAI `input_audio` content-part shape proven here is the v1 contract. Recording it in code as the canonical request builder.

## Decision

Architecture bet validated. Proceed to Phase 1: pick the language/framework for the macOS app + CLI, scaffold, and start building the capture pipeline against the request shape proved here.
