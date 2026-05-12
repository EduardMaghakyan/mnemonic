# Mnemonic — Build Specification

A macOS menu-bar app that records short voice notes, transcribes and structures them with Gemma 4 E4B (native audio via llama-server), and writes structured Markdown files to disk. A companion CLI named `mnemonic` retrieves notes from the same store.

The product is fully local. No network calls except the loopback HTTP request to the user's own llama-server. No telemetry. No accounts.

---

## 1. Non-goals

The following are explicitly out of scope and must not be built:

- A GUI for editing notes, configuring the app, or browsing history.
- Cloud sync, multi-device, or any background networking.
- Live streaming transcription. Capture is start–stop, processed after the user ends the recording.
- Voice activity detection, speaker diarization, or noise suppression.
- Any model other than Gemma 4 E4B in v1. The model is not user-swappable in v1.
- Windows or Linux support in v1.

---

## 2. Hard requirements (v1)

### 2.1 Capture

- Global hotkey toggles recording. Default `Control+Option+Space`. User-configurable via the config file.
- Tray icon must visibly reflect three states: idle, recording, processing. State transitions must be perceptible within 100 ms of the triggering action.
- Recording produces 16 kHz, mono, 16-bit PCM WAV. No other formats. No resampling at write time — capture at this rate.
- Hard cap of 5 minutes per recording. At 5:00 the recording auto-stops and processing begins. Cap is configurable.
- Hotkey debounce: ignore presses within 250 ms of the previous press.
- If the hotkey is pressed during processing, ignore it and emit a soft system sound. Do not queue.
- A short system sound must play on recording start and stop. Both sounds must be distinct from each other and from macOS error sounds.

### 2.2 Permissions

- The app must declare `NSMicrophoneUsageDescription` with a user-visible string explaining audio is processed locally.
- The global hotkey is registered via Carbon `RegisterEventHotKey` (through `tauri-plugin-global-shortcut`). This API does **not** require Accessibility permission for normal use, so the app does not prompt for it. `mnemonic doctor` reports Accessibility status as informational only — it matters only in edge cases (e.g., apps using Secure Input mode capture the hotkey before it reaches us).
- If microphone permission is denied, pressing the hotkey must produce a notification explaining how to grant it. It must not silently fail. The tray menu surfaces "Grant Microphone Access…" which opens System Settings to the correct pane.

### 2.3 Model integration

- The app communicates with a user-run `llama-server` over HTTP at a configurable endpoint (default `http://127.0.0.1:5809`).
- The app does not start, stop, manage, or download the model. It assumes llama-server is running with `gemma-4-E4B-it` and the audio-capable mmproj loaded.
- A single request per recording carries the WAV audio plus the structuring system prompt. No multi-turn conversation.
- The app must use Gemma 4's thinking mode (`<|think|>` token in the system prompt) by default. Configurable.
- Request timeout: 120 seconds. On timeout, treat as a model failure (see 2.6).

### 2.4 Structured output contract

The model must return a JSON object matching this schema. Any deviation is a structuring failure:

```json
{
  "title":     "string",      // 3–8 words, present tense, no trailing punctuation
  "tags":      ["string"],    // 1–5 entries, lowercase, kebab-case, no leading '#'
  "summary":   "string",      // 1–2 sentences, neutral tone
  "cleaned":   "string",      // markdown body: user's wording preserved, fillers removed
  "actions":   ["string"],    // imperative-mood TODOs implied by the user; [] if none
  "questions": ["string"],    // open questions raised; [] if none
  "entities":  {
    "people":   ["string"],
    "projects": ["string"],
    "places":   ["string"]
  }
}
```

Rules baked into the system prompt:

- Output JSON only. No markdown fence. No preamble.
- If audio is silent or unintelligible, return the schema with `title: "untranscribable"` and all other string fields empty / arrays empty.
- The model must never invent details not present in the audio.

If the model returns invalid JSON: retry once with a stricter reminder prompt appended. If the second attempt also fails, persist the raw model output verbatim and mark the note `status: malformed`.

### 2.5 Output file

Every recording produces exactly one Markdown file at `~/Mnemonic/notes/YYYY-MM-DD/HHMMSS-{slug}.md`.

Frontmatter is YAML and must contain at minimum:

- `id`: ISO 8601 timestamp + slug, used as the canonical reference
- `created`: ISO 8601 with timezone offset
- `duration_sec`: integer
- `audio`: relative path to the saved WAV, or `null` if `keep_raw = false`
- `tags`, `people`, `projects`, `places`: arrays from the model's output
- `model`: model name string (e.g. `gemma-4-e4b-it`)
- `mmproj`: mmproj file identifier
- `status`: one of `ok`, `silent`, `malformed`, `failed`

