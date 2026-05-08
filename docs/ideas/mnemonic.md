# Mnemonic — Idea One-Pager

## Problem Statement
How might we let a keyboard-native developer capture a fleeting thought by voice and have it land on disk as a structured, searchable Markdown note — without giving up local-first guarantees and without building a UI to maintain?

## Recommended Direction
Build Mnemonic as a tray app + CLI pair where the tray app does one job (capture → model → Markdown) and the CLI is where everything else lives (search, list, redo, doctor). The capture surface stays invisible: a hotkey, a three-state icon, a notification on save. This is for someone who already lives in their terminal and editor.

The local-model integration is the central bet. Gemma 4 E4B with native audio is the right v1 choice because it fits a 16 GB Mac and avoids a two-step Whisper-then-LLM pipeline. But since model choice is treated as soft, the architecture should treat structuring as a pure function — `WAV → structured JSON` — with the HTTP shape as one implementation. The spec mostly does this already; the leaks worth fixing now are the hard-coded `<|think|>` token and the implicit mmproj assumption. Both belong in config, not in code.

The submission post is a launch artifact, not the goal. Optimize for "still using it on day 30," not "finished by day 17."

## Key Assumptions to Validate
- [ ] **Gemma 4 E4B handles 5-minute WAVs reliably on a 16 GB Mac.** Phase 0 spike: 30s / 2min / 5min samples — record latency, JSON validity, memory pressure.
- [ ] **One retry before `malformed` is enough.** Run 50 real recordings; if >5% land in `malformed`, the retry prompt or the schema is wrong — not the model.
- [ ] **Filesystem-walk search stays fast through ~500 notes.** Bench `mnemonic find` at 100 / 500 / 1000 mock notes; if 1000 is sluggish, `qmd` work moves up.
- [ ] **Audio is a sufficient safety net.** If llama-server is unreachable for a week, you have WAVs but no text. Decide before shipping: is a Whisper fallback in `mnemonic doctor` worth the complexity, or is `mnemonic redo` later good enough?
- [ ] **5 minutes is the right cap.** Self-observe for a week without the app: if 95% of your memos are under 90s, ship as-is; if you regularly hit the cap, raise it before v1.

## MVP Scope (Phase 0 + 1)
**In:** half-day spike proving llama-server round-trip → tray + hotkey + WAV capture → HTTP call → JSON parse → Markdown writer with full frontmatter and section ordering → slug logic per 2.7. End state: hotkey, talk, hotkey, correctly-named Markdown file appears.

**Deferred to v1 phases 2–4:** permissions surfacing, notification flows, config hot-reload, log rotation, the six CLI commands, signing + DMG.

## Not Doing (and Why)
- **No GUI for config or browsing** — target user edits TOML in `$EDITOR`. A settings UI is a maintenance tax for a feature that wouldn't get used.
- **No streaming / live transcription** — adds complexity, no clear win for short memos, doesn't match the model's batch-call API.
- **No user-facing model swap in v1** — but the seams stay clean: prompt template, mmproj id, and `<|think|>` token live in config so v2 swap is config, not refactor.
- **No in-app search index** — filesystem walk until it hurts. Frontmatter + Markdown is the canonical data model; a future `qmd` indexes over it without v1 inventing a competing format.
- **No wikilinks in note bodies** — plain readers and v1 CLI don't render them. v2 gates them behind `[output] wikilinks = true`.
- **No raw transcript persistence in v1** — leave schema open. Adding a `transcript` sidecar later is additive; committing now risks the wrong shape.
- **No Intel Mac, no Windows/Linux** — the model fits 16 GB Apple Silicon and the tray APIs are macOS-native. Cross-platform is a different product.

## Open Questions
- **Where does the prompt template live?** If the model is soft-swappable, the system prompt itself (not just `thinking = true`) should be config or a swappable file. A different model won't take the same prompt.
- **What is the "model boundary" contract, precisely?** A future Whisper + text-LLM pipeline breaks the single-HTTP-call shape. Either accept v2 will refactor, or define the boundary now as `fn structure(wav_path) -> StructuredNote` with HTTP as one impl.
- **Is `keep_raw = true` the right default?** ~2 MB/min × 5 notes/day ≈ 150 MB/year. Probably fine, but worth naming the trade — and `mnemonic redo` only works when this is true.
- **Daily-notes folder convention.** Spec defers wikilinks to v2, but a one-line `[paths] daily_subfolder` flag would preserve Obsidian-vault use today without forcing it.
