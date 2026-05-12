# Idea: Intent routing — voice notes that trigger macOS actions

## Problem Statement

**How might we** let Mnemonic trigger native macOS actions from voice notes — without polluting the source-of-truth Markdown, losing user trust to false-positive side effects, or opening up classic AppleScript injection bugs?

The unlock: every recording already passes through Gemma 4 E4B for transcription. Forking the post-transcription text through a second, purpose-built intent-extraction prompt lets the same audio do double duty — produce a clean note *and* fire a Shortcut — with no new hotkey, no new UI ceremony, no new model.

## Recommended Direction

- **Architecture:** two LLM calls (user choice — accepted with eyes open about the latency cost; see *Assumptions* and *Risks*).
- **Trust:** notification + 5-second Undo. Silent execution is a ghost-reminder factory; explicit confirmation kills the magic.
- **Tool surface:** generic Shortcuts router only. Mnemonic emits `{tool: "run_shortcut", shortcut: "<name>", input: "<text>"}`; the user wires Reminders/Calendar/Fantastical/Messages once in Shortcuts.app and Mnemonic never touches AppleScript or `osascript`.
- **Audit trail:** the bullet gets a `↳ Ran shortcut "<name>": …` continuation line under the prose, same indented-block style as screenshot OCR. Notes remain the source of truth — you can grep what fired and when.

### Pipeline shape (post-v0.4)

```
recording  →  inbox/<ts>/         (already shipped)
worker      →  structure_audio    (already shipped — Gemma call 1)
            →  extract_intent      ← NEW Gemma call 2 (text-only)
            →  append_entry       (writes bullet + ↳ continuation)
            →  if intent matched: shortcuts run <name> --input <text>
                                    + notification("Ran X. Undo?")
                                    + 5-second undo timer
            →  inbox::complete
```

### Critical code surfaces

- **NEW `core/src/intent.rs`** — `extract_intent(text, &cfg, &endpoint) -> IntentResult`. Owns the system prompt, few-shot examples, JSON schema (`{tool: "run_shortcut", shortcut, input} | {tool: "none"}`), and the second `/v1/chat/completions` call. Mirrors the shape of `core/src/llama.rs::structure_audio` but text-in / JSON-out, no audio, no image, no thinking mode.
- **NEW `core/src/schema.rs::Intent` enum** — same `#[serde(tag = "tool", rename_all = "snake_case")]` pattern already used by `ImageNote`. Variants: `RunShortcut { shortcut, input }`, `None`. Easy to extend.
- **`app/src/main.rs::process_job`** — after the existing `structure_audio` await, on `StructuringResult::Ok` only, fork to `extract_intent`. Skip on Silent / Malformed / Failed.
- **NEW `app/src/shortcuts.rs`** — wraps `Command::new("shortcuts").args(["run", &name]).stdin(...)`. Input passes via stdin, *never* string-interpolated into a shell-escaped command line. No injection vector.
- **`core/src/markdown.rs::compose_bullet`** — third optional continuation block after embed + image_note. Format: `  ↳ Ran shortcut "<name>": <one-line summary>`.
- **`core/src/config.rs`** — new `[intents]` section: `enabled` (default `false` — opt-in), `allowed_shortcuts: Vec<String>` (whitelist, model can only emit names in this list), `undo_window_ms` (default 5000).

## Key Assumptions to Validate

1. **Second-call latency.** Gemma 4 E4B text-only on a 100-token transcript + 200-token prompt completes in <3s on M-series. → *Test:* `time curl /v1/chat/completions` with sample transcripts and the intent prompt. If >5s, revisit either the model (E2B for the router) or fold back to single-call.
2. **JSON-mode reliability for "no intent".** The model emits `{"tool": "none"}` for thought-dump recordings ≥95% of the time without hallucinating shortcut names. → *Test:* 30 hand-labeled transcripts (15 actionable, 15 not). Precision + recall on intent detection.
3. **Whitelist enforcement.** Even if the model emits a shortcut name that doesn't exist, the executor refuses to fire. Defense-in-depth — never trust model output to decide what runs.
4. **`shortcuts run --input` ergonomics.** macOS Shortcuts CLI accepts stdin input reliably and the user can build a "create-reminder" Shortcut that consumes the input cleanly. → *Test:* trivial shortcut "echo-input" prints what it got.
5. **Undo via paired shortcut.** A user-defined `undo-<name>` Shortcut, invoked within the 5-second window, can rollback the primary action. Realistic only for some Apple apps (Reminders' most-recent-reminder deletion works; Messages' "unsend" is impossible after send). Needs honest UX copy: "Undo available if your shortcut supports it."
6. **Two-call serial queue feel.** 5 rapid recordings × (5s structuring + 2s intent + 1s shortcut) ≈ 40s of worker grind. Acceptable for the "fire and forget" workflow; intent doesn't block the daily-note bullet from appearing.