Body sections, rendered in this order, omitted if empty:

1. `# {title}` — H1
2. `> {summary}` — blockquote
3. `## Note` — followed by `cleaned`
4. `## Actions` — unchecked task list `- [ ] ...`
5. `## Questions` — bullet list

### 2.6 Failure handling

| Failure | Behavior |
|---|---|
| llama-server unreachable | Save audio. Save stub markdown with `status: failed` and error in frontmatter. Tray shows red dot. Notification offers `mnemonic doctor`. |
| Model returns malformed JSON twice | Save audio. Save markdown with `status: malformed` containing raw model output verbatim under `## Raw Output`. |
| Audio is silent (model returns the silent schema) | Skip the write entirely. Audio is discarded too. Notification: "Silent recording — nothing saved." |
| Microphone permission denied | Hotkey is a no-op. Notification on press explains how to grant. |
| Disk full at write time | Native error notification. Audio buffer kept in memory and offered for retry on next launch. |
| Recording exceeds 5 min cap | Auto-stop, process normally. No data loss. |

No failure mode may discard audio. Audio retention is the safety net.

### 2.7 Slug rules

Title → slug:

1. Unicode NFKD normalize, drop combining marks (ASCII fold)
2. Lowercase
3. Replace any run of non-`[a-z0-9]` with a single `-`
4. Trim leading and trailing `-`
5. Truncate to 60 characters at the nearest preceding `-` (word boundary)
6. If empty after these steps, fall back to `note-YYYYMMDD-HHMMSS`

### 2.8 Configuration

Single TOML file at `~/.config/mnemonic/config.toml`. Keys:

- `[hotkey] combo` — string, default `"ctrl+alt+space"`
- `[audio] max_seconds` — integer, default `300`
- `[audio] keep_raw` — bool, default `true`
- `[paths] notes_dir` — string, default `"~/Mnemonic/notes"`
- `[paths] audio_dir` — string, default `"~/Mnemonic/audio"`
- `[model] endpoint` — string, default `"http://127.0.0.1:5809"`
- `[model] name` — string, default `"gemma-4-e4b-it"`
- `[model] thinking` — bool, default `true`

There must be no GUI for editing this file. Tray menu must include "Open config…" which opens it in `$EDITOR` or the system default.

The app must hot-reload the config on file change. A change to `[hotkey] combo` re-registers the hotkey without restart.

### 2.9 Notifications

On successful save: native notification with title = note title, subtitle = "Saved to Mnemonic". Click action opens the markdown file via `open <path>`, deferring to whatever app owns `.md`.

On failure: native notification with the error class and a hint pointing to `mnemonic doctor`.

### 2.10 Logging

A rotating log file at `~/Library/Logs/Mnemonic/mnemonic_rCURRENT.log` (rotated files at `mnemonic_rNNNNN.log`). Default level INFO; configurable via `MNEMONIC_LOG=debug`. Rotation: 5 MB per file, keep 3 files. Tray menu: "Reveal log in Finder".

No log line may contain transcribed text or the model's structured output. Logs are for engineering, not content.

### 2.11 Image attachments (v0.3+)

Added in v0.3. Optional; never required to record.

- **Capture triggers.** Two paths, both producing the same `(audio, png)` pair downstream:
  - **A. Clipboard auto-attach.** When the voice hotkey (`hotkey.combo`) is pressed, the app reads `NSPasteboard` once via `arboard::Clipboard::get_image()`. If an image is present, it is PNG-encoded and attached to the recording. If not, the recording proceeds as audio-only.
  - **B. One-shot screenshot + voice.** A second hotkey (`hotkey.screenshot_combo`, default `ctrl+alt+cmd+space`, empty string disables) spawns `screencapture -i <tempfile>` — macOS's native region-select crosshair without `-c`, so the user's clipboard is not clobbered. The blocking child returns when the user finishes the drag (file written) or hits Escape (file absent). On success the captured bytes are passed directly into the same recording entry point, bypassing the clipboard read; on cancel a "Screenshot cancelled" notification fires and no recording starts. Requires the macOS Screen Recording entitlement, which the system prompts for on first invocation. This hotkey is always toggle-style — `hotkey.mode = hold` does not apply to it.
- **Hard limits.** After PNG encoding, the image must be ≤ 4 MB. Over the limit: log a warning, skip the image, continue with audio-only.
- **Storage.** PNG is saved next to the WAV at `audio_dir/YYYY-MM-DD/HHMMSS.png`. Same `keep_raw` gating: if `keep_raw = false`, no PNG is written (and no `![](...)` embed appears in the bullet).
- **Model request.** Audio and image are sent in the same `/v1/chat/completions` request as two `content` parts: `input_audio` for the WAV, `image_url` (data URL with base64 PNG) for the image. Spike verified Gemma 4 E4B handles both in one forward pass; no two-pass fallback is needed.
- **Schema extension.** `StructuredNote` gains an optional `image_note` field:
  ```json
  { "cleaned": "...", "image_note": null
                                  | { "kind": "text",    "text": "verbatim text from the image" }
                                  | { "kind": "caption", "caption": "single-line description" } }
  ```
  The model picks exactly one of `null` / `text` / `caption`. Never both. `text` is used when extractable text is present (terminal output, code, error messages, written lists). `caption` is used for visual-only images (chart, mockup, photo). `null` for blank/noise.
- **Rendering.** The bullet keeps its single-line shape (prose + `[audio]` link). When an image is kept, the `![](rel.png)` embed is appended as its own 2-space indented block separated by a blank line; when `image_note` is present, another blank line and the continuation block follows (fenced code block for `text`, italicised one-liner for `caption`). The 2-space indent keeps every block inside the bullet's list item in CommonMark/Obsidian. Embed and `image_note` are independent: either, both, or neither may appear.
- **Privacy invariance.** Image bytes never leave loopback; they travel only to `127.0.0.1:5809`. The privacy guarantees in §4 hold unchanged.

### 2.12 Recording queue (v0.4+)

Added in v0.4. Decouples "user stopped speaking" from "model finished structuring."

- **Disk layout.** Each pending recording is a directory under `paths.inbox_dir` (default `~/Mnemonic/inbox`). The directory contains `manifest.json` (`{ "schema_version": 1, "recorded_at": "<RFC3339>" }`), `audio.wav`, and optionally `image.png`. The dir name begins with a UTC compact-RFC3339 timestamp so lexicographic sort = chronological order.
- **Atomic enqueue.** Files are staged under `inbox/.partial-<id>/` and the dir is renamed to its final name only after all writes flush. Scanner skips `.partial-*`.
- **Stop semantics.** On recording stop the audio capture drains and the WAV is encoded; the result is written to the inbox via the atomic enqueue, then the recorder state flips back to Idle. A second recording can start before the first has been structured.
- **Worker.** A single long-lived task pulls jobs oldest-first via `inbox::scan`, calls `structure_audio` once per job, and runs the same `append_entry` path the synchronous v0.3 flow used. After a successful append the inbox dir is removed.
- **Failure handling.** On `structure_audio` failure the existing `NoteContent::Failed` path writes a `_recording failed: …_` stub bullet; the WAV/PNG still land in `audio_dir/` per `keep_raw`. The inbox dir is then removed — no retries, no inbox accumulation. `mnemonic redo ID` is still the path for retrying a specific recording later.
- **Crash recovery.** On app startup the worker scans `inbox/` before accepting new signals. Anything left from a previous session (crash, kill, or `llama-server` outage during a session) is processed in chronological order without user intervention.
- **UI.** A non-clickable tray-menu line at the top shows queue depth: `Queue: idle` when empty, `Queue: N waiting` otherwise. The tray icon itself remains binary (gray = Idle, red = Recording); there is no longer a "Processing" color.

### 2.13 Intent routing (v0.5+)

Added in v0.5. Forks the worker pipeline so a successful transcription can trigger a macOS Shortcut without polluting the daily-note source-of-truth.

- **Config.** `[intents]` section with `enabled` (default `false` — opt-in), `allowed_shortcuts: Vec<String>` (whitelist — required), `undo_window_ms` (default `5000`).
- **Pipeline.** After `structure_audio` returns `Ok` (silence check has passed) and the config gates above are satisfied, the worker calls `extract_intent(cleaned, allowed_shortcuts, &req)`. The intent request always uses `chat_template_kwargs: { enable_thinking: false }` and `max_tokens: 512` per the Phase 0 spike findings (`docs/spike/intent/PHASE-0-INTENT-FINDINGS.md`).
- **Schema.** The model returns one of:
  ```json
  { "tool": "run_shortcut", "shortcut": "<name from allowlist>", "input": "<one-line task>" }
  { "tool": "none" }
  ```