## MVP Scope

One milestone. One tool. Opt-in.

- `intent.rs` + new `Intent` schema variant + `shortcuts.rs` executor.
- Config: `intents.enabled = false` by default. User flips it on after defining a Shortcut and adding its name to `allowed_shortcuts`.
- One worked example shipped in README: "Create reminder" — show how to build a 3-step Shortcut that accepts text input and creates a Reminders item.
- Bullet continuation line under the prose, separated by blank line.
- Notification with Undo button (Tauri-notification plugin already in deps); Undo invokes `undo-<name>` shortcut if defined, else shows "Undo not configured for this shortcut."
- `mnemonic redo ID` does NOT re-fire intents (one-shot, otherwise testing/debugging becomes hazardous).

Out of scope for the MVP:

- Multi-intent recordings (`"remind me X AND schedule Y"`). Emit the first detected intent; revisit if user data shows multi-intent is common.
- Built-in tool catalogue (Reminders, Calendar) — Shortcuts is the only execution surface.
- Per-recording context (passing prior bullets into the intent prompt). Keep the prompt tight, retrieval is a v2 problem.
- AppleScript path — never touch it. Shortcuts replaces it.
- Cross-platform — Mnemonic is macOS-only by design.

## Not Doing (and Why)

- **Direct AppleScript / `osascript` integration.** The original proposal had a real f-string injection bug (interpolating `task` straight into `set newReminder to make new reminder with properties {name:"{task}"}`). With Shortcuts CLI + stdin, the user-controlled text is never executable. One layer of risk eliminated for free.
- **Silent fire-and-forget.** Speech is consequential when it triggers side effects. A 5s undo notification is the cheapest trust buffer that doesn't kill the magic.
- **Single-call schema extension.** Two-call was chosen for prompt-purity. Cost noted: ~2× the per-job latency. If the serial queue feels sluggish in real use, single-call is a non-invasive escape hatch — only `core/src/llama.rs` and the system prompt change.
- **Pending-actions tray queue.** Asks the user to confirm every intent. Defeats "just speak."
- **Auto-discovery of installed Shortcuts.** Allowlist is the contract. Adding a Shortcut to macOS doesn't grant Mnemonic permission to fire it. Explicit > implicit for things that touch the OS.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| Second LLM call doubles per-job time on a serial queue | Document the latency; queue tray indicator already shows depth; offer single-call as a v2 config flag if it becomes a real complaint |
| Model emits hallucinated shortcut names | Whitelist enforcement in `shortcuts.rs::run` — refuse unknown names with a logged error, no notification fired |
| Model fires intent on non-action text ("I should remind Sarah… nah") | 5s undo + clear notification UX + few-shot examples in the prompt that explicitly call out hedged/hypothetical phrasings as `{tool: "none"}` |
| Undo window expires before user notices | Undo isn't perfect — surfacing the `↳ Ran shortcut` line in the bullet means a misfire is still discoverable hours later by reading the daily note |
| User edits the bullet text in the daily note before reviewing the intent | Intent fires from the worker's structured output, not from the bullet. Bullet is a *record*, not the trigger source |

## Open Questions

1. **Discovery UX.** How does the user know which Shortcuts Mnemonic will route to? Tray menu listing of `allowed_shortcuts`? CLI `mnemonic intents ls`?
2. **Multi-intent handling.** If a recording contains two distinct asks, do we emit two intents, ask the model to pick the strongest, or process them sequentially?
3. **Failure mode for the second LLM call.** If `extract_intent` fails (timeout, malformed JSON), do we silently skip (note still gets written), or surface the failure somewhere?
4. **Re-running intents.** If `mnemonic redo ID` re-structures an old recording, the intent has presumably already fired. Hard block re-fire? Or a `--with-intents` opt-in flag for the rare case where the user wants to test?

## Next Step

Phase 0 spike at `docs/spike/intent/`: a hand-labeled transcript set + curl-driven runner against the running llama-server + a findings doc. If the spike says latency and accuracy are both fine, this becomes the v0.5 line of work.