- **Whitelist enforcement.** Defence in depth: even if the model emits a shortcut name outside `allowed_shortcuts`, the executor refuses to fire it. No process is spawned for unknown names.
- **Execution.** `app/src/shortcuts.rs::run_shortcut(name, input)` invokes `Command::new("shortcuts").args(["run", name])` with `input` piped via stdin. No string interpolation into the command line — eliminates AppleScript-style injection bugs by construction. Hard 5-second timeout on the child to avoid hung Shortcut prompts stalling the worker.
- **Rendering.** On successful fire, `append_entry` receives an `ExecutedIntent { shortcut, input }` which renders as a 2-space-indented `↳ Ran shortcut "<name>": <input>` continuation line under the bullet, after any image embed and image_note block. Notes remain the source of truth for what side effects fired.
- **Undo.** After a successful fire, the tray menu's *Undo* item is enabled with text `Undo: <shortcut>` for `undo_window_ms`. Click runs `undo-<shortcut>` via the same `run_shortcut` executor (no separate allowlist entry needed for the paired undo). After the window expires, the item disables and the action is committed.
- **Failure modes.** `IntentResult::NoIntent` / `Malformed` / `Failed` / executor errors all log a warning and proceed without a continuation line — the bullet still gets written. `mnemonic redo ID` does *not* re-fire intents (one-shot at first transcription).
- **Cold-start warmer.** At app startup, after `health_check`, a throwaway `extract_intent("ping", &allowlist, ...)` call primes llama-server so the first real intent doesn't pay the cold-prime tax (per spike, 6.6s cold vs 1.7s warm). No-op when intents are disabled.
- **Privacy invariance.** Intent calls hit `127.0.0.1:5809` only, same loopback as the structuring call. §4 guarantees hold.

---

## 3. CLI requirements (v1)

The `mnemonic` binary is a separate executable. It does not depend on the menu-bar app's UI or audio capture libraries. It reads the same config file and walks the same notes directory.

Required commands:

- **`mnemonic ls [--since DURATION] [--tag TAG] [--limit N]`** — list notes newest-first. Columns: time, title, tags. `DURATION` accepts `7d`, `24h`, `1w`, etc.
- **`mnemonic find QUERY [--open]`** — case-insensitive substring search across: frontmatter `title`, `tags`, `people`, `projects`, `places`; body `summary` and `cleaned`. Output: file path, matched line, ±1 line of context. `--open` opens the first result via `open`. Raw transcript is not searched in v1.
- **`mnemonic show ID`** — prints the file. Accepts full id or unambiguous prefix. Uses `bat` if on PATH, else `cat`.
- **`mnemonic todo [--since DURATION] [--tag TAG]`** — collect every unchecked `- [ ]` line from all notes, grouped by note title.
- **`mnemonic doctor`** — checklist with pass/fail for: config readable, notes/audio dirs writable, llama-server reachable, model loaded, mmproj loaded, microphone permission, accessibility permission. Each line has a remediation hint when failing.
- **`mnemonic redo ID`** — re-runs the structuring prompt against the saved audio for that note. Overwrites the markdown file in place. Fails cleanly if `keep_raw` was false when the original was made.

CLI must not invent its own search index in v1. It walks the filesystem and reads frontmatter. If this becomes too slow at scale, it is a v2 problem (see `qmd` in section 8).

---

## 4. Security and privacy guarantees

These are user-facing claims that must remain true:

- No network call ever leaves the loopback interface. The release build must compile with no telemetry crates.
- No analytics, no crash reporting service, no auto-updater that phones home in v1.
- Audio and notes live on disk in user-owned directories with default permissions. The app does not read other applications' data.
- The README and the in-app About screen state these guarantees verbatim.

---

## 5. Performance budgets

- Tray state transition latency: ≤ 100 ms from hotkey press to icon change.
- Audio capture must not drop frames. If the audio backend reports a buffer overrun, log it and continue.
- The app's idle CPU usage must be effectively zero (no polling loops). Hotkey is event-driven.
- App memory footprint when idle must be under 100 MB. Recording adds the audio buffer (≈ 10 MB max at the 5-min cap).
- The model call latency is whatever the model takes; not the app's budget. But the app must not block the tray UI thread during the call.

---

## 6. Distribution (v1)

- Single signed and notarized DMG for Apple Silicon. Intel Mac is not a v1 target.
- The DMG must install both the menu-bar app and the `mnemonic` CLI. The CLI is symlinked into `/usr/local/bin` by the installer or surfaced via a "Install CLI" tray menu item.
- Public GitHub release with the DMG attached and a SHA256.
- Homebrew tap is a v2 nicety, not a v1 requirement.

---

## 7. Build phases

### Phase 0 — Spike (timeboxed, half day)

Confirm end-to-end that llama-server with the Gemma 4 E4B audio mmproj accepts a WAV via HTTP and returns a usable response. No app code yet. A `curl` command and a saved response is enough. If this does not work in half a day, reassess scope before proceeding.

### Phase 1 — Capture pipeline (days 1–4)

- Tray icon with three states.
- Global hotkey registration with debounce.
- Audio capture writing 16k mono WAV to disk.
- Recording auto-stops at the configured cap.
- HTTP client posting WAV to llama-server with the structuring prompt.
- Response parsing into the schema.
- Markdown writer with full frontmatter and section ordering.
- Slug logic per 2.7.

Acceptance: pressing the hotkey, talking, pressing again, produces a correctly-named, correctly-structured markdown file with audio next to it. Failure modes from 2.6 are handled.

### Phase 2 — Correctness daemons (days 5–7)

- Permissions surfacing for mic and accessibility.
- All notification flows.
- Config hot-reload, including hotkey re-registration.
- Logging with rotation.
- The retry-once-then-malformed flow for invalid JSON.

Acceptance: deliberately breaking each failure mode in 2.6 produces the documented behavior. Editing the config without quitting the app changes behavior live.

### Phase 3 — CLI (days 8–10)

All six commands per section 3. `mnemonic doctor` is highest priority because it is what users will run when something is wrong.

Acceptance: each command runs against a notes directory containing at least 20 mixed `ok` / `silent` / `malformed` / `failed` notes and behaves per spec.

### Phase 4 — Distribution (days 11–13)

- Code signing and notarization.
- DMG packaging with both binaries.
- README with the privacy guarantees verbatim, demo GIF, install steps, model setup pointer, troubleshooting via `mnemonic doctor`.
- GitHub release.

### Phase 5 — Polish and submission (days 14–17)

- Buffer for the things that took longer than expected. They will.
- Submission write-up explaining model choice (E4B because it is the only Gemma 4 size with native audio that fits a 16 GB laptop) and what each capability unlocked.

---

## 8. v2 — out of scope for v1, design v1 to not preclude them

### 8.1 `qmd` — quick mnemonic discovery

A v2 CLI tool replacing the v1 grep-style `mnemonic find`. `qmd` is a faster, smarter search command. Concretely:

- Maintains an on-disk index over the notes directory (full-text plus frontmatter facets).
- Watches the notes directory and updates the index incrementally as new notes are written.
- Supports filters like `qmd "build spec" tag:mnemonic person:sarah since:7d`.
- Supports raw transcript search if v2 ships transcript persistence (see 8.3).
- Replaces `mnemonic find` outright; `mnemonic find` becomes an alias.

v1 must not invent a competing index format. The frontmatter and Markdown layout in 2.5 are the data model `qmd` will index over.

### 8.2 Obsidian integration

Two distinct features, do not conflate:

- **Vault target**: configurable `notes_dir` already accommodates pointing Mnemonic at an Obsidian vault. v2 work is wikilink rendering (`[[note-id]]` referencing related notes) and respecting the vault's daily-notes folder convention.
- **Plugin**: an Obsidian plugin that surfaces Mnemonic actions inside Obsidian (record from a command palette entry, see today's notes in a side panel). This requires a stable IPC contract between the app and the plugin. Mnemonic must expose a localhost JSON-RPC or Unix-socket endpoint in v2 — not in v1.

v1 must not introduce wikilinks in note bodies because plain Markdown readers and the v1 CLI do not handle them. v2 will gate wikilink emission behind a `[output] wikilinks = true` config flag.

### 8.3 Raw transcript persistence

v1 only persists the model's `cleaned` field, not the raw transcript before structuring. v2 may add a separate `transcript` field to frontmatter or a sidecar file, enabling `qmd` to search words the model dropped during cleaning. v1 must not commit to a schema choice for this — leave it open.

---

## 9. Definition of done for v1

All of the following must be true to ship:

- Every requirement in section 2 and section 3 passes manual verification.
- The DMG installs cleanly on a fresh macOS user account and the first recording succeeds without the user touching anything beyond granting permissions.
- `mnemonic doctor` returns all green on a correctly-set-up machine and produces actionable output for each documented failure mode.
- README contains the privacy guarantees verbatim, a 10-second demo GIF, install steps, and the model-choice rationale.
- No log line, no error message, and no notification ever contains transcribed text or model output.
- The submission post links to the GitHub release and explains why E4B was the right model for this job.